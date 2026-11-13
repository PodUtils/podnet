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
    let datagrams = super::one_family::socket(stack, false).await?;
    let endpoint = pod_quic::over(datagrams, None)
        .map_err(|err| anyhow::anyhow!("building the QUIC endpoint: {err}"))?;
    endpoint.set_default_client_config(client_config()?);
    Ok(endpoint)
}

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
