//! The status of `tailscaled` and the tailnet guard (T-243).
//!
//! The guard reads the status only. It never changes the login, the exit
//! node or the prefs.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

use super::{parse_http_head, Dialer};

/// The time limit and the size limit of a read of the status.
const STATUS_TIMEOUT: Duration = Duration::from_secs(3);
const STATUS_LIMIT: u64 = 4 << 20;

/// How long the start waits for the tailnet, and how often the door
/// checks it again (choices, not measurements).
pub const GUARD_WAIT: Duration = Duration::from_secs(20);
pub const GUARD_RECHECK: Duration = Duration::from_secs(30);

/// The facts of `/localapi/v0/status` that the door uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub backend_state: String,
    pub tailnet_name: Option<String>,
    pub magic_dns_suffix: Option<String>,
}

impl Status {
    /// The tailnet as a person names it: the name, else the DNS suffix.
    pub fn tailnet(&self) -> Option<&str> {
        self.tailnet_name
            .as_deref()
            .or(self.magic_dns_suffix.as_deref())
    }
}

/// The answer of [`guard`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    Ready,
    Wait,
    /// The daemon runs in another tailnet. The text names it.
    Mismatch(String),
}

fn same(left: &str, right: &str) -> bool {
    left.trim_end_matches('.')
        .eq_ignore_ascii_case(right.trim_end_matches('.'))
}

/// Decide if the daemon is `Running` in the `expected` tailnet.
pub fn guard(expected: Option<&str>, status: &Status) -> Verdict {
    if status.backend_state != "Running" {
        return Verdict::Wait;
    }
    let Some(expected) = expected else {
        return Verdict::Ready;
    };
    let Some(actual) = status.tailnet() else {
        return Verdict::Wait;
    };
    let named = |value: &Option<String>| value.as_deref().is_some_and(|v| same(expected, v));
    if named(&status.tailnet_name) || named(&status.magic_dns_suffix) {
        Verdict::Ready
    } else {
        Verdict::Mismatch(actual.to_string())
    }
}

/// Why the guard did not open the door.
#[derive(Debug)]
pub enum GuardError {
    /// The daemon is in another tailnet (exit 78).
    Mismatch { expected: String, actual: String },
    /// The daemon was not ready in time (exit 69). The text says why.
    NotReady(String),
}

impl std::fmt::Display for GuardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Mismatch { expected, actual } => write!(
                f,
                "the daemon is in tailnet \"{actual}\", not \"{expected}\"; nothing was bound"
            ),
            Self::NotReady(why) => write!(f, "tailscaled is not ready for the tailnet: {why}"),
        }
    }
}

impl std::error::Error for GuardError {}

/// The settings of the guard.
#[derive(Clone, Debug)]
pub struct GuardConfig {
    pub expected: String,
    pub wait: Duration,
    pub recheck: Duration,
}

impl GuardConfig {
    pub fn new(expected: impl Into<String>) -> Self {
        Self {
            expected: expected.into(),
            wait: GUARD_WAIT,
            recheck: GUARD_RECHECK,
        }
    }
}

