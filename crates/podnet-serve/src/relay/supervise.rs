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

use super::{open, Live, RelayConfig, TunnelHandle};

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
/// The future ends only when the receiver of `events` is dropped. Drop the
/// future to stop: that closes the live session.
pub async fn supervise(config: RelayConfig, backoff: Backoff, events: UnboundedSender<Event>) {
    let mut delay = backoff.initial;
    loop {
        let reason = match open(&config).await {
            Ok(session) => {
                let started = Instant::now();
                if events.send(Event::Up(session.tunnel.clone())).is_err() {
                    return;
                }
                let reason = wait_end(session).await;
                if started.elapsed() >= backoff.stable {
                    delay = backoff.initial;
                }
                reason
            }
            Err(err) => err.to_string(),
        };
        tracing::warn!(relay = %config.address, %reason, retry_in = ?delay, "the relay session is down");
        if events.send(Event::Down { reason }).is_err() {
            return;
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
