//! A WARP session that comes back (issue 75): `pod-warp`'s [`Live`], with
//! podnet's `--probe`, `--session-max-age`, the split of the first
//! ClientHello (`--fragment-client-hello`, issue 135) and the record of the
//! instance.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::Result;
use pod_tls::fragment::{Part, Split};
use tracing::info;

pub mod instance;

pub use pod_warp::live::{
    Closable, Connect, Live, Probe, ProbeFuture, Report, DEFAULT_PROBE, RENEW_DRAIN, WAIT_FOR_NEW,
};

/// The probe of this process (None: no probe).
static PROBE: std::sync::Mutex<Option<Probe>> = std::sync::Mutex::new(Some(DEFAULT_PROBE));

/// `--probe IP:PORT` or `--probe off` (T-068).
pub fn parse_probe(spec: &str) -> Result<Option<Probe>> {
    pod_warp::live::parse_probe(spec)
        .map_err(|_| anyhow::anyhow!("--probe={spec:?}: give IP:PORT or off"))
}

/// Set the probe of the sessions of this process (None: no probe).
pub fn set_probe(probe: Option<Probe>) {
    *PROBE.lock().unwrap_or_else(|e| e.into_inner()) = probe;
}

/// The option `--probe` for `probe` (warp up gives it to its child).
pub fn probe_arg(probe: Option<Probe>) -> String {
    format!(
        "--probe={}",
        probe.map_or("off".into(), |p| p.target.to_string())
    )
}

/// The probe of this process (None: no probe).
pub fn probe() -> Option<Probe> {
    *PROBE.lock().unwrap_or_else(|e| e.into_inner())
}

/// `--session-max-age` (issue 185): each session of [`Live`] is closed on
/// purpose at this age, and a new one opens. Seconds; 0 is off.
static MAX_AGE: AtomicU64 = AtomicU64::new(0);

/// Set the age at which a session of [`Live`] is renewed (None: never).
pub fn set_max_age(age: Option<Duration>) {
    MAX_AGE.store(age.map_or(0, |a| a.as_secs().max(1)), Ordering::SeqCst);
}

/// The age at which a session is renewed (None: never).
pub fn max_age() -> Option<Duration> {
    match MAX_AGE.load(Ordering::SeqCst) {
        0 => None,
        secs => Some(Duration::from_secs(secs)),
    }
}

/// `--fragment-client-hello` and `--fragment-size` (issue 135): the split of
/// the first ClientHello of each MASQUE connection, when one is set.
static SPLIT: std::sync::OnceLock<Split> = std::sync::OnceLock::new();

/// The split of `--fragment-client-hello` (`off`, `record` or `sni`; `off`
/// gives `None`) and `--fragment-size` (`N` or `MIN-MAX`). A bad size is an
/// error also with `off` (issue 252).
pub fn parse_split(mode: &str, size: Option<&str>) -> Result<Option<Split>> {
    Split::parse(mode, size).map_err(|err| match err {
        pod_tls::Error::Split {
            part: Part::Mode,
            value,
        } => anyhow::anyhow!(
            "--fragment-client-hello={value:?}: give {}",
            Part::Mode.accepted()
        ),
        pod_tls::Error::Split {
            part: Part::Size,
            value,
        } => anyhow::anyhow!("--fragment-size={value}: give {}", Part::Size.accepted()),
        other => other.into(),
    })
}

/// Set the split of this process. A later call does nothing.
pub fn set_split(split: Split) {
    if SPLIT.set(split).is_ok() {
        info!(mode = ?split.mode, min = split.min, max = split.max, "the first ClientHello of each MASQUE connection is split across records");
    }
}

/// The split of the MASQUE connections, when one is set.
pub fn split() -> Option<Split> {
    SPLIT.get().copied()
}

/// The settings of a [`Live`] of this process: the backoff of 1 s to 30 s,
/// `--session-max-age`, `--probe`, and podnet's log.
pub fn settings() -> pod_warp::LiveSettings {
    pod_warp::LiveSettings {
        max_age: max_age(),
        probe: probe(),
        report: super::report(),
        ..pod_warp::LiveSettings::default()
    }
}

/// Start a [`Live`] with `first` and the settings of this process; the
/// first [`Live`] of the process writes its report in the record of the
/// instance.
pub fn start<T: Closable>(first: T, connect: Connect<T>) -> std::sync::Arc<Live<T>> {
    let live = Live::start(first, connect, &settings());
    instance::publish_own(&live);
    live
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_probe_option() {
        assert_eq!(parse_probe("off").unwrap(), None);
        let probe = parse_probe("192.0.2.1:80").unwrap().unwrap();
        assert_eq!(probe.target, "192.0.2.1:80".parse().unwrap());
        let err = parse_probe("nope").unwrap_err().to_string();
        assert!(err.starts_with("--probe=\"nope\""), "{err}");
        assert_eq!(probe_arg(None), "--probe=off");
        assert_eq!(probe_arg(Some(probe)), "--probe=192.0.2.1:80");
    }
}
