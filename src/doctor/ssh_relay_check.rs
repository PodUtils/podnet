//! The SSH relay of `podnet ssh-relay` (T-165): the TCP connect, the SSH
//! banner and the auth with the user. The probe opens no session and asks
//! for no forward.

use super::*;
use crate::relay::probe::{probe, Failure, Probe};
use crate::relay::{RelayConfig, UrlRule};

/// The relay to test.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshRelayTarget {
    /// `HOST:PORT`.
    pub address: String,
    pub user: String,
    /// An HTTP CONNECT proxy, `HOST:PORT`.
    pub proxy: Option<String>,
    /// Where the target comes from: `flag`, or `instance NAME`.
    pub source: String,
}

/// The row of the SSH relay.
#[derive(Clone, Debug)]
pub struct SshRelay {
    pub verdict: Verdict,
    pub target: Option<SshRelayTarget>,
    pub probe: Option<Probe>,
    pub detail: String,
}

fn ms(d: Duration) -> f64 {
    (d.as_secs_f64() * 10000.0).round() / 10.0
}

impl SshRelay {
    /// The row when no relay is given.
    pub(super) fn none() -> Self {
        Self {
            verdict: Verdict::Unknown,
            target: None,
            probe: None,
            detail: "no SSH relay is given".into(),
        }
    }

    pub(super) fn late() -> Self {
        Self {
            verdict: Verdict::Unknown,
            target: None,
            probe: None,
            detail: LATE.into(),
        }
    }

    pub(super) fn to_json(&self) -> Value {
        let probe = self.probe.as_ref();
        json!({
            "verdict": self.verdict.name(),
            "status": self.verdict.name(),
            "relay": self.target.as_ref().map(|t| t.address.clone()),
            "user": self.target.as_ref().map(|t| t.user.clone()),
            "source": self.target.as_ref().map(|t| t.source.clone()),
            "connect_ms": probe.and_then(|p| p.connect).map(ms),
            "banner": probe.and_then(|p| p.banner.clone()),
            "auth": probe.and_then(|p| p.auth),
            "total_ms": probe.map(|p| ms(p.total)),
            "detail": self.detail,
        })
    }

    pub(super) fn line(&self) -> String {
        format!("\nssh-relay {}: {}\n", self.verdict.name(), self.detail)
    }
}

/// Test `target`. All stages end in `limit`.
pub(super) async fn ssh_relay(target: SshRelayTarget, limit: Duration) -> SshRelay {
    let config = RelayConfig {
        address: target.address.clone(),
        proxy: target.proxy.clone(),
        username: target.user.clone(),
        url_rule: UrlRule::default(),
        ..RelayConfig::default()
    };
    let result = probe(&config, limit).await;
    let (verdict, detail) = judge(&target, &result);
    SshRelay {
        verdict,
        target: Some(target),
        probe: Some(result),
        detail,
    }
}

/// The verdict of a probe: `ok` when each stage passed, `unknown` when a
/// stage had no answer in time, else `fail`.
pub(super) fn judge(target: &SshRelayTarget, probe: &Probe) -> (Verdict, String) {
    let who = format!("{} ({})", target.address, target.source);
    match &probe.failure {
        None => (
            Verdict::Ok,
            format!(
                "{who}: connect {} ms, banner {}, auth ok for {}, total {} ms; podnet ssh-relay can run",
                ms(probe.connect.unwrap_or_default()),
                probe.banner.as_deref().unwrap_or("-"),
                target.user,
                ms(probe.total),
            ),
        ),
        Some(Failure {
            stage,
            timed_out,
            reason,
        }) => (
            if *timed_out {
                Verdict::Unknown
            } else {
                Verdict::Refused
            },
            format!("{who}: {} failed: {reason}", stage.name()),
        ),
    }
}

/// The relay of a running `ssh-relay` instance, from its record: the
/// flags `--server`, `--user` and `--connect-proxy`.
pub(super) fn target_of_record(record: &crate::instances::Record) -> Option<SshRelayTarget> {
    if record.command != "ssh-relay" {
        return None;
    }
    let flag = |name: &str| {
        let with = format!("{name}=");
        record.args.iter().enumerate().find_map(|(i, a)| {
            a.strip_prefix(&with).map(str::to_string).or_else(|| {
                (a == name)
                    .then(|| record.args.get(i + 1).cloned())
                    .flatten()
            })
        })
    };
    Some(SshRelayTarget {
        address: flag("--server")?,
        user: flag("--user")?,
        proxy: flag("--connect-proxy"),
        source: format!("instance {}", record.name),
    })
}

/// The relay of the first live `ssh-relay` instance of the run directory.
pub(super) fn instance_target() -> Option<SshRelayTarget> {
    crate::instances::list(&crate::instances::lookup_dir())
        .iter()
        .filter(|e| e.alive)
        .find_map(|e| target_of_record(&e.record))
}
