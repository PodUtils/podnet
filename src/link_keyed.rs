//! The carriers of `pod-link` that pin the Ed25519 key of each end and came
//! with podcrates v0.40.0: `quic` (T-273), `udp` (T-276), and the poll
//! carriers `poll`, `sse`, `chunked` and `httppoll` (T-271, T-272, T-280).
//!
//! `link listen`, `link dial` and `link connect` open them by name through
//! [`dial`] and [`listen`]. Each one sits behind a build feature
//! ([`feature_of`]); a name whose feature is off exits 69 and names the
//! feature. [`Tune`] holds what the query of the address and the flags give
//! to a carrier.

use anyhow::anyhow;
use pod_identity::PublicKey;
use pod_link::{BoxIo, Carrier, CarrierError, Link, Role, Settings};

use crate::build_features;
use crate::exit::{self, Fault};
use crate::identity::{AllowList, Identity};
use crate::link::{self, carrier_fault, Door};
use crate::link_query::Query;

/// The names of these carriers, the words of `--carrier`.
pub const NAMES: [&str; 6] = ["quic", "udp", "poll", "sse", "chunked", "httppoll"];

/// The build feature that holds the carrier `name`, when it is one of
/// [`NAMES`].
pub fn feature_of(name: &str) -> Option<&'static str> {
    match name {
        "quic" => Some("quic"),
        "udp" => Some("udp"),
        "poll" | "sse" | "chunked" | "httppoll" => Some("poll"),
        _ => None,
    }
}

/// What the query of an address and the flags give to a carrier of
/// [`NAMES`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tune {
    /// `kcp-nodelay=1` (udp): the "nodelay" of KCP with the largest window.
    pub kcp_nodelay: bool,
    /// `mtu=N` (udp): the largest datagram of KCP, 64 to 1200 bytes.
    pub mtu: Option<usize>,
    /// `poll=N` (the poll carriers): the hold of a long poll, 1 to 60 s.
    pub hold: Option<u8>,
}

/// The largest `mtu` of the udp carrier: no IP fragmentation (GitHub #468).
pub const UDP_MTU: (usize, usize) = (64, 1200);

impl Tune {
    /// The tune of the carrier `name` from `query`. A key that is for
    /// another carrier exits 64.
    pub fn of_query(name: &str, query: &Query) -> anyhow::Result<Tune> {
        let only = |key: &str, carrier: &str| {
            usage(format!(
                "the query key {key} is for the {carrier} carrier, not for {name}"
            ))
        };
        let mut tune = Tune::default();
        if let Some(nodelay) = query.kcp_nodelay {
            if name != "udp" {
                return Err(only("kcp-nodelay", "udp"));
            }
            tune.kcp_nodelay = nodelay;
        }
        if let Some(hold) = query.poll {
            if feature_of(name) != Some("poll") {
                return Err(only("poll", "poll, sse, chunked or httppoll"));
            }
            tune.hold = Some(hold);
        }
        if let (Some(mtu), "udp") = (query.mtu, name) {
            if !(UDP_MTU.0..=UDP_MTU.1).contains(&mtu) {
                return Err(usage(format!(
                    "the query key mtu: the udp carrier takes {} to {} bytes",
                    UDP_MTU.0, UDP_MTU.1
                )));
            }
            tune.mtu = Some(mtu);
        }
        Ok(tune)
    }
}

fn usage(text: String) -> anyhow::Error {
    exit::fault(Fault::Usage, anyhow!(text))
}

/// The error of a name that this build cannot open: its feature is off
/// (exit 69), or it is no carrier of [`NAMES`] (exit 64).
fn off(name: &str) -> anyhow::Error {
    match feature_of(name).map(build_features::require) {
        Some(Err(e)) => e,
        _ => usage(format!(
            "{name:?} is not one of the carriers {}",
            NAMES.join(", ")
        )),
    }
}

/// Listen at `address` through `carrier`: the host check, then the bind.
pub async fn listen_on<C: Carrier>(carrier: &C, address: &str) -> anyhow::Result<Door> {
    carrier.available().map_err(carrier_fault)?;
    let listener = carrier.listen(address).await.map_err(carrier_fault)?;
    Ok(Door::from_listener(listener))
}

