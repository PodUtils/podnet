//! The UDP carrier of the link with KCP (T-276, feature `udp`).
//!
//! `pod_link::udp` of PodUtils/podcrates does the work: KCP (the crate
//! `kcp`, a port of ikcp) over UDP in datagrams of at most 1200 bytes and
//! segments of 1 KiB, TLS 1.3 above it with the ALPN `podlink/1`, and a pin
//! of the Ed25519 key of the peer on each side. This module is the glue, as
//! [`crate::link_tcp`]:
//!
//! * [`carrier`] makes the carrier from the key of this install, the allow
//!   list and the settings of KCP ([`kcp`]). A dial sends from a UDP socket
//!   of the system: the way out of the process ([`crate::egress`]) carries
//!   no datagrams.
//! * [`kcp`] gives the settings of the query of the address: `kcp-nodelay=1`
//!   turns on the "nodelay" of KCP with the largest window, `mtu=N` sets the
//!   largest datagram.
//! * [`listen`] binds `IP:PORT` on UDP. [`dial`] connects to
//!   `KEY@HOST:PORT`.
//! * A refused key exits 77 ([`crate::link::carrier_fault`]).

use pod_identity::PublicKey;
use pod_link::udp::{KcpSettings, Udp, KCP_HEADER, MAX_WINDOW, SEGMENT};
use pod_link::{Link, Settings};

use crate::identity::{AllowList, Identity};
use crate::link::{carrier_fault, Door};
use crate::link_keyed::{dialed, listen_on, Tune};
use crate::link_probe::{self, Probe};
use crate::link_tcp::{key_of, keys, provider};

/// The settings of KCP for `tune`. The default is the congestion window of
/// KCP with "nodelay" off. `kcp-nodelay=1` turns "nodelay" on with a window
/// of [`MAX_WINDOW`] segments (podcrates v0.40.0: at 20 % drop, 3.7 to 5.3
/// times the clean time, against 9 to 17 times with a window of 256). It is
/// unfair to the other flows of a path. `mtu` sets the largest datagram;
/// a segment then fits in it with its header.
pub fn kcp(tune: &Tune) -> KcpSettings {
    let mut kcp = KcpSettings::default();
    if tune.kcp_nodelay {
        kcp.nodelay = true;
        kcp.window = MAX_WINDOW;
    }
    if let Some(mtu) = tune.mtu {
        kcp.mtu = mtu;
        kcp.segment = SEGMENT.min(mtu.saturating_sub(KCP_HEADER));
    }
    kcp
}

/// The carrier of this install: it presents the key of `me`, a listener
/// takes the clients of `allow`, and KCP has the settings of `tune`.
pub fn carrier(me: &Identity, allow: &AllowList, tune: &Tune) -> Udp {
    let mut udp = Udp::new(provider(), key_of(me)).with_clients(keys(allow));
    udp.kcp = kcp(tune);
    udp
}

/// Listen at `address` (`IP:PORT`, UDP) through `udp`.
pub async fn listen(udp: &Udp, address: &str) -> anyhow::Result<Door> {
    listen_on(udp, address).await
}

/// Connect to `address` (`KEY@HOST:PORT`) through `udp` as the dialer.
/// The handshake pins `KEY`. Then the key that the listener proved must be
/// in `allow` (else exit 77). It gives the link and that key.
pub async fn dial(
    udp: &Udp,
    allow: &AllowList,
    address: &str,
    settings: Settings,
) -> anyhow::Result<(Link, PublicKey)> {
    let (name, target) = Udp::address(address).map_err(carrier_fault)?;
    pod_link::Carrier::available(udp).map_err(carrier_fault)?;
    dialed(udp.connect(&target, &name).await, allow, settings)
}

/// The carrier for the registry: the facts and the host check, no key of
/// this install.
pub fn probe() -> Probe<Udp> {
    Probe::new(Udp::new(provider(), link_probe::key()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_query_sets_kcp() {
        let base = kcp(&Tune::default());
        assert_eq!(base, KcpSettings::default());
        assert!(!base.nodelay, "nodelay is off by default");
        let fast = kcp(&Tune {
            kcp_nodelay: true,
            ..Tune::default()
        });
        assert!(fast.nodelay);
        assert_eq!(fast.window, MAX_WINDOW);
        let small = kcp(&Tune {
            mtu: Some(576),
            ..Tune::default()
        });
        assert_eq!(small.mtu, 576);
        assert_eq!(small.segment, 576 - KCP_HEADER);
        assert!(small.check().is_ok());
    }
}
