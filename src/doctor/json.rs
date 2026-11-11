//! The JSON document of the doctor: schema 1 (`docs/protocol.md`, section 9).

use super::*;

/// Milliseconds with one decimal.
fn ms(d: Option<Duration>) -> Option<f64> {
    d.map(|d| (d.as_secs_f64() * 10000.0).round() / 10.0)
}

impl Report {
    pub fn to_json(&self) -> Value {
        crate::status::stamped(json!({
            "ok": self.ok(),
            "quick": self.quick,
            "roads": self.roads().iter().map(|(road, verdict, reason)| json!({
                "road": road,
                "verdict": verdict.name(),
                "status": verdict.name(),
                "reason": reason,
            })).collect::<Vec<_>>(),
            "clock": {
                "verdict": self.clock.verdict.name(),
                "status": self.clock.verdict.name(),
                "skew_s": self.clock.skew,
                "detail": self.clock.detail,
            },
            "git": self.git.to_json(),
            "proxy": self.proxy,
            "paths": self.paths.iter().map(|p| json!({
                "id": p.target.id,
                "target": p.target.authority(),
                "via": p.via,
                "verdict": p.verdict.name(),
                "status": p.verdict.name(),
                "detail": p.detail,
                "samples": {
                    "sent": p.sent,
                    "answered": p.answered,
                    "loss": p.loss(),
                    "best_ms": ms(p.best),
                    "median_ms": ms(p.median),
                    "p95_ms": ms(p.p95),
                    "jitter_ms": ms(p.jitter),
                },
                "tls": p.tls.as_ref().map(|t| json!({
                    "verdict": t.verdict.name(),
                    "status": t.verdict.name(),
                    "issuer": t.issuer,
                    "detail": t.detail,
                })),
                "dns": p.dns.as_ref().map(|d| match d {
                    Ok((addrs, took)) => json!({
                        "addresses": addrs.iter().map(|a| a.to_string()).collect::<Vec<_>>(),
                        "ms": ms(Some(*took)),
                    }),
                    Err(e) => json!({ "error": e }),
                }),
            })).collect::<Vec<_>>(),
            "udp": {
                "target": self.udp.0.to_string(),
                "verdict": self.udp.1.name(),
                "status": match self.udp.1 {
                    Egress::Answered(_) => "ok",
                    Egress::Refused(_) => "fail",
                    Egress::Unknown(_) => "unknown",
                },
                "detail": self.udp.1.detail(),
            },
            "names": self.names,
            "next": self.next(),
            "legs": self.legs.iter().map(|l| json!({
                "leg": l.name,
                "verdict": l.verdict.name(),
                "status": l.verdict.name(),
                "reason": l.reason,
            })).collect::<Vec<_>>(),
        }))
    }
}
