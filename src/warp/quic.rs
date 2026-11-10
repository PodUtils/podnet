//! A QUIC socket whose datagrams travel through the WARP session.
//!
//! The Cloudflare edge's preferred transport is QUIC on UDP 7844. WARP carries
//! UDP, so the only missing piece is a QUIC socket backed by the userspace
//! stack instead of a kernel socket. pod-quic has that adapter
//! (`pod_quic::queued::QueuedDatagrams`: a send queue with a writer task, a
//! receive queue with a reader task, a waker for each sender of noq); this
//! module gives it the UDP socket of the stack.
//!
//! # Why not a kernel socket
//!
//! A sealed host can refuse `bind(2)` on `AF_INET` and `connect(2)` to every
//! destination, including loopback. The userspace stack already owns a UDP
//! socket inside the tunnel's address space, so the adapter moves datagrams
//! between noq and that socket and the kernel is never asked.

use std::io;
use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context as _, Result};
use pod_core::BoxFuture;
use pod_quic::queued::{Link, QueuedDatagrams};

use super::ip::{UdpConn, WarpStack};

/// The UDP socket of the userspace stack as a link of pod-quic.
struct StackLink(UdpConn);

impl Link for StackLink {
    fn send_to(&self, datagram: Vec<u8>, to: SocketAddr) -> BoxFuture<'_, io::Result<()>> {
        Box::pin(async move {
            self.0
                .send_to(datagram, to)
                .await
                .map(drop)
                .map_err(io::Error::other)
        })
    }

    fn recv_from(&self, max: usize) -> BoxFuture<'_, io::Result<(Vec<u8>, SocketAddr)>> {
        Box::pin(async move { self.0.recv_from(max).await.map_err(io::Error::other) })
    }

    fn local_addr(&self) -> SocketAddr {
        self.0.local_addr()
    }
}

/// Build a QUIC endpoint whose only socket is the WARP session.
///
/// The endpoint dials the real edge address. Its datagrams leave the host on
/// TCP 443 inside the MASQUE session and arrive at the edge on 7844, so this
/// host never opens a connection to 7844.
pub async fn endpoint(stack: &WarpStack) -> Result<noq::Endpoint> {
    let udp = stack
        .udp_bind()
        .await
        .context("binding a UDP socket inside the WARP session")?;
    let datagrams = QueuedDatagrams::new(Arc::new(StackLink(udp)));
    let endpoint = pod_quic::over(Arc::new(datagrams), None)
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
