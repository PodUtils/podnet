//! Provider-agnostic HTTP/3 MASQUE client supporting CONNECT-IP, CONNECT-UDP,
//! and WebTransport on one QUIC connection (QUIC-1, QUIC-2, QUIC-10).
//!
//! Reuses `pod-quic` (`noq`) and `pod-warp::h3` with no second QUIC stack.

use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use bytes::Bytes;
use pod_quic::noq;
use pod_warp::capsule::CapsuleDecoder;
use pod_warp::h3::{self, H3Conn};
use pod_warp::UdpWay as _;
use rustls::pki_types::CertificateDer;
use tokio::sync::mpsc;

use super::enrol::{EnrolledEndpoint, MasqueEnrolment, MasqueTlsAuth};
use super::error::MasqueError;
use super::family::{select_endpoints, AttemptTracker, IpPolicy, TrackedUdp};
use super::session::{
    quarter_stream_id, route_datagrams, IpSession, SharedConn, UdpSession, WebTransportSession,
};

/// Default handshake timeout for MASQUE over QUIC.
pub const DEFAULT_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// Bounded per-session queue for routed HTTP/3 datagrams.
const DATAGRAM_CHANNEL: usize = 256;

/// Provider-agnostic MASQUE client over one QUIC/HTTP/3 stack.
pub struct MasqueClient {
    enrolment: Arc<dyn MasqueEnrolment>,
    policy: IpPolicy,
    tracker: AttemptTracker,
    timeout: Duration,
}

impl MasqueClient {
    /// Build a MASQUE client driven by `enrolment`.
    pub fn new(enrolment: Arc<dyn MasqueEnrolment>) -> Self {
        Self {
            enrolment,
            policy: IpPolicy::Auto,
            tracker: AttemptTracker::new(),
            timeout: DEFAULT_HANDSHAKE_TIMEOUT,
        }
    }

    /// Set the address-family policy (`DualStack`, `Ipv6Only`, or `Auto`).
    pub fn with_ip_policy(mut self, policy: IpPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// Attach a custom [`AttemptTracker`] to observe bind and connect attempts.
    pub fn with_tracker(mut self, tracker: AttemptTracker) -> Self {
        self.tracker = tracker;
        self
    }

    /// Set the QUIC handshake timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Return the attempt tracker of this client.
    pub fn tracker(&self) -> &AttemptTracker {
        &self.tracker
    }

    /// Enrol with the provider and open one shared HTTP/3 QUIC connection.
    pub async fn connect(&self) -> Result<MasqueConnection, MasqueError> {
        let enrolled = self.enrolment.enrol().await?;
        let candidates = resolve_candidates(&enrolled, self.policy, &self.tracker).await?;
        let allowed = select_endpoints(&candidates, self.policy, &self.tracker)?;
        let tls = build_tls_config(&enrolled.tls)?;
        let udp = TrackedUdp::new(self.policy, self.tracker.clone());

        let mut last_err = None;
        for door in allowed {
            let datagrams = match udp.datagrams(door).await {
                Ok(d) => d,
                Err(err) => {
                    last_err = Some(MasqueError::from_transport_failure(&err));
                    continue;
                }
            };
            match h3::open(door, datagrams, tls.clone(), &enrolled.sni, self.timeout).await {
                Ok(h3_conn) => {
                    return Ok(MasqueConnection::new(h3_conn, enrolled, door));
                }
                Err(err) => {
                    last_err = Some(MasqueError::from_transport_failure(&err));
                }
            }
        }
        Err(last_err.unwrap_or_else(|| MasqueError::transport("no MASQUE endpoint accepted QUIC")))
    }

    /// Open a `CONNECT-IP` session (RFC 9484).
    pub async fn connect_ip(&self) -> Result<IpSession, MasqueError> {
        let conn = self.connect().await?;
        conn.open_connect_ip().await
    }

    /// Open a `CONNECT-UDP` session (RFC 9298) to `target_host:target_port`.
    pub async fn connect_udp(
        &self,
        target_host: &str,
        target_port: u16,
    ) -> Result<UdpSession, MasqueError> {
        let conn = self.connect().await?;
        conn.open_connect_udp(target_host, target_port).await
    }

    /// Open a `WebTransport` session over HTTP/3 at `path`.
    pub async fn connect_webtransport(
        &self,
        path: &str,
    ) -> Result<WebTransportSession, MasqueError> {
        let conn = self.connect().await?;
        conn.open_webtransport(path).await
    }
}

/// One live HTTP/3 QUIC connection that multiplexes `CONNECT-IP`, `CONNECT-UDP`,
/// and `WebTransport` sessions.
#[derive(Clone)]
pub struct MasqueConnection {
    shared: Arc<SharedConn>,
    enrolled: Arc<EnrolledEndpoint>,
    peer_addr: SocketAddr,
}

impl MasqueConnection {
    fn new(h3_conn: H3Conn, enrolled: EnrolledEndpoint, peer_addr: SocketAddr) -> Self {
        let H3Conn {
            conn,
            control,
            endpoint,
        } = h3_conn;
        let routes: Arc<Mutex<HashMap<u64, mpsc::Sender<Bytes>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let reader_conn = conn.clone();
        let reader_routes = routes.clone();
        let router_task = tokio::spawn(async move {
            route_datagrams(reader_conn, reader_routes).await;
        });
        Self {
            shared: Arc::new(SharedConn {
                conn,
                _control: control,
                _endpoint: endpoint,
                routes,
                router_task,
            }),
            enrolled: Arc::new(enrolled),
            peer_addr,
        }
    }

