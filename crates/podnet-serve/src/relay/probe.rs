//! A probe of the SSH relay (T-165): the TCP connect, the SSH banner, the
//! handshake and the auth with the user. The probe opens no session channel
//! and asks for no forward, so the relay publishes no URL.

use std::time::{Duration, Instant};

use super::*;

/// The stage of a probe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Connect,
    Banner,
    Handshake,
    Auth,
}

impl Stage {
    /// The name of the stage.
    pub fn name(self) -> &'static str {
        match self {
            Stage::Connect => "connect",
            Stage::Banner => "banner",
            Stage::Handshake => "handshake",
            Stage::Auth => "auth",
        }
    }
}

/// The stage that did not pass.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub stage: Stage,
    /// True when the stage did not end in the time limit: the relay did not
    /// answer, which is not a refusal.
    pub timed_out: bool,
    pub reason: String,
}

/// The result of a probe.
#[derive(Clone, Debug, Default)]
pub struct Probe {
    /// The time of the TCP connect (through the proxy, when one is set).
    pub connect: Option<Duration>,
    /// The SSH identification line of the relay.
    pub banner: Option<String>,
    /// True when the relay accepted the user.
    pub auth: Option<bool>,
    /// The time of the whole probe.
    pub total: Duration,
    /// The stage that failed, or `None` when all stages passed.
    pub failure: Option<Failure>,
}

/// The longest text that the probe reads before the SSH identification.
const BANNER_LIMIT: usize = 4096;

/// Run the stages in order. All stages end in `limit`.
pub async fn probe(config: &RelayConfig, limit: Duration) -> Probe {
    let start = Instant::now();
    let deadline = tokio::time::Instant::now() + limit;
    let mut out = Probe::default();
    let result = stages(config, deadline, start, &mut out).await;
    out.total = start.elapsed();
    out.failure = result.err();
    out
}

async fn within<T>(
    deadline: tokio::time::Instant,
    stage: Stage,
    limit: Duration,
    work: impl std::future::Future<Output = Result<T, String>>,
) -> Result<T, Failure> {
    match tokio::time::timeout_at(deadline, work).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(reason)) => Err(Failure {
            stage,
            timed_out: false,
            reason,
        }),
        Err(_) => Err(Failure {
            stage,
            timed_out: true,
            reason: format!("no answer in {} s", limit.as_secs().max(1)),
        }),
    }
}

async fn stages(
    config: &RelayConfig,
    deadline: tokio::time::Instant,
    start: Instant,
    out: &mut Probe,
) -> Result<(), Failure> {
    let limit = deadline.saturating_duration_since(tokio::time::Instant::now());
    let mut stream = within(deadline, Stage::Connect, limit, async {
        connect_transport(config).await.map_err(|e| e.to_string())
    })
    .await?;
    out.connect = Some(start.elapsed());

    // Read up to the identification line. The bytes go back to the front of
    // the stream, so the handshake sees them.
    let mut seen: Vec<u8> = Vec::new();
    let line = within(deadline, Stage::Banner, limit, async {
        let mut chunk = [0u8; 512];
        loop {
            if let Some(line) = identification(&seen) {
                return Ok(line);
            }
            if seen.len() >= BANNER_LIMIT {
                return Err("no SSH identification in the first 4096 bytes".to_string());
            }
            let n = stream.read(&mut chunk).await.map_err(|e| e.to_string())?;
            if n == 0 {
                return Err(if seen.is_empty() {
                    "the relay closed the connection and sent no byte".to_string()
                } else {
                    "the relay closed the connection before an SSH identification".to_string()
                });
            }
            seen.extend_from_slice(&chunk[..n]);
        }
    })
    .await?;
    out.banner = Some(line);
    seen.extend_from_slice(&stream.prefix[stream.offset..]);
    stream.prefix = seen;
    stream.offset = 0;

    let key = Arc::new(
        PrivateKey::random(&mut UnwrapErr(SysRng), Algorithm::Ed25519).map_err(|e| Failure {
            stage: Stage::Handshake,
            timed_out: false,
            reason: RelayError::Key(e.to_string()).to_string(),
        })?,
    );
    let ssh_config = Arc::new(Config {
        inactivity_timeout: Some(limit.max(Duration::from_secs(1))),
        preferred: russh::Preferred::DEFAULT,
        ..Config::default()
    });
    let client = Client::new(config.unix_socket.clone(), Arc::new(Mutex::new(0)));
    let mut handle = within(deadline, Stage::Handshake, limit, async {
        russh::client::connect_stream(ssh_config, stream, client)
            .await
            .map_err(|e| e.to_string())
    })
    .await?;
    let auth = within(deadline, Stage::Auth, limit, async {
        handle
            .authenticate_publickey(
                config.username.clone(),
                PrivateKeyWithHashAlg::new(key, None),
            )
            .await
            .map_err(|e| e.to_string())
    })
    .await?;
    let _ = handle
        .disconnect(russh::Disconnect::ByApplication, "podnet doctor", "")
        .await;
    out.auth = Some(auth.success());
    if auth.success() {
        Ok(())
    } else {
        Err(Failure {
            stage: Stage::Auth,
            timed_out: false,
            reason: format!("the relay rejected the user {:?}", config.username),
        })
    }
}

/// The line `SSH-2.0-...` in the bytes read so far, when it is complete.
fn identification(seen: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(seen);
    let mut complete = text.split_inclusive('\n');
    // The last piece may have no end of line yet.
    complete
        .find(|l| l.ends_with('\n') && l.starts_with("SSH-"))
        .map(|l| l.trim_end().to_string())
}