/// Poll the status until the daemon is `Running` in the expected tailnet.
/// A mismatch ends the wait at once.
pub async fn wait_ready(
    dialer: &Dialer,
    expected: &str,
    wait: Duration,
) -> Result<Status, GuardError> {
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        let last = match dialer.status().await {
            Ok(status) => match guard(Some(expected), &status) {
                Verdict::Ready => return Ok(status),
                Verdict::Mismatch(actual) => {
                    return Err(GuardError::Mismatch {
                        expected: expected.to_string(),
                        actual,
                    })
                }
                Verdict::Wait if status.backend_state == "Running" => {
                    "BackendState is Running and the tailnet is not known yet".to_string()
                }
                Verdict::Wait => format!("BackendState is {}", status.backend_state),
            },
            Err(err) => format!("unreadable status ({err})"),
        };
        if tokio::time::Instant::now() >= deadline {
            return Err(GuardError::NotReady(last));
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// The state that the door shares with its clients.
pub(super) struct Gate {
    expected: String,
    blocked: AtomicBool,
}

impl Gate {
    pub(super) fn new(expected: String) -> Arc<Self> {
        Arc::new(Self {
            expected,
            blocked: AtomicBool::new(false),
        })
    }

    pub(super) fn is_blocked(&self) -> bool {
        self.blocked.load(Ordering::SeqCst)
    }

    /// Read the status again. Only a mismatch blocks the door; a status
    /// that is not readable changes nothing.
    pub(super) async fn refresh(&self, dialer: &Dialer) {
        let Ok(status) = dialer.status().await else {
            return;
        };
        match guard(Some(&self.expected), &status) {
            Verdict::Mismatch(actual) => {
                if !self.blocked.swap(true, Ordering::SeqCst) {
                    crate::warn(&format!(
                        "tailscale: the daemon is now in tailnet \"{actual}\", not \"{}\": new dials are refused",
                        self.expected
                    ));
                }
            }
            Verdict::Ready => {
                if self.blocked.swap(false, Ordering::SeqCst) {
                    crate::warn(&format!(
                        "tailscale: the daemon is back in tailnet \"{}\": dials are allowed",
                        self.expected
                    ));
                }
            }
            Verdict::Wait => {}
        }
    }
}

impl Dialer {
    /// The status of `tailscaled` from `/localapi/v0/status`. A body that is
    /// too large or is not JSON is an error, never a state.
    pub async fn status(&self) -> io::Result<Status> {
        let read = async {
            let mut stream = UnixStream::connect(self.socket()).await?;
            // HTTP/1.0: the answer has no chunks, and the daemon closes.
            stream
                .write_all(b"GET /localapi/v0/status?peers=false HTTP/1.0\r\nHost: local-tailscaled.sock\r\n\r\n")
                .await?;
            let mut answer = Vec::new();
            (&mut stream)
                .take(STATUS_LIMIT)
                .read_to_end(&mut answer)
                .await?;
            Ok::<_, io::Error>(answer)
        };
        let answer = tokio::time::timeout(STATUS_TIMEOUT, read)
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "no status in 3 s"))??;
        status_of(&answer)
    }

    /// The `BackendState` of `tailscaled` (`Running`, `NeedsLogin`,
    /// `Stopped`, ...) (T-081).
    pub async fn backend_state(&self) -> io::Result<String> {
        Ok(self.status().await?.backend_state)
    }

    /// One line for a doctor: the socket, the state, the tailnet, and the
    /// value of `TS_FORCE_NOISE_443` as this process sees it.
    pub async fn describe(&self) -> Result<String, String> {
        let noise = std::env::var("TS_FORCE_NOISE_443").unwrap_or_else(|_| "unset".into());
        match self.status().await {
            Ok(status) => Ok(format!(
                "socket={} BackendState={} tailnet={} suffix={} TS_FORCE_NOISE_443={noise}",
                self.socket().display(),
                status.backend_state,
                status.tailnet_name.as_deref().unwrap_or("unknown"),
                status.magic_dns_suffix.as_deref().unwrap_or("unknown"),
            )),
            Err(err) => Err(format!(
                "socket={} does not answer ({err}) TS_FORCE_NOISE_443={noise}",
                self.socket().display()
            )),
        }
    }
}

/// The status of an HTTP answer of `/localapi/v0/status`.
pub(super) fn status_of(answer: &[u8]) -> io::Result<Status> {
    let bad = |what: &str| io::Error::new(io::ErrorKind::InvalidData, what.to_string());
    if answer.len() as u64 >= STATUS_LIMIT {
        return Err(bad("the status is too large"));
    }
    let split = answer
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| bad("the status has no end of head"))?;
    let head = String::from_utf8_lossy(&answer[..split]);
    let (status, _) = parse_http_head(&format!("{head}\r\n\r\n"))?;
    if status != 200 {
        return Err(bad(&format!("the status answered {status}")));
    }
    let body: serde_json::Value =
        serde_json::from_slice(&answer[split + 4..]).map_err(|e| bad(&e.to_string()))?;
    let text =
        |value: &serde_json::Value| value.as_str().filter(|s| !s.is_empty()).map(str::to_string);
    let backend_state =
        text(&body["BackendState"]).ok_or_else(|| bad("the status has no BackendState"))?;
    // Old daemons carry only the top-level MagicDNSSuffix: read both.
    let current = &body["CurrentTailnet"];
    Ok(Status {
        backend_state,
        tailnet_name: text(&current["Name"]),
        magic_dns_suffix: text(&current["MagicDNSSuffix"])
            .or_else(|| text(&body["MagicDNSSuffix"])),
    })
}
