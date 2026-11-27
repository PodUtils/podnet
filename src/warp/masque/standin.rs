//! Local HTTP/3 MASQUE stand-in server for integration tests (QUIC-1 to QUIC-5).
//!
//! Serves `CONNECT-IP`, `CONNECT-UDP`, and `WebTransport` over `pod-quic` (`noq`)
//! and `pod-warp::h3` with a self-signed certificate.

use std::collections::HashMap;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, Mutex, PoisonError};

use bytes::{Bytes, BytesMut};
use pod_quic::noq;
use pod_quic::noq::crypto::rustls::QuicServerConfig;
use pod_warp::capsule::{self, CapsuleDecoder, CAPSULE_DATA};
use pod_warp::h3::wire::{self, Frames, FRAME_DATA, FRAME_HEADERS, FRAME_SETTINGS};
use pod_warp::h3::{self, Shape};
use rustls::pki_types::PrivateKeyDer;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;

use super::WEBTRANSPORT_BIDI_STREAM;

const CHUNK: usize = 64 * 1024;
type DgramRoutes = Arc<Mutex<HashMap<u64, mpsc::Sender<Bytes>>>>;

/// Generated self-signed CA and leaf certificate material.
pub struct ServerCert {
    pub quic: Arc<QuicServerConfig>,
    pub ca_der: Vec<u8>,
    pub leaf_spki_der: Vec<u8>,
}

/// Generate a self-signed CA and leaf certificate valid for localhost, `127.0.0.1`,
/// `::1`, and the WARP MASQUE SNI.
pub fn generate_cert() -> ServerCert {
    let provider = crate::warp::provider();
    let ca_key = rcgen::KeyPair::generate().expect("CA key");
    let mut ca_params = rcgen::CertificateParams::new(Vec::<String>::new()).expect("CA params");
    ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    ca_params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "podnet masque test CA");
    let ca = ca_params.self_signed(&ca_key).expect("self-signed CA");
    let issuer = rcgen::Issuer::new(ca_params, ca_key);
    let leaf_key = rcgen::KeyPair::generate().expect("leaf key");
    let leaf = rcgen::CertificateParams::new(vec![
        "localhost".into(),
        "127.0.0.1".into(),
        "::1".into(),
        "consumer-masque.cloudflareclient.com".into(),
    ])
    .expect("leaf params")
    .signed_by(&leaf_key, &issuer)
    .expect("signed leaf");
    let leaf_spki_der =
        pod_warp::identity::spki_from_cert(leaf.der().as_ref()).expect("leaf SPKI DER");
    let mut tls = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("TLS versions")
        .with_no_client_auth()
        .with_single_cert(
            vec![leaf.der().clone(), ca.der().clone()],
            PrivateKeyDer::Pkcs8(leaf_key.serialize_der().into()),
        )
        .expect("server TLS");
    tls.alpn_protocols = vec![b"h3".to_vec()];
    let quic = Arc::new(QuicServerConfig::try_from(Arc::new(tls)).expect("QUIC server TLS"));
    ServerCert {
        quic,
        ca_der: ca.der().to_vec(),
        leaf_spki_der,
    }
}

/// Local HTTP/3 MASQUE stand-in listener.
pub struct StandinMasque {
    pub addr: SocketAddr,
    pub ca_der: Vec<u8>,
    pub leaf_spki_der: Vec<u8>,
    task: tokio::task::JoinHandle<()>,
}