    /// Return the provider name that produced this connection.
    pub fn provider(&self) -> &str {
        &self.enrolled.provider
    }

    /// Return the connected remote QUIC address.
    pub fn peer_addr(&self) -> SocketAddr {
        self.peer_addr
    }

    /// Return the underlying `noq::Connection`.
    pub fn quic_connection(&self) -> noq::Connection {
        self.shared.conn.clone()
    }

    /// Open a `CONNECT-IP` session on this HTTP/3 connection (RFC 9484).
    pub async fn open_connect_ip(&self) -> Result<IpSession, MasqueError> {
        let protocol = self.enrolled.ip_protocol.as_str();
        let headers = encode_connect_headers(
            &self.enrolled.authority,
            "/",
            protocol,
            &self.enrolled.extra_headers,
            protocol == "cf-connect-ip",
        );
        let (send, recv, frames, status) = h3::request(&self.shared.conn, &headers)
            .await
            .map_err(|e| MasqueError::from_transport_failure(&e))?;
        if status != 200 {
            return Err(MasqueError::transport(format!(
                "CONNECT-IP ({protocol}) refused with HTTP {status}"
            )));
        }
        let quarter = quarter_stream_id(send.id());
        let dgram_rx = self.register_route(quarter);
        Ok(IpSession {
            shared: self.shared.clone(),
            send,
            recv,
            frames,
            dgram_rx,
            capsules: CapsuleDecoder::new(),
            ready: VecDeque::new(),
            quarter,
            capsule_context: protocol != "cf-connect-ip",
        })
    }

    /// Open a `CONNECT-UDP` session on this HTTP/3 connection (RFC 9298).
    pub async fn open_connect_udp(
        &self,
        target_host: &str,
        target_port: u16,
    ) -> Result<UdpSession, MasqueError> {
        let path = h3::udp_path(target_host, target_port);
        let headers = encode_connect_headers(
            &self.enrolled.authority,
            &path,
            "connect-udp",
            &self.enrolled.extra_headers,
            false,
        );
        let (send, recv, frames, status) = h3::request(&self.shared.conn, &headers)
            .await
            .map_err(|e| MasqueError::from_transport_failure(&e))?;
        if status != 200 {
            return Err(MasqueError::transport(format!(
                "CONNECT-UDP to {target_host}:{target_port} refused with HTTP {status}"
            )));
        }
        let quarter = quarter_stream_id(send.id());
        let dgram_rx = self.register_route(quarter);
        Ok(UdpSession {
            shared: self.shared.clone(),
            send,
            recv,
            frames,
            dgram_rx,
            capsules: CapsuleDecoder::new(),
            ready: VecDeque::new(),
            quarter,
        })
    }

    /// Open a `WebTransport` session on this HTTP/3 connection.
    pub async fn open_webtransport(&self, path: &str) -> Result<WebTransportSession, MasqueError> {
        let path = if path.is_empty() { "/" } else { path };
        let headers = encode_connect_headers(
            &self.enrolled.authority,
            path,
            "webtransport",
            &self.enrolled.extra_headers,
            false,
        );
        let (send, recv, frames, status) = h3::request(&self.shared.conn, &headers)
            .await
            .map_err(|e| MasqueError::from_transport_failure(&e))?;
        if status != 200 {
            return Err(MasqueError::transport(format!(
                "WebTransport ({path}) refused with HTTP {status}"
            )));
        }
        let session_id = u64::from(noq::VarInt::from(send.id()));
        let quarter = session_id / 4;
        let dgram_rx = self.register_route(quarter);
        Ok(WebTransportSession {
            shared: self.shared.clone(),
            send,
            recv,
            frames,
            dgram_rx,
            session_id,
            quarter,
        })
    }

