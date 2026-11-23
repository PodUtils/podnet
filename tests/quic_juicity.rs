//! The Juicity gate of PodUtils/podnet#486 (QUIC-10): a Juicity client of
//! podnet completes a UDP session and a TCP session against a server built
//! from the reference wire.
//!
//! The server is in this file and follows the reference implementation of
//! `juicity/juicity` v0.5.0: the version and the command on a
//! unidirectional stream, the token of the TLS exporter, the address type
//! (1 IPv4, 4 IPv6, 3 a name), the address, the port, the 2-byte length of
//! each UDP datagram, and an echo. It is a stand-in, not the reference
//! server. The test proves the layout of the client, the token, both
//! networks and the bytes in each direction; the test
//! `quic_juicity_reference` runs the same paths against the reference
//! binary.

mod test_host;

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use pod_quic::noq;
use pod_quic::noq::crypto::rustls::QuicServerConfig;
use podnet::roads::juicity::{self, Link};

const UUID: &str = "123e4567-e89b-12d3-a456-426614174000";
const PASSWORD: &str = "juicity-password";
const ALPN: &[u8] = b"h3";

fn crypto() -> Arc<QuicServerConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let key = rcgen::KeyPair::generate().unwrap();
    let params = rcgen::CertificateParams::new(vec!["localhost".to_string()]).unwrap();
    let cert = params.self_signed(&key).unwrap();
    let mut tls = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![cert.der().clone()],
            rustls::pki_types::PrivateKeyDer::Pkcs8(key.serialize_der().into()),
        )
        .unwrap();
    tls.alpn_protocols = vec![ALPN.to_vec()];
    Arc::new(QuicServerConfig::try_from(Arc::new(tls)).unwrap())
}

/// The server: authentication, then one task for each stream of the
/// connection, which echoes what it carries.
async fn serve(conn: noq::Connection) {
    let Ok(mut auth) = conn.accept_uni().await else {
        return;
    };
    let mut bytes = vec![0u8; 2 + 16 + 32];
    if auth.read_exact(&mut bytes).await.is_err() {
        return;
    }
    let uuid = uuid::Uuid::parse_str(UUID).unwrap();
    let mut token = [0u8; 32];
    if conn
        .export_keying_material(&mut token, uuid.as_bytes(), PASSWORD.as_bytes())
        .is_err()
    {
        return;
    }
    if bytes[..2] != [0, 0] || bytes[2..18] != *uuid.as_bytes() || bytes[18..] != token {
        return;
    }
    let _auth = auth;
    while let Ok((send, recv)) = conn.accept_bi().await {
        tokio::spawn(handle(send, recv));
    }
}

/// One stream: the network byte tells a UDP flow from a TCP stream.
async fn handle(mut send: noq::SendStream, mut recv: noq::RecvStream) {
    let mut network = [0u8; 1];
    if recv.read_exact(&mut network).await.is_err() {
        return;
    }
    match network[0] {
        juicity::UDP => {
            if read_address(&mut recv).await.is_none() {
                return;
            }
            loop {
                let Some(header) = read_address(&mut recv).await else {
                    return;
                };
                let mut len = [0u8; 2];
                if recv.read_exact(&mut len).await.is_err() {
                    return;
                }
                let mut payload = vec![0u8; usize::from(u16::from_be_bytes(len))];
                if recv.read_exact(&mut payload).await.is_err() {
                    return;
                }
                let mut out = header.clone();
                out.extend_from_slice(&len);
                out.extend_from_slice(&payload);
                if send.write_all(&out).await.is_err() {
                    return;
                }
            }
        }
        juicity::TCP => {
            if read_address(&mut recv).await.is_none() {
                return;
            }
            let _ = tokio::io::copy(&mut recv, &mut send).await;
        }
        _ => {}
    }
}

/// One address header: the type, the address, the port and the bytes of
/// the header. A UDP datagram of the reference wire has no network byte.
async fn read_address(recv: &mut noq::RecvStream) -> Option<Vec<u8>> {
    let mut kind = [0u8; 1];
    recv.read_exact(&mut kind).await.ok()?;
    let mut rest = Vec::new();
    let from = match kind[0] {
        juicity::IPV4 => {
            rest.resize(4 + 2, 0);
            0
        }
        juicity::IPV6 => {
            rest.resize(16 + 2, 0);
            0
        }
        juicity::DOMAIN => {
            let mut len = [0u8; 1];
            recv.read_exact(&mut len).await.ok()?;
            rest.push(len[0]);
            rest.resize(1 + usize::from(len[0]) + 2, 0);
            1
        }
        _ => return None,
    };
    recv.read_exact(&mut rest[from..]).await.ok()?;
    let mut header = kind.to_vec();
    header.extend_from_slice(&rest);
    Some(header)
}

