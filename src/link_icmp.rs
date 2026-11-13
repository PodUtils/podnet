//! The ICMP echo carrier of the link (T-282, feature `icmp`).
//!
//! `pod_link::icmp` of PodUtils/podcrates does the work: the frames of the
//! link ride in the data of ICMP echo messages (1400 bytes of payload), with
//! a session id that tells one link from another on one raw socket. The
//! selective-repeat ARQ of `pod-link` runs over the echoes, and TLS 1.3 (ALPN
//! `podlink/1`) over the ARQ, each side pinning the Ed25519 key of the other.
//! The socket is a raw ICMPv4 socket: it needs `CAP_NET_RAW` (or uid 0).
//! This module is the glue:
//!
//! * [`carrier`] makes the carrier from the key of this install and the
//!   allow list.
//! * [`listen`] takes an IPv4 address (`0.0.0.0`, `127.0.0.1`), with no
//!   port. A bad address exits 64 before the socket opens.
//! * [`dial`] connects to `KEY@HOST` (IPv4). It pins `KEY`.
//! * A host that refuses the raw socket exits 69 with the reason of the
//!   crate; a refused key exits 77 ([`crate::link::carrier_fault`]).
//! * The command line asks for `--allow-raw icmp` first
//!   ([`crate::link_keyed::allow_raw`]).

use std::net::Ipv4Addr;

use anyhow::anyhow;
use pod_identity::PublicKey;
use pod_link::icmp::Icmp;
use pod_link::{Link, Settings};

use crate::exit::{self, Fault};
use crate::identity::{AllowList, Identity};
use crate::link::{carrier_fault, Door};
use crate::link_keyed::{dialed, listen_on};
use crate::link_probe::{self, Probe};
use crate::link_tcp::{key_of, keys, provider};

/// The carrier of this install: it presents the key of `me`, and a
/// listener takes the clients of `allow`.
pub fn carrier(me: &Identity, allow: &AllowList) -> Icmp {
    Icmp::new(provider(), key_of(me)).with_clients(keys(allow))
}

/// Listen at `address` (an IPv4 address, no port) through `icmp`. An
/// address that is not IPv4 exits 64 before the host check.
pub async fn listen(icmp: &Icmp, address: &str) -> anyhow::Result<Door> {
    if address.parse::<Ipv4Addr>().is_err() {
        return Err(exit::fault(
            Fault::Usage,
            anyhow!("the icmp carrier listens at an IPv4 address with no port (0.0.0.0), not {address:?}"),
        ));
    }
    listen_on(icmp, address).await
}

/// Connect to `address` (`KEY@HOST`) through `icmp` as the dialer. The
/// address is checked first (exit 64), then the host (exit 69). The
/// handshake pins `KEY`. Then the key that the listener proved must be in
/// `allow` (else exit 77). It gives the link and that key.
pub async fn dial(
    icmp: &Icmp,
    allow: &AllowList,
    address: &str,
    settings: Settings,
) -> anyhow::Result<(Link, PublicKey)> {
    let (name, target) = Icmp::address(address).map_err(carrier_fault)?;
    pod_link::Carrier::available(icmp).map_err(carrier_fault)?;
    dialed(icmp.connect(&target, &name).await, allow, settings)
}

/// The carrier for the registry: the facts (the cost class `high`) and the
/// host check (a raw ICMP socket opens), no key of this install.
pub fn probe() -> Probe<Icmp> {
    Probe::new(Icmp::new(provider(), link_probe::key()))
}