impl StandinMasque {
    /// Bind and start a stand-in MASQUE server on `bind`.
    pub fn start(bind: SocketAddr, expected_bearer: Option<String>) -> Self {
        let cert = generate_cert();
        let mut server_cfg = pod_quic::endpoint::server_config(cert.quic.clone()).expect("cfg");
        let settings = pod_quic::Settings {
            datagram_buffer: Some(h3::DATAGRAM_BUFFER),
            ..pod_quic::Settings::default()
        };
        let mut transport = Shape::common().transport(&settings);
        transport.datagram_receive_buffer_size(Some(h3::DATAGRAM_BUFFER));
        transport.datagram_send_buffer_size(h3::DATAGRAM_BUFFER);
        server_cfg.transport_config(Arc::new(transport));
        let endpoint = pod_quic::endpoint::server(server_cfg, bind).expect("bind MASQUE server");
        let addr = endpoint.local_addr().expect("local addr");
        let task = tokio::spawn(async move {
            while let Some(incoming) = endpoint.accept().await {
                let bearer = expected_bearer.clone();
                tokio::spawn(async move {
                    if let Ok(conn) = incoming.await {
                        serve_connection(conn, bearer).await;
                    }
                });
            }
        });
        Self {
            addr,
            ca_der: cert.ca_der,
            leaf_spki_der: cert.leaf_spki_der,
            task,
        }
    }
}

