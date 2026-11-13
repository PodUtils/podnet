//! The QUIC carrier of the link (T-273, feature `quic`).
//!
//! `pod_link::quic` of PodUtils/podcrates does the work: one bidirectional
//! stream of a QUIC connection of `pod-quic`, TLS 1.3 with the ALPN
//! `podlink/1` (as the `tcp` carrier), and a pin of the Ed25519 key of the
//! peer on each side. This module is the glue, as [`crate::link_tcp`]:
//!
//! * [`carrier`] makes the carrier from the key of this install and the
//!   allow list. The list feeds the client keys of the listener. A dial
//!   sends from a UDP socket of the system: the way out of the process
//!   ([`crate::egress`]) carries no datagrams.
//! * [`listen`] binds `IP:PORT` on UDP.
//! * [`dial`] connects to `KEY@HOST:PORT`. It pins `KEY`, and it checks the
//!   key that the listener proved against the allow list.
//! * A refused key exits 77 ([`crate::link::carrier_fault`]).

use pod_identity::PublicKey;
use pod_link::quic::Quic;
use pod_link::{Link, Settings};

use crate::identity::{AllowList, Identity};
use crate::link::{carrier_fault, Door};
use crate::link_keyed::{dialed, listen_on};
use crate::link_probe::{self, Probe};
use crate::link_tcp::{key_of, keys, provider};

/// The carrier of this install: it presents the key of `me`, and a
/// listener takes the clients of `allow`.
pub fn carrier(me: &Identity, allow: &AllowList) -> Quic {
    Quic::new(provider(), key_of(me)).with_clients(keys(allow))
}

/// Listen at `address` (`IP:PORT`, UDP) through `quic`.
pub async fn listen(quic: &Quic, address: &str) -> anyhow::Result<Door> {
    listen_on(quic, address).await
}

/// Connect to `address` (`KEY@HOST:PORT`) through `quic` as the dialer.
/// The handshake pins `KEY`. Then the key that the listener proved must be
/// in `allow` (else exit 77). It gives the link and that key.
pub async fn dial(
    quic: &Quic,
    allow: &AllowList,
    address: &str,
    settings: Settings,
) -> anyhow::Result<(Link, PublicKey)> {
    let (name, target) = Quic::address(address).map_err(carrier_fault)?;
    pod_link::Carrier::available(quic).map_err(carrier_fault)?;
    dialed(quic.connect(&target, &name).await, allow, settings)
}

/// The carrier for the registry: the facts and the host check, no key of
/// this install.
pub fn probe() -> Probe<Quic> {
    Probe::new(Quic::new(provider(), link_probe::key()))
}
