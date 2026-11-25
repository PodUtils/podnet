//! The row `nat` of `podnet doctor` (T-543): the class of the NAT of this
//! host by `pod-natprobe` (podcrates), with the profile `Fast` (600 ms), the
//! STUN servers of `pod-services` and the resolver of podnet. A server that
//! the egress policy refuses is not asked; a host where UDP does not leave
//! gets `unknown` with that reason and sends nothing.

use std::sync::Arc;
use std::time::Duration;

use pod_natprobe::report::{Connectivity, Report as NatReport};
use pod_natprobe::{Prober, Profile, Seams, ServerEndpoint, Settings};

use super::Verdict;
use crate::udp::Egress;

/// The time limit of the probe and of the lookup of its servers.
const LIMIT: Duration = Duration::from_secs(4);

/// The NAT of this host: the verdict, the classes of IPv4 and one line.
#[derive(Clone, Debug)]
pub struct Nat {
    pub verdict: Verdict,
    pub connectivity: Option<String>,
    pub mapping: Option<String>,
    pub filtering: Option<String>,
    pub detail: String,
}

impl Nat {
    fn unknown(detail: impl Into<String>) -> Nat {
        Nat {
            verdict: Verdict::Unknown,
            connectivity: None,
            mapping: None,
            filtering: None,
            detail: detail.into(),
        }
    }

    /// The row of a probe that did not end in the time limit of `--quick`.
    pub fn late(reason: &str) -> Nat {
        Nat::unknown(reason)
    }

    /// The fields of `--json`.
    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "verdict": self.verdict.name(),
            "connectivity": self.connectivity,
            "mapping": self.mapping,
            "filtering": self.filtering,
            "detail": self.detail,
        })
    }
}

/// Probe the NAT of this host. `udp` is the result of the UDP probe of the
/// doctor: where UDP does not leave, no STUN request goes out.
pub async fn nat(udp: &Egress) -> Nat {
    if let Egress::Refused(why) = udp {
        return Nat::unknown(format!(
            "UDP does not leave this host ({why}): no NAT probe"
        ));
    }
    let catalog = pod_services::Catalog::new();
    let resolve: Arc<dyn pod_core::Resolve> = crate::resolve::plain();
    let found = tokio::time::timeout(
        LIMIT,
        pod_natprobe::run::system::resolve_pod_services_stun(&catalog, resolve),
    )
    .await;
    let Ok((v4, _v6)) = found else {
        return Nat::unknown("no STUN server resolved in the time limit");
    };
    let v4 = permitted(v4);
    if v4.is_empty() {
        return Nat::unknown("no STUN server resolves, or the egress policy refuses each one");
    }
    let settings = Settings::for_profile(Profile::Fast, v4);
    match tokio::time::timeout(LIMIT, Prober::new().run(settings, Seams::system())).await {
        Ok(Ok(report)) => of_report(&report),
        Ok(Err(e)) => Nat::unknown(format!("the NAT probe cannot run: {e}")),
        Err(_) => Nat::unknown("the NAT probe did not end in the time limit"),
    }
}

/// The servers that the egress policy of the process lets through.
fn permitted(servers: Vec<ServerEndpoint>) -> Vec<ServerEndpoint> {
    servers
        .into_iter()
        .filter(|s| crate::policy::permit(&s.addr.ip().to_string(), s.addr.port()).is_ok())
        .collect()
}

/// The row of a report: `ok` with the classes of IPv4, `refused` where UDP
/// to the servers is blocked, else `unknown` with the line of the report.
fn of_report(report: &NatReport) -> Nat {
    let name = |v: Option<String>| v.map(|v| snake(&v));
    let Some(v4) = report.v4.as_ref() else {
        return Nat::unknown(report.to_string());
    };
    let connectivity = v4.connectivity.value().copied();
    let verdict = match connectivity {
        Some(Connectivity::UdpBlocked) => Verdict::Refused,
        Some(_) => Verdict::Ok,
        None => Verdict::Unknown,
    };
    Nat {
        verdict,
        connectivity: name(connectivity.map(|c| format!("{c:?}"))),
        mapping: name(v4.mapping.value().map(|m| format!("{m:?}"))),
        filtering: name(v4.filtering.value().map(|f| format!("{f:?}"))),
        detail: report.to_string(),
    }
}