impl Drop for StandinMasque {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn serve_connection(conn: noq::Connection, expected_bearer: Option<String>) {
    let Ok(mut control) = conn.open_uni().await else {
        return;
    };
    let mut settings_buf = Vec::new();
    for (id, value) in h3::SETTINGS {
        wire::put_varint(&mut settings_buf, *id);
        wire::put_varint(&mut settings_buf, *value);
    }
    let mut init = vec![0x00];
    init.extend_from_slice(&h3::frame(FRAME_SETTINGS, &settings_buf));
    if control.write_all(&init).await.is_err() {
        return;
    }
    let uni_conn = conn.clone();
    let uni_task = tokio::spawn(async move {
        while let Ok(mut uni) = uni_conn.accept_uni().await {
            tokio::spawn(async move { while let Ok(Some(_)) = uni.read_chunk(CHUNK).await {} });
        }
    });
    let routes: DgramRoutes = Arc::new(Mutex::new(HashMap::new()));
    let dgram_conn = conn.clone();
    let dgram_routes = routes.clone();
    let dgram_task = tokio::spawn(async move {
        while let Ok(dgram) = dgram_conn.read_datagram().await {
            let Some((quarter, used)) = wire::get_varint(&dgram) else {
                continue;
            };
            let tx = dgram_routes
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(&quarter)
                .cloned();
            if let Some(tx) = tx {
                let _ = tx.try_send(dgram.slice(used..));
            }
        }
    });
    while let Ok((send, recv)) = conn.accept_bi().await {
        let (c, r, b) = (conn.clone(), routes.clone(), expected_bearer.clone());
        tokio::spawn(async move { handle_bi(c, send, recv, r, b).await });
    }
    uni_task.abort();
    dgram_task.abort();
    drop(control);
}

async fn send_status(send: &mut noq::SendStream, section: &[u8], finish: bool) -> bool {
    let ok = send
        .write_all(&h3::frame(FRAME_HEADERS, section))
        .await
        .is_ok();
    if finish {
        let _ = send.finish();
    }
    ok
}

fn reg_route(routes: &DgramRoutes, quarter: u64) -> mpsc::Receiver<Bytes> {
    let (tx, rx) = mpsc::channel(256);
    routes
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(quarter, tx);
    rx
}

async fn handle_bi(
    conn: noq::Connection,
    mut send: noq::SendStream,
    mut recv: noq::RecvStream,
    routes: DgramRoutes,
    expected_bearer: Option<String>,
) {
    let mut initial = Vec::new();
    let first = loop {
        if let Some(v) = wire::get_varint(&initial) {
            break v;
        }
        match recv.read_chunk(CHUNK).await {
            Ok(Some(chunk)) => initial.extend_from_slice(&chunk),
            _ => return,
        }
    };
    if first.0 == WEBTRANSPORT_BIDI_STREAM {
        let mut rest = initial[first.1..].to_vec();
        let used = loop {
            if let Some((_, u)) = wire::get_varint(&rest) {
                break u;
            }
            match recv.read_chunk(CHUNK).await {
                Ok(Some(chunk)) => rest.extend_from_slice(&chunk),
                _ => return,
            }
        };
        if rest.len() > used && send.write_all(&rest[used..]).await.is_err() {
            return;
        }
        while let Ok(Some(chunk)) = recv.read_chunk(CHUNK).await {
            if send.write_all(&chunk).await.is_err() {
                break;
            }
        }
        let _ = send.finish();
        return;
    }

    let mut frames = Frames::default();
    frames.push(&initial);
    let fields = loop {
        if let Ok(Some((kind, payload))) = frames.next_frame() {
            if kind == FRAME_HEADERS {
                match wire::request_fields(&payload) {
                    Ok(f) => break f,
                    Err(_) => return,
                }
            }
            continue;
        }
        match recv.read_chunk(CHUNK).await {
            Ok(Some(chunk)) => frames.push(&chunk),
            _ => return,
        }
    };
    let get = |name: &str| {
        fields
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    };
    if let Some(required) = expected_bearer.as_deref() {
        if get("authorization") != Some(format!("Bearer {required}").as_str()) {
            send_status(&mut send, &[0x00, 0x00, 0xc0 | 63, 0x05], true).await;
            return;
        }
    }
    let protocol = get(":protocol").unwrap_or("");
    let path = get(":path").unwrap_or("/");
    let quarter = u64::from(noq::VarInt::from(send.id())) / 4;
    let status_200 = [0x00, 0x00, 0xc0 | 25];
    let status_400 = [0x00, 0x00, 0xc0 | 63, 0x04];

    match protocol {
        "connect-ip" | "cf-connect-ip" => {
            let rx = reg_route(&routes, quarter);
            if send_status(&mut send, &status_200, false).await {
                serve_connect_ip(
                    conn,
                    send,
                    recv,
                    frames,
                    rx,
                    quarter,
                    protocol != "cf-connect-ip",
                )
                .await;
            }
        }
        "connect-udp" => {
            let Some(target) = parse_udp_target(path) else {
                send_status(&mut send, &status_400, true).await;
                return;
            };
            let rx = reg_route(&routes, quarter);
            if send_status(&mut send, &status_200, false).await {
                serve_connect_udp(conn, recv, frames, rx, quarter, target).await;
            }
        }
        "webtransport" => {
            let rx = reg_route(&routes, quarter);
            if send_status(&mut send, &status_200, false).await {
                serve_webtransport(conn, send, recv, frames, rx, quarter).await;
            }
        }
        _ => {
            send_status(&mut send, &status_400, true).await;
            return;
        }
    }
    routes
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .remove(&quarter);
}

async fn serve_connect_ip(
    conn: noq::Connection,
    mut send: noq::SendStream,
    mut recv: noq::RecvStream,
    mut frames: Frames,
    mut dgram_rx: mpsc::Receiver<Bytes>,
    quarter: u64,
    capsule_context: bool,
) {
    let mut capsules = CapsuleDecoder::new();
    let mut prefix = Vec::new();
    wire::put_varint(&mut prefix, quarter);
    wire::put_varint(&mut prefix, 0);
    loop {
        while let Ok(Some((kind, payload))) = frames.next_frame() {
            if kind == FRAME_DATA {
                capsules.push(&payload);
                while let Some((CAPSULE_DATA, value)) = capsules.next_capsule() {
                    let mut out = BytesMut::new();
                    capsule::put_capsule(&mut out, CAPSULE_DATA, &value);
                    if send.write_all(&h3::frame(FRAME_DATA, &out)).await.is_err() {
                        return;
                    }
                }
            }
        }
        tokio::select! {
            dgram = dgram_rx.recv() => {
                let Some(bytes) = dgram else { break };
                let Some((0, used)) = wire::get_varint(&bytes) else { continue };
                let packet = &bytes[used..];
                let mut reply = BytesMut::with_capacity(prefix.len() + packet.len());
                reply.extend_from_slice(&prefix);
                reply.extend_from_slice(packet);
                if conn.send_datagram_wait(reply.freeze()).await.is_err() {
                    let mut value = BytesMut::new();
                    if capsule_context {
                        wire::put_varint(&mut value, 0);
                    }
                    value.extend_from_slice(packet);
                    let mut out = BytesMut::new();
                    capsule::put_capsule(&mut out, CAPSULE_DATA, &value);
                    if send.write_all(&h3::frame(FRAME_DATA, &out)).await.is_err() {
                        break;
                    }
                }
            }
            chunk = recv.read_chunk(CHUNK) => match chunk {
                Ok(Some(chunk)) => frames.push(&chunk),
                _ => break,
            }
        }
    }
}

async fn serve_connect_udp(
    conn: noq::Connection,
    mut recv: noq::RecvStream,
    mut frames: Frames,
    mut dgram_rx: mpsc::Receiver<Bytes>,
    quarter: u64,
    target: SocketAddr,
) {
    let bind: SocketAddr = match target {
        SocketAddr::V4(_) => (Ipv4Addr::UNSPECIFIED, 0).into(),
        SocketAddr::V6(_) => (Ipv6Addr::UNSPECIFIED, 0).into(),
    };
    // egress-exempt: local loopback stand-in forwarding CONNECT-UDP in tests.
    let Ok(sock) = UdpSocket::bind(bind).await else {
        return;
    };
    if sock.connect(target).await.is_err() {
        return;
    }
    let mut capsules = CapsuleDecoder::new();
    let mut prefix = Vec::new();
    wire::put_varint(&mut prefix, quarter);
    wire::put_varint(&mut prefix, 0);
    let mut buf = vec![0u8; 65535];
    loop {
        while let Ok(Some((kind, payload))) = frames.next_frame() {
            if kind == FRAME_DATA {
                capsules.push(&payload);
                while let Some((CAPSULE_DATA, value)) = capsules.next_capsule() {
                    if let Some((0, used)) = capsule::get_varint(&value) {
                        let _ = sock.send(&value[used..]).await;
                    }
                }
            }
        }
        tokio::select! {
            dgram = dgram_rx.recv() => {
                let Some(bytes) = dgram else { break };
                if let Some((0, used)) = wire::get_varint(&bytes) {
                    let _ = sock.send(&bytes[used..]).await;
                }
            }
            read = sock.recv(&mut buf) => {
                let Ok(n) = read else { break };
                let mut reply = BytesMut::with_capacity(prefix.len() + n);
                reply.extend_from_slice(&prefix);
                reply.extend_from_slice(&buf[..n]);
                let _ = conn.send_datagram_wait(reply.freeze()).await;
            }
            chunk = recv.read_chunk(CHUNK) => match chunk {
                Ok(Some(chunk)) => frames.push(&chunk),
                _ => break,
            }
        }
    }
}

async fn serve_webtransport(
    conn: noq::Connection,
    mut send: noq::SendStream,
    mut recv: noq::RecvStream,
    mut frames: Frames,
    mut dgram_rx: mpsc::Receiver<Bytes>,
    quarter: u64,
) {
    let mut prefix = Vec::new();
    wire::put_varint(&mut prefix, quarter);
    loop {
        while let Ok(Some((kind, payload))) = frames.next_frame() {
            if kind == FRAME_DATA
                && send
                    .write_all(&h3::frame(FRAME_DATA, &payload))
                    .await
                    .is_err()
            {
                return;
            }
        }
        tokio::select! {
            dgram = dgram_rx.recv() => {
                let Some(bytes) = dgram else { break };
                let mut reply = BytesMut::with_capacity(prefix.len() + bytes.len());
                reply.extend_from_slice(&prefix);
                reply.extend_from_slice(&bytes);
                let _ = conn.send_datagram_wait(reply.freeze()).await;
            }
            chunk = recv.read_chunk(CHUNK) => match chunk {
                Ok(Some(chunk)) => frames.push(&chunk),
                _ => break,
            }
        }
    }
}

fn parse_udp_target(path: &str) -> Option<SocketAddr> {
    let tail = path
        .strip_prefix("/.well-known/masque/udp/")?
        .trim_end_matches('/');
    let (host, port_str) = tail.rsplit_once('/')?;
    Some(SocketAddr::new(host.parse().ok()?, port_str.parse().ok()?))
}
