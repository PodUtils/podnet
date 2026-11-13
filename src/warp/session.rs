//! The MASQUE session: `pod-warp`'s CONNECT-IP over HTTP/2 or HTTP/3, and
//! podnet's measure of one front door (`warp scan`, issue 139).

use std::time::Duration;

use anyhow::{Context as _, Result};

use super::Warped as _;
use crate::egress::Proxy;

pub use pod_warp::session::*;
pub use pod_warp::tls::PIN_MISMATCH;

/// The times of one front door (issue 139): the TCP connect and the TLS
/// handshake with the pinned key and the client certificate.
#[derive(Clone, Copy, Debug)]
pub struct DoorTimes {
    pub tcp: Duration,
    pub tls: Duration,
}

impl DoorTimes {
    pub fn total(&self) -> Duration {
        self.tcp + self.tls
    }
}

/// Measure one front door: TCP (through the proxy of the egress, as a
/// session), then the TLS handshake. The key of the front door must be the
/// pinned key. No CONNECT-IP request is sent.
pub async fn probe_door(
    endpoint: std::net::SocketAddr,
    identity: &super::Identity,
    pinned_spki: Vec<u8>,
    proxy: Option<&Proxy>,
    sni: Option<&str>,
    timeout: Duration,
) -> Result<DoorTimes> {
    let start = std::time::Instant::now();
    let tcp = crate::egress::dial(&endpoint.ip().to_string(), endpoint.port(), proxy, timeout)
        .await
        .with_context(|| format!("TCP to {endpoint}"))?;
    let tcp = pod_tls::fragment::wrap(Box::new(tcp), super::live::split());
    let tcp_time = start.elapsed();
    let config = pod_warp::tls::client_config(
        super::provider(),
        identity,
        pinned_spki,
        pod_warp::tls::ALPN_H2,
    )
    .warped()?;
    let name = sni.unwrap_or(pod_warp::service::masque_sni()).to_string();
    let server_name =
        rustls::pki_types::ServerName::try_from(name).context("invalid MASQUE SNI")?;
    let rest = timeout
        .saturating_sub(tcp_time)
        .max(Duration::from_millis(100));
    let start = std::time::Instant::now();
    let tls = tokio::time::timeout(
        rest,
        tokio_rustls::TlsConnector::from(config).connect(server_name, tcp),
    )
    .await
    .map_err(|_| anyhow::anyhow!("no TLS handshake in {} ms", rest.as_millis()))?
    .map_err(|e| anyhow::anyhow!("TLS: {}", crate::hop::tls_text(&e)))?;
    let tls_time = start.elapsed();
    drop(tls);
    Ok(DoorTimes {
        tcp: tcp_time,
        tls: tls_time,
    })
}
