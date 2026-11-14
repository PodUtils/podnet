//! The WireGuard tunnel of WARP (issue 152): `pod-warp`'s, on a UDP socket
//! of the host, and the peer of a device of `warp export`.

use std::net::SocketAddr;

use anyhow::{Context as _, Result};

use super::Warped as _;

pub use pod_warp::wg::*;

/// The peer of the device of `warp export`, at `endpoint` or at the
/// WireGuard door of its record.
pub fn from_exported(
    exported: &super::export::Exported,
    endpoint: Option<SocketAddr>,
) -> Result<Peer> {
    use base64::Engine as _;
    let b64 = base64::engine::general_purpose::STANDARD;
    let device = &exported.device;
    let private: [u8; 32] = b64
        .decode(&exported.private_key)
        .ok()
        .and_then(|k| k.try_into().ok())
        .context("the private key of the WireGuard device")?;
    let peer = device
        .config
        .peers
        .first()
        .context("the WireGuard device has no peer")?;
    let peer_public: [u8; 32] = b64
        .decode(&peer.public_key)
        .ok()
        .and_then(|k| k.try_into().ok())
        .context("the key of the WireGuard peer")?;
    let endpoint = match endpoint {
        Some(e) => e,
        None => super::export::wireguard_socket_addr(&peer.endpoint)?,
    };
    Ok(Peer {
        keys: Keys {
            private,
            peer_public,
            reserved: device.reserved().unwrap_or([0; 3]),
            preshared: None,
        },
        endpoint,
        v4: device.config.interface.addresses.v4.parse().ok(),
        v6: device.config.interface.addresses.v6.parse().ok(),
    })
}

/// The handshake with `peer` over a UDP socket of the host.
pub async fn connect(peer: &Peer) -> Result<(WgTx, WgRx, WgKeepalive)> {
    use pod_warp::UdpWay as _;
    let datagrams = pod_warp::SystemUdp
        .datagrams(peer.endpoint)
        .await
        .warped()?;
    pod_warp::wg::connect(peer, datagrams).await.warped()
}

/// The session of `peer` over a UDP socket of the host.
pub async fn session(peer: &Peer) -> Result<super::MasqueSession> {
    let mut way = pod_warp::Way::new(crate::egress::egress(None), super::provider());
    way.udp = Some(std::sync::Arc::new(pod_warp::SystemUdp));
    super::MasqueSession::connect_wg(peer, &way).await.warped()
}

#[cfg(test)]
pub(crate) mod standin;