    fn register_route(&self, quarter: u64) -> mpsc::Receiver<Bytes> {
        let (tx, rx) = mpsc::channel(DATAGRAM_CHANNEL);
        let mut map = self
            .shared
            .routes
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        map.insert(quarter, tx);
        rx
    }
}

async fn resolve_candidates(
    enrolled: &EnrolledEndpoint,
    policy: IpPolicy,
    tracker: &AttemptTracker,
) -> Result<Vec<SocketAddr>, MasqueError> {
    if !enrolled.endpoints.is_empty() {
        return Ok(enrolled.endpoints.clone());
    }
    let Some(ep) = enrolled.unresolved else {
        return Err(MasqueError::enrolment(
            "enrolment returned no MASQUE endpoint",
        ));
    };
    let resolved: Vec<SocketAddr> = tokio::net::lookup_host((ep.host(), ep.port()))
        .await
        .map_err(|e| MasqueError::transport(format!("resolve {}: {e}", ep.authority())))?
        .collect();
    select_endpoints(&resolved, policy, tracker)
}

fn build_tls_config(auth: &MasqueTlsAuth) -> Result<Arc<rustls::ClientConfig>, MasqueError> {
    let provider = crate::warp::provider();
    match auth {
        MasqueTlsAuth::PinnedKey {
            identity,
            peer_spki_der,
        } => pod_warp::tls::client_config(provider, identity, peer_spki_der.clone(), h3::ALPN)
            .map_err(|e| MasqueError::enrolment(format!("TLS pin configuration: {}", e.chain()))),
        MasqueTlsAuth::CustomCa { ca_der } => {
            let mut roots = rustls::RootCertStore::empty();
            for der in ca_der {
                roots.add(CertificateDer::from(der.clone())).map_err(|e| {
                    MasqueError::enrolment(format!("invalid CA certificate DER: {e}"))
                })?;
            }
            pod_quic::connect::tls(provider, Arc::new(roots), &[h3::ALPN])
                .map_err(|e| MasqueError::enrolment(format!("TLS CA configuration: {}", e.chain())))
        }
        MasqueTlsAuth::SystemRoots => {
            let cfg = crate::trust::context()
                .client_config_with(&[], &[h3::ALPN])
                .map_err(|e| MasqueError::enrolment(format!("TLS system roots: {e}")))?;
            Ok(Arc::new(cfg))
        }
    }
}

/// Encode an extended CONNECT QPACK field section (static table, no Huffman)
/// with custom `:path`, `:protocol`, and extra headers (such as `authorization`).
pub fn encode_connect_headers(
    authority: &str,
    path: &str,
    protocol: &str,
    extra_headers: &[(String, String)],
    pq_hint: bool,
) -> Vec<u8> {
    let mut out = vec![0x00, 0x00, 0xc0 | 15, 0xc0 | 23];
    if path == "/" {
        out.push(0xc0 | 1);
    } else {
        put_prefixed(&mut out, 0x50, 4, 1);
        put_string(&mut out, 0x00, 7, path);
    }
    put_prefixed(&mut out, 0x50, 4, 0);
    put_string(&mut out, 0x00, 7, authority);
    for (name, value) in [(":protocol", protocol), ("capsule-protocol", "?1")] {
        put_string(&mut out, 0x20, 3, name);
        put_string(&mut out, 0x00, 7, value);
    }
    if pq_hint {
        put_string(&mut out, 0x20, 3, "pq-enabled");
        put_string(&mut out, 0x00, 7, "false");
    }
    for (name, value) in extra_headers {
        put_string(&mut out, 0x20, 3, &name.to_ascii_lowercase());
        put_string(&mut out, 0x00, 7, value);
    }
    out
}

fn put_prefixed(buf: &mut Vec<u8>, flags: u8, bits: u8, mut value: u64) {
    let max = (1u64 << bits) - 1;
    if value < max {
        buf.push(flags | value as u8);
        return;
    }
    buf.push(flags | max as u8);
    value -= max;
    while value >= 128 {
        buf.push((value % 128) as u8 | 0x80);
        value /= 128;
    }
    buf.push(value as u8);
}

fn put_string(buf: &mut Vec<u8>, flags: u8, bits: u8, text: &str) {
    put_prefixed(buf, flags, bits, text.len() as u64);
    buf.extend_from_slice(text.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use pod_warp::h3::wire;

    #[test]
    fn connect_headers_encode_all_three_modes_and_extra_headers() {
        let extras = vec![("Authorization".into(), "Bearer tok-1".into())];
        let ip = encode_connect_headers("masque.test:443", "/", "connect-ip", &extras, false);
        let fields = wire::request_fields(&ip).unwrap();
        let get = |k: &str| fields.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        assert_eq!(get(":method"), Some("CONNECT"));
        assert_eq!(get(":scheme"), Some("https"));
        assert_eq!(get(":path"), Some("/"));
        assert_eq!(get(":authority"), Some("masque.test:443"));
        assert_eq!(get(":protocol"), Some("connect-ip"));
        assert_eq!(get("capsule-protocol"), Some("?1"));
        assert_eq!(get("authorization"), Some("Bearer tok-1"));

        let wt = encode_connect_headers("masque.test:443", "/wt", "webtransport", &extras, false);
        let wt_fields = wire::request_fields(&wt).unwrap();
        let wt_get = |k: &str| {
            wt_fields
                .iter()
                .find(|(n, _)| n == k)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(wt_get(":protocol"), Some("webtransport"));
        assert_eq!(wt_get(":path"), Some("/wt"));
        assert_eq!(wt_get("authorization"), Some("Bearer tok-1"));
    }
}