fn link(server: SocketAddr) -> Link {
    Link::parse(&format!(
        "juicity://{UUID}:{PASSWORD}@{server}?sni=localhost&allow_insecure=1"
    ))
    .unwrap()
}

/// The gate: a UDP session and a TCP session, each byte echoed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_juicity_client_completes_udp_and_tcp_sessions() {
    test_host::need_loopback!();
    let endpoint = pod_quic::endpoint::server(
        pod_quic::endpoint::server_config(crypto()).unwrap(),
        "127.0.0.1:0".parse().unwrap(),
    )
    .unwrap();
    let server = endpoint.local_addr().unwrap();
    let serving = tokio::spawn(async move {
        while let Some(incoming) = endpoint.accept().await {
            let Ok(conn) = incoming.await else { continue };
            tokio::spawn(serve(conn));
        }
    });
    let client = juicity::Juicity::connect(&link(server))
        .await
        .expect("the Juicity client");

    // UDP: one stream, each datagram with its header and its length.
    let udp = client.udp("echo.test", 53).await.expect("the UDP flow");
    let count = 50u8;
    for seq in 0..count {
        let mut datagram = vec![seq];
        datagram.extend_from_slice(&[seq; 23]);
        udp.send(&datagram).await.unwrap();
    }
    let mut seen = Vec::new();
    while seen.len() < usize::from(count) {
        let echo = tokio::time::timeout(Duration::from_secs(5), udp.recv())
            .await
            .expect("each UDP echo in 5 s")
            .expect("the UDP flow ended");
        assert_eq!(echo.len(), 24);
        assert_eq!(echo[1..], [echo[0]; 23]);
        seen.push(echo[0]);
    }
    seen.sort_unstable();
    assert_eq!(
        seen,
        (0..count).collect::<Vec<_>>(),
        "each datagram came back once"
    );

    // TCP: one stream, the header once, then the bytes.
    let (mut send, mut recv) = client.tcp("echo.test", 80).await.expect("the TCP stream");
    send.write_all(b"podnet juicity").await.unwrap();
    let mut back = [0u8; 14];
    tokio::time::timeout(Duration::from_secs(5), recv.read_exact(&mut back))
        .await
        .expect("the TCP echo in 5 s")
        .expect("the TCP echo");
    assert_eq!(&back, b"podnet juicity");
    println!("juicity: udp datagrams={count} tcp bytes=14 both echoed");
    serving.abort();
}

/// The 30-stream gap of `docs/spec_en.md`: the 31st stream opens a second
/// QUIC connection, and the server, which counts what it accepts, sees two.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_juicity_client_opens_a_new_connection_after_thirty_streams() {
    test_host::need_loopback!();
    let endpoint = pod_quic::endpoint::server(
        pod_quic::endpoint::server_config(crypto()).unwrap(),
        "127.0.0.1:0".parse().unwrap(),
    )
    .unwrap();
    let server = endpoint.local_addr().unwrap();
    let accepted = Arc::new(AtomicUsize::new(0));
    let serving = tokio::spawn({
        let accepted = accepted.clone();
        async move {
            while let Some(incoming) = endpoint.accept().await {
                let Ok(conn) = incoming.await else { continue };
                accepted.fetch_add(1, Ordering::Relaxed);
                tokio::spawn(serve(conn));
            }
        }
    });
    let client = juicity::Juicity::connect(&link(server))
        .await
        .expect("the Juicity client");
    assert_eq!(client.connections().await, 1);
    let streams = juicity::STREAM_LIMIT + 1;
    for seq in 0..streams {
        let (mut send, mut recv) = client.tcp("echo.test", 80).await.expect("a TCP stream");
        send.write_all(&[seq as u8]).await.unwrap();
        let mut back = [0u8; 1];
        tokio::time::timeout(Duration::from_secs(5), recv.read_exact(&mut back))
            .await
            .expect("the echo in 5 s")
            .expect("the echo");
        assert_eq!(back[0], seq as u8);
    }
    assert_eq!(
        client.connections().await,
        2,
        "the stream past the bound opens the second connection"
    );
    for _ in 0..50 {
        if accepted.load(Ordering::Relaxed) == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        accepted.load(Ordering::Relaxed),
        2,
        "the server saw two QUIC connections"
    );
    println!(
        "juicity 30-stream: streams={streams} connections=2 server_connections=2 limit={}",
        juicity::STREAM_LIMIT
    );
    serving.abort();
}