/// `EndpointIndependent` -> `endpoint-independent`.
fn snake(word: &str) -> String {
    let mut out = String::new();
    for (i, c) in word.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            out.push('-');
        }
        out.extend(c.to_lowercase());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // T-543: a host where UDP does not leave sends no STUN request and
    // gets `unknown` with the reason; a class name reads as words.
    #[tokio::test]
    async fn a_host_with_no_udp_gets_no_probe() {
        let row = nat(&Egress::Refused("send: Operation not permitted".into())).await;
        assert_eq!(row.verdict, Verdict::Unknown);
        assert!(
            row.detail.contains("UDP does not leave this host"),
            "{}",
            row.detail
        );
        assert_eq!(row.json()["verdict"], "unknown");
        assert_eq!(
            snake("AddressAndPortDependent"),
            "address-and-port-dependent"
        );
        assert_eq!(snake("UdpBlocked"), "udp-blocked");
    }

    // T-543: the row of each class of NAT of the simulator of pod-natprobe:
    // a mapping is named; a blocked UDP path that a check of the network
    // corroborates is `refused` (the profile `Standard`; in the 600 ms of
    // `Fast`, which the doctor runs, it stays `unknown`); with no
    // corroboration it is `unknown`.
    #[test]
    fn each_class_of_nat_gives_its_row() {
        use pod_natprobe::report::{FilteringBehavior as F, MappingBehavior as M};
        use pod_natprobe::sim::{drive_sim, NatSpec, SimStunServer, SimWorld};
        use pod_natprobe::wire::AddrPolicy;
        use std::net::{IpAddr, Ipv4Addr, SocketAddr};
        let wan = IpAddr::V4(Ipv4Addr::new(198, 51, 100, 10));
        let pp = SocketAddr::from(([203, 0, 113, 1], 3478));
        let aa = SocketAddr::from(([203, 0, 113, 2], 3479));
        // (mapping, filtering, UDP blocked, corroborated, verdict, mapping word)
        let (fast, standard) = (Profile::Fast, Profile::Standard);
        let (eim, apdm) = (M::EndpointIndependent, M::AddressAndPortDependent);
        let (eif, apdf) = (F::EndpointIndependent, F::AddressAndPortDependent);
        let (ok, refused, unknown) = (Verdict::Ok, Verdict::Refused, Verdict::Unknown);
        // (profile, mapping, filtering, UDP blocked, corroborated, verdict,
        // mapping word)
        let table = [
            (
                fast,
                eim,
                eif,
                false,
                true,
                ok,
                Some("endpoint-independent"),
            ),
            (
                fast,
                apdm,
                apdf,
                false,
                true,
                ok,
                Some("address-and-port-dependent"),
            ),
            (fast, eim, eif, true, true, unknown, None),
            (standard, eim, eif, true, true, refused, None),
            (standard, eim, eif, true, false, unknown, None),
        ];
        for (profile, mapping, filtering, blocked, online, verdict, word) in table {
            let mut spec = NatSpec::home_eim_apdf(wan);
            spec.mapping = mapping;
            spec.filtering = filtering;
            spec.udp_blocked = blocked;
            // A blocked path is decided over two servers.
            let b = SocketAddr::from(([203, 0, 113, 3], 19302));
            let (servers, endpoints) = if blocked {
                (
                    vec![
                        SimStunServer::plain("s1", pp),
                        SimStunServer::plain("s2", b),
                    ],
                    vec![ServerEndpoint::new("s1", pp), ServerEndpoint::new("s2", b)],
                )
            } else {
                (
                    vec![SimStunServer::rfc5780("s1", pp, aa)],
                    vec![ServerEndpoint::rfc5780("s1", pp)],
                )
            };
            let mut world = SimWorld::new(Ipv4Addr::new(192, 168, 1, 10), vec![spec], servers, 7);
            world.online_ok = online;
            let mut settings =
                Settings::for_profile(profile, endpoints).with_addr_policy(AddrPolicy::Lab);
            settings.run_nat64 = false;
            settings.run_portmap = false;
            settings.run_ipv6 = false;
            let (report, _) = drive_sim(settings, &mut world, 7).expect("the simulator runs");
            let row = of_report(&report);
            let case =
                format!("{profile:?} {mapping:?} {filtering:?} blocked={blocked} online={online}");
            assert_eq!(row.verdict, verdict, "{case}: {}", row.detail);
            assert_eq!(row.mapping.as_deref(), word, "{case}: {}", row.detail);
            assert!(
                row.detail.starts_with("the NAT probe"),
                "{case}: {}",
                row.detail
            );
            if verdict == Verdict::Refused {
                assert_eq!(row.connectivity.as_deref(), Some("udp-blocked"), "{case}");
            }
        }
    }
}
