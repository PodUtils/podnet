//! A QUIC socket whose datagrams travel through the WARP session.
//!
//! The Cloudflare edge's preferred transport is QUIC on UDP 7844. WARP carries
//! UDP, so the only missing piece is a QUIC socket backed by the userspace
//! stack instead of a kernel socket: the datagrams of the session
//! (`pod_core::Datagrams` of `pod-warp`'s `WarpNet`).
//!
//! # Why not a kernel socket
//!
//! A sealed host can refuse `bind(2)` on `AF_INET` and `connect(2)` to every
//! destination, including loopback. The userspace stack already owns a UDP
//! socket inside the tunnel's address space, so the adapter moves datagrams
//! between noq and that socket and the kernel is never asked.

use std::sync::Arc;

use anyhow::Result;

use super::ip::WarpStack;

/// Build a QUIC endpoint whose only socket is the WARP session.
///
/// The endpoint dials the real edge address. Its datagrams leave the host on
/// TCP 443 inside the MASQUE session and arrive at the edge on 7844, so this
/// host never opens a connection to 7844.
pub async fn endpoint(stack: &WarpStack) -> Result<noq::Endpoint> {
    let datagrams = one_family_socket(stack).await?;
    let endpoint = pod_quic::over(datagrams, None)
        .map_err(|err| anyhow::anyhow!("building the QUIC endpoint: {err}"))?;
    endpoint.set_default_client_config(client_config()?);
    Ok(endpoint)
}

/// A UDP socket of the stack on one address of the device: the IPv4 address,
/// else the IPv6 address.
///
/// A fix (PodUtils/podnet#523): the datagrams of `WarpNet` bind `[::]:port`
/// (`bind_udp_any`). noq sends an IPv4 destination on an IPv6 socket as an
/// IPv4-mapped IPv6 address, which the stack does not route, so the QUIC
/// handshake with the edge timed out. A socket on the IPv4 address of the
/// device sends IPv4, as the socket of podnet did before the take of pod-warp.
async fn one_family_socket(stack: &WarpStack) -> Result<Arc<dyn pod_core::Datagrams>> {
    use anyhow::Context as _;
    let addresses = stack.addresses();
    let ip = addresses
        .iter()
        .find(|ip| ip.is_ipv4())
        .or_else(|| addresses.first())
        .copied()
        .context("the WARP device has no address")?;
    let mut last = None;
    for step in 0..64u16 {
        let port = EPHEMERAL_BASE + (std::process::id() as u16).wrapping_add(step * 97) % 16384;
        match stack
            .stack()
            .bind_udp(std::net::SocketAddr::new(ip, port))
            .await
        {
            Ok(socket) => return Ok(Arc::new(socket.into_datagrams())),
            Err(err) if err.kind() == std::io::ErrorKind::AddrInUse => last = Some(err),
            Err(err) => return Err(err).context("binding a UDP socket inside the WARP session"),
        }
    }
    Err(last
        .map(anyhow::Error::from)
        .unwrap_or_else(|| anyhow::anyhow!("no free port")))
    .context("binding a UDP socket inside the WARP session")
}

/// The first port of the ephemeral range (RFC 6335).
const EPHEMERAL_BASE: u16 = 49152;

/// The TLS and transport configuration the edge expects.
fn client_config() -> Result<noq::ClientConfig> {
    use crate::cloudflare::{edge_roots_pem, quic_alpn};
    let tls = crate::trust::context().client_config_with(&[edge_roots_pem()], &[quic_alpn()])?;

    let crypto = noq::crypto::rustls::QuicClientConfig::try_from(tls)
        .map_err(|err| anyhow::anyhow!("rustls to noq: {err}"))?;
    let mut config = noq::ClientConfig::new(Arc::new(crypto));
    // The tunnel carries packets of TUNNEL_MTU bytes. 1232 fits an IPv6 and
    // UDP header in that; MTU discovery stops at the IPv4 size.
    let ip_udp_v4 = 28;
    let mut transport = crate::cloudflare::quic::transport_config(1232);
    let mut discovery = noq::MtuDiscoveryConfig::default();
    discovery.upper_bound((super::ip::TUNNEL_MTU - ip_udp_v4) as u16);
    transport.mtu_discovery_config(Some(discovery));
    config.transport_config(Arc::new(transport));
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The transport and the TLS of the edge build: noq takes the TLS of
    /// podnet's trust, as the endpoint gives it.
    #[test]
    fn the_client_config_of_the_warp_socket_builds() {
        crate::init_crypto();
        client_config().expect("a client configuration");
    }
}
