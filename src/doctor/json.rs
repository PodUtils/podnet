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
            "lanes": self.lanes_json(),
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
            "ssh_relay": self.ssh_relay.to_json(),
            "carriers": self.carriers.to_json(),
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

    /// The `lanes` object: the seven egress lanes of [`lanes`].
    fn lanes_json(&self) -> Value {
        let mut map = serde_json::Map::new();
        for lane in lanes(self) {
            map.insert(
                lane.name.to_string(),
                json!({
                    "verdict": lane.verdict.name(),
                    "ms": ms(lane.time),
                    "reason": lane.reason,
                }),
            );
        }
        Value::Object(map)
    }
}

/// The names of the lanes, in the order of the document.
pub const LANES: [&str; 7] = [
    "tunnel_quic",
    "tunnel_http2",
    "tunnel_quic_warp",
    "masque_tcp443",
    "masque_udp443",
    "ws_upgrade",
    "proxy",
];

/// One egress lane (T-217).
#[derive(Clone, Debug)]
pub struct Lane {
    pub name: &'static str,
    pub verdict: Verdict,
    /// The best sample of the TCP path of the lane, when it has one.
    pub time: Option<Duration>,
    pub reason: String,
}

/// Each lane, from the legs and the paths, in the order of [`LANES`]. The
/// sources:
///
/// * `tunnel_quic`: the leg `quic`.
/// * `tunnel_http2`: the leg `http2`; the time of the path `edge-address`.
/// * `tunnel_quic_warp`: the leg `quic+warp`; the time of the MASQUE path.
/// * `masque_tcp443`: the path `masque` (`masque-v6` when only it works).
/// * `masque_udp443`, `ws_upgrade`: the probes of `lane_probes` (T-260).
/// * `proxy`: the paths that go through a proxy.
pub fn lanes(report: &Report) -> Vec<Lane> {
    let leg = |name: &str| report.legs.iter().find(|l| l.name == name);
    let path = |id: &str| report.paths.iter().find(|p| p.target.id == id);
    let from_leg = |lane: &'static str, name: &str, time: Option<Duration>| match leg(name) {
        Some(l) => Lane {
            name: lane,
            verdict: l.verdict,
            time,
            reason: l.reason.clone(),
        },
        None => Lane {
            name: lane,
            verdict: Verdict::Unknown,
            time: None,
            reason: format!("the leg {name} was not tested"),
        },
    };
    // The MASQUE path that the leg quic+warp uses.
    let masque = [path("masque"), path("masque-v6")]
        .into_iter()
        .flatten()
        .find(|p| p.usable().0 == Verdict::Ok)
        .or_else(|| path("masque"));
    let masque_tcp = match masque {
        Some(p) => {
            let (verdict, why) = p.usable();
            Lane {
                name: "masque_tcp443",
                verdict,
                time: p.best,
                reason: format!("{}: {why}", p.target.authority()),
            }
        }
        None => Lane {
            name: "masque_tcp443",
            verdict: Verdict::Unknown,
            time: None,
            reason: "the MASQUE path was not tested".into(),
        },
    };
    let edge_time = path("edge-address").and_then(|p| p.best);
    vec![
        from_leg("tunnel_quic", "quic", None),
        from_leg("tunnel_http2", "http2", edge_time),
        from_leg("tunnel_quic_warp", "quic+warp", masque.and_then(|p| p.best)),
        masque_tcp,
        from_probe("masque_udp443", &report.probes.masque_udp),
        from_probe("ws_upgrade", &report.probes.ws_upgrade),
        proxy_lane(&report.paths),
    ]
}

/// A lane from the result of its probe.
fn from_probe(name: &'static str, probe: &LaneProbe) -> Lane {
    Lane {
        name,
        verdict: probe.verdict,
        time: probe.time,
        reason: probe.reason.clone(),
    }
}

/// The lane `proxy`: `ok` when a path through the proxy has TCP and TLS,
/// `fail` when each path through the proxy fails, else `unknown`.
fn proxy_lane(paths: &[Path]) -> Lane {
    let via: Vec<&Path> = paths.iter().filter(|p| p.via.is_some()).collect();
    let lane = |verdict, time, reason: String| Lane {
        name: "proxy",
        verdict,
        time,
        reason,
    };
    let Some(first) = via.first() else {
        return lane(Verdict::Unknown, None, "no proxy is set".into());
    };
    let proxy = first.via.clone().unwrap_or_default();
    let works: Vec<&&Path> = via.iter().filter(|p| p.usable().0 == Verdict::Ok).collect();
    if !works.is_empty() {
        let time = works.iter().filter_map(|p| p.best).min();
        return lane(
            Verdict::Ok,
            time,
            format!(
                "{} of {} paths work through {proxy}",
                works.len(),
                via.len()
            ),
        );
    }
    let verdict = if via.iter().all(|p| p.usable().0 == Verdict::Refused) {
        Verdict::Refused
    } else {
        Verdict::Unknown
    };
    lane(verdict, None, format!("{proxy}: {}", first.usable().1))
}