/// The end of a dial: the key that the listener proved must be in `allow`
/// (else exit 77). It gives the link and that key.
pub fn dialed(
    connected: Result<(BoxIo, PublicKey), CarrierError>,
    allow: &AllowList,
    settings: Settings,
) -> anyhow::Result<(Link, PublicKey)> {
    let (io, proved) = connected.map_err(carrier_fault)?;
    allow.check(&proved)?;
    Ok((link::from_io(io, Role::Dialer, settings)?, proved))
}

/// Connect to `address` through the carrier `name` as the dialer. The
/// handshake pins the key of the address; the key that the listener
/// proved must be in `allow` (else exit 77).
pub async fn dial(
    name: &str,
    tune: &Tune,
    me: &Identity,
    allow: &AllowList,
    address: &str,
    settings: Settings,
) -> anyhow::Result<(Link, PublicKey)> {
    let _ = (&tune, &me, &allow, &address, &settings);
    match name {
        #[cfg(feature = "quic")]
        "quic" => {
            let carrier = crate::link_quic::carrier(me, allow);
            crate::link_quic::dial(&carrier, allow, address, settings).await
        }
        #[cfg(feature = "udp")]
        "udp" => {
            let carrier = crate::link_udp::carrier(me, allow, tune);
            crate::link_udp::dial(&carrier, allow, address, settings).await
        }
        #[cfg(feature = "poll")]
        "poll" | "sse" | "chunked" | "httppoll" => {
            let form = crate::link_poll::form_of(name).expect("a poll form");
            let carrier = crate::link_poll::carrier(me, allow, form, tune);
            crate::link_poll::dial(&carrier, allow, address, settings).await
        }
        _ => Err(off(name)),
    }
}

/// Listen at `address` through the carrier `name`. A client must prove a
/// key of `allow`.
pub async fn listen(
    name: &str,
    tune: &Tune,
    me: &Identity,
    allow: &AllowList,
    address: &str,
) -> anyhow::Result<Door> {
    let _ = (&tune, &me, &allow, &address);
    match name {
        #[cfg(feature = "quic")]
        "quic" => crate::link_quic::listen(&crate::link_quic::carrier(me, allow), address).await,
        #[cfg(feature = "udp")]
        "udp" => {
            let carrier = crate::link_udp::carrier(me, allow, tune);
            crate::link_udp::listen(&carrier, address).await
        }
        #[cfg(feature = "poll")]
        "poll" | "sse" | "chunked" | "httppoll" => {
            let form = crate::link_poll::form_of(name).expect("a poll form");
            let carrier = crate::link_poll::carrier(me, allow, form, tune);
            crate::link_poll::listen(&carrier, address).await
        }
        _ => Err(off(name)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_name_has_an_optional_feature() {
        for name in NAMES {
            let feature = feature_of(name).unwrap();
            assert!(build_features::OPTIONAL.contains(&feature), "{name}");
        }
        assert_eq!(feature_of("unix"), None);
    }

    #[test]
    fn a_name_that_is_off_or_unknown_has_its_exit_code() {
        let unknown = off("pigeon");
        assert_eq!(exit::code_of(&unknown), 64, "{unknown:#}");
        for name in NAMES {
            let feature = feature_of(name).unwrap();
            if !build_features::built(feature) {
                let e = off(name);
                assert_eq!(exit::code_of(&e), 69, "{e:#}");
                assert!(format!("{e:#}").contains(feature), "{e:#}");
            }
        }
    }

    #[test]
    fn the_keys_of_udp_are_for_udp_only() {
        let q = crate::link_query::parse("kcp-nodelay=1&mtu=576").unwrap();
        let tune = Tune::of_query("udp", &q).unwrap();
        assert!(tune.kcp_nodelay);
        assert_eq!(tune.mtu, Some(576));
        let e = Tune::of_query("quic", &q).unwrap_err();
        assert_eq!(exit::code_of(&e), 64, "{e:#}");
        assert!(format!("{e:#}").contains("kcp-nodelay"), "{e:#}");
        let big = crate::link_query::parse("mtu=1500").unwrap();
        let e = Tune::of_query("udp", &big).unwrap_err();
        assert!(format!("{e:#}").contains("64 to 1200"), "{e:#}");
        assert_eq!(
            Tune::of_query("udp", &Query::default()).unwrap(),
            Tune::default()
        );
    }
}
