use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn edge_target(port: u16) -> Target {
    Target {
        id: "edge-name",
        host: "127.0.0.1".into(),
        port,
        tls: Tls::Edge,
    }
}

type Server = (
    u16,
    Arc<dyn ServerCertVerifier>,
    tokio::sync::oneshot::Receiver<String>,
);

/// A TLS server on loopback for the name of the edge. `answer` is what it
/// sends after it reads the request. Gives its port, the verifier that
/// trusts it, and the request it read.
async fn tls_server(answer: &'static [u8]) -> Server {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let ca_key = rcgen::KeyPair::generate().unwrap();
    let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let ca = params.self_signed(&ca_key).unwrap();
    let issuer = rcgen::Issuer::new(params, ca_key);
    let leaf_key = rcgen::KeyPair::generate().unwrap();
    let leaf = rcgen::CertificateParams::new(vec![crate::cloudflare::h2_sni().to_string()])
        .unwrap()
        .signed_by(&leaf_key, &issuer)
        .unwrap();
    let config = rustls::ServerConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![leaf.der().clone()],
            rustls::pki_types::PrivateKeyDer::Pkcs8(leaf_key.serialize_der().into()),
        )
        .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let mut roots = rustls::RootCertStore::empty();
    roots.add(ca.der().clone()).unwrap();
    let verifier =
        rustls::client::WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider)
            .build()
            .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (send, got) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let Ok(mut tls) = acceptor.accept(tcp).await else {
            return;
        };
        let mut head = Vec::new();
        let mut chunk = [0u8; 512];
        while !head.ends_with(b"\r\n\r\n") {
            match tls.read(&mut chunk).await {
                Ok(n) if n > 0 => head.extend_from_slice(&chunk[..n]),
                _ => break,
            }
        }
        let _ = send.send(String::from_utf8_lossy(&head).into_owned());
        let _ = tls.write_all(answer).await;
        let _ = tls.flush().await;
        tokio::time::sleep(Duration::from_millis(500)).await;
    });
    (port, verifier, got)
}

const LIMIT: Duration = Duration::from_secs(2);

#[tokio::test]
async fn lanes_ws_upgrade_is_ok_on_101() {
    crate::test_host::need_loopback!();
    let (port, verifier, request) = tls_server(
        b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n",
    )
    .await;
    let got = ws_upgrade(&edge_target(port), LIMIT, verifier).await;
    assert_eq!(got.verdict, Verdict::Ok, "{}", got.reason);
    assert!(got.time.is_some());
    assert!(got.reason.contains("101"), "{}", got.reason);
    let request = request.await.unwrap();
    assert!(request.starts_with("GET / HTTP/1.1\r\n"), "{request}");
    assert!(request.contains("Upgrade: websocket\r\n"), "{request}");
    assert!(request.contains("Sec-WebSocket-Key: "), "{request}");
}

#[tokio::test]
async fn lanes_ws_upgrade_fails_on_another_status() {
    crate::test_host::need_loopback!();
    let (port, verifier, _) =
        tls_server(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n").await;
    let got = ws_upgrade(&edge_target(port), LIMIT, verifier).await;
    assert_eq!(got.verdict, Verdict::Refused, "{}", got.reason);
    assert!(got.reason.contains("403"), "{}", got.reason);
}

#[tokio::test]
async fn lanes_ws_upgrade_fails_on_a_tls_fault() {
    crate::test_host::need_loopback!();
    // The verifier of another server does not trust this one.
    let (port, _, _) = tls_server(b"HTTP/1.1 101 x\r\n\r\n").await;
    let (_, other, _) = tls_server(b"").await;
    let got = ws_upgrade(&edge_target(port), LIMIT, other).await;
    assert_eq!(got.verdict, Verdict::Refused, "{}", got.reason);
    assert!(got.reason.contains("TLS"), "{}", got.reason);
}

#[tokio::test]
async fn lanes_ws_upgrade_fails_on_a_closed_port() {
    crate::test_host::need_loopback!();
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let (_, verifier, _) = tls_server(b"").await;
    let got = ws_upgrade(&edge_target(port), LIMIT, verifier).await;
    assert_eq!(got.verdict, Verdict::Refused, "{}", got.reason);
}

#[tokio::test]
async fn lanes_ws_upgrade_is_unknown_when_the_peer_is_silent() {
    crate::test_host::need_loopback!();
    // A TLS server that never sends a status line.
    let (port, verifier, _) = tls_server(b"").await;
    let got = ws_upgrade(&edge_target(port), Duration::from_millis(100), verifier).await;
    assert_eq!(got.verdict, Verdict::Unknown, "{}", got.reason);
}

fn udp_peer(reply: bool) -> std::net::SocketAddr {
    let peer = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let addr = peer.local_addr().unwrap();
    std::thread::spawn(move || {
        let mut buf = [0u8; 1500];
        if let Ok((_, from)) = peer.recv_from(&mut buf) {
            if reply {
                // Any packet is an answer.
                let _ = peer.send_to(&[0x80, 0, 0, 0, 0, 0, 0], from);
            } else {
                std::thread::sleep(Duration::from_millis(600));
            }
        }
    });
    addr
}

const DEADLINE: Duration = Duration::from_millis(300);

#[test]
fn lanes_masque_udp_is_ok_on_any_answer() {
    crate::test_host::need_loopback!();
    let edge = Egress::Unknown("x".into());
    let got = udp_send(udp_peer(true), false, DEADLINE).lane(&edge);
    assert_eq!(got.verdict, Verdict::Ok, "{}", got.reason);
    assert!(got.time.is_some());
}

#[test]
fn lanes_masque_udp_fails_on_a_refusal() {
    crate::test_host::need_loopback!();
    let addr = std::net::UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let edge = Egress::Unknown("x".into());
    let got = udp_send(addr, false, DEADLINE).lane(&edge);
    assert_eq!(got.verdict, Verdict::Refused, "{}", got.reason);
}

#[test]
fn lanes_masque_udp_silence_is_a_fail_only_when_udp_is_open_elsewhere() {
    crate::test_host::need_loopback!();
    let open = Egress::Answered("a datagram".into());
    let got = udp_send(udp_peer(false), false, DEADLINE).lane(&open);
    assert_eq!(got.verdict, Verdict::Refused, "{}", got.reason);
    assert!(got.reason.contains("UDP 7844"), "{}", got.reason);
    let shut = Egress::Unknown("no answer".into());
    let got = udp_send(udp_peer(false), false, DEADLINE).lane(&shut);
    assert_eq!(got.verdict, Verdict::Unknown, "{}", got.reason);
}

#[test]
fn lanes_masque_udp_with_a_proxy_is_unknown_and_sends_nothing() {
    // The address is not a local one: a packet would not come back.
    let target: std::net::SocketAddr = "192.0.2.1:443".parse().unwrap();
    let got = udp_send(target, true, DEADLINE).lane(&Egress::Answered("x".into()));
    assert_eq!(got.verdict, Verdict::Unknown);
    assert!(
        got.reason.contains("no proxy carries UDP"),
        "{}",
        got.reason
    );
}
