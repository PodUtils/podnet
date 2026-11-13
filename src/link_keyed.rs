//! The carriers of `pod-link` that pin the Ed25519 key of each end and came
//! with podcrates v0.40.0: `quic` (T-273).
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
pub const NAMES: [&str; 1] = ["quic"];

/// The build feature that holds the carrier `name`, when it is one of
/// [`NAMES`].
pub fn feature_of(name: &str) -> Option<&'static str> {
    match name {
        "quic" => Some("quic"),
        _ => None,
    }
}

/// What the query of an address and the flags give to a carrier of
/// [`NAMES`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tune {}

impl Tune {
    /// The tune of the carrier `name` from `query`. A key that is for
    /// another carrier exits 64.
    pub fn of_query(name: &str, query: &Query) -> anyhow::Result<Tune> {
        let _ = (name, query);
        Ok(Tune::default())
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
}
