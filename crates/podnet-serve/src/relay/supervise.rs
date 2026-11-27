//! The supervisor of the SSH relay leg.
//!
//! [`super::open`] gives a session and does not watch it. The relay can end
//! the connection or the banner channel at any time (the free tier of the
//! default relay ends a tunnel after a fixed time). A new session has a new
//! remote port and a new URL. [`supervise`] opens a session, waits for its
//! end, and opens the next one after a backoff. It sends one [`Event`] for
//! each change of the URL.

use std::time::{Duration, Instant};

use russh::ChannelMsg;
use tokio::sync::mpsc::UnboundedSender;

use super::{open, Live, RelayConfig, RelayError, TunnelHandle};

/// A change of the state of the leg.
#[derive(Debug, Clone)]
pub enum Event {
    /// A session is up. The URL is the one to publish now.
    Up(TunnelHandle),
    /// The session ended, or an attempt to open one failed. The old URL is
    /// dead. The supervisor tries again after the backoff.
    Down {
        /// The reason, as one line.
        reason: String,
    },
}

/// The wait between two sessions.
#[derive(Debug, Clone, Copy)]
pub struct Backoff {
    /// The first wait.
    pub initial: Duration,
    /// The longest wait. Each failure doubles the wait up to this limit.
    pub max: Duration,
    /// A session that lasts at least this long resets the wait to
    /// `initial`. A shorter session counts as a failure.
    pub stable: Duration,
}

impl Default for Backoff {
    /// 1 s to 30 s, as the reverse relay (`podnet relay node`).
    fn default() -> Self {
        Self {
            initial: Duration::from_secs(1),
            max: Duration::from_secs(30),
            stable: Duration::from_secs(10),
        }
    }
}

impl Backoff {
    /// The wait after `delay`: twice `delay`, at most `max`.
    pub fn next(&self, delay: Duration) -> Duration {
        (delay * 2).min(self.max)
    }
}

/// Keep one session of `config` open, and send an [`Event`] at each change.
///
/// If the first session fails to start, return its [`RelayError`] with no
/// retry (T-624). Once a session is up, a later drop retries with `backoff`
/// until the receiver of `events` is dropped. Drop the future to stop: that
/// closes the live session.
pub async fn supervise(
    config: RelayConfig,
    backoff: Backoff,
    events: UnboundedSender<Event>,
) -> Result<(), RelayError> {
    let mut delay = backoff.initial;
    let mut started_once = false;
    loop {
        let reason = match open(&config).await {
            Ok(session) => {
                started_once = true;
                let started = Instant::now();
                if events.send(Event::Up(session.tunnel.clone())).is_err() {
                    return Ok(());
                }
                let reason = wait_end(session).await;
                if started.elapsed() >= backoff.stable {
                    delay = backoff.initial;
                }
                reason
            }
            Err(err) if !started_once => return Err(err),
            Err(err) => err.to_string(),
        };
        tracing::warn!(relay = %config.address, %reason, retry_in = ?delay, "the relay session is down");
        if events.send(Event::Down { reason }).is_err() {
            return Ok(());
        }
        tokio::time::sleep(delay).await;
        delay = backoff.next(delay);
    }
}

/// Wait until the relay ends `session`. Give the reason.
async fn wait_end(session: Live) -> String {
    let Live {
        handle, mut banner, ..
    } = session;
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            msg = banner.wait() => match msg {
                None => return "the relay closed the connection".to_string(),
                Some(ChannelMsg::Eof | ChannelMsg::Close) => {
                    return "the relay closed the banner channel".to_string();
                }
                Some(ChannelMsg::Data { data }) => {
                    tracing::debug!(text = %String::from_utf8_lossy(&data), "relay banner");
                }
                Some(_) => {}
            },
            _ = tick.tick() => {
                if handle.is_closed() {
                    return "the relay closed the connection".to_string();
                }
            }
        }
    }
}

/// T-624 (GitHub #790): when the first start fails, the supervisor ends
/// immediately with the exit code (69 unavailable, 68 unresolved name) and
/// the cause line instead of retrying forever.
#[cfg(test)]
#[tokio::test]
async fn a_first_start_that_fails_ends_with_the_exit_code() {
    pod_testkit::need_loopback!();
    let (_held, refused) = pod_testkit::refused::refused();

    // Silent server: accepts TCP and never writes.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let silent_addr = listener.local_addr().unwrap().to_string();
    let _server = tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((s, _)) = listener.accept().await {
            held.push(s);
        }
    });

    let cases = [
        (
            "refused port",
            refused.to_string(),
            podnet_core::exit::UNAVAILABLE,
            "transport",
        ),
        (
            "unresolved name",
            "nothing.invalid:443".to_string(),
            podnet_core::exit::NO_HOST,
            "could not resolve",
        ),
        (
            "silent server",
            silent_addr,
            podnet_core::exit::UNAVAILABLE,
            "did not answer",
        ),
    ];

    for (name, address, want_code, want_text) in cases {
        let cfg = RelayConfig {
            address,
            username: "test".into(),
            inactivity: None,
            connect_timeout: Duration::from_millis(300),
            ..RelayConfig::default()
        };
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let outcome = tokio::time::timeout(
            Duration::from_secs(3),
            supervise(cfg, Backoff::default(), tx),
        )
        .await
        .unwrap_or_else(|_| panic!("{name}: first start did not end"));
        let err = outcome.expect_err(&format!("{name}: expected failure"));
        assert_eq!(err.exit_code(), want_code, "{name}: {err}");
        assert!(err.to_string().contains(want_text), "{name}: {err}");
        assert!(
            rx.try_recv().is_err(),
            "{name}: must emit no Event::Down loop"
        );
    }
}
