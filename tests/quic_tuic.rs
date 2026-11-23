//! The TUIC gate of PodUtils/podnet#486 (QUIC-8): UDP echo through the TUIC
//! road at 5 percent loss of the QUIC packets, with the bounded queue and
//! its stop-read signal asserted.
//!
//! The server and the lossy link are in this file: the server answers the
//! commands of TUIC version 5 (`EAimTY/tuic`'s `SPEC.md`) with an echo, and
//! the link drops 5 of each 100 datagrams of the loopback. The test proves
//! the client of the road, the reliability of QUIC's own retransmission
//! over a lossy path, and the bound of the queue. The server is written
//! from the specification for this test, not a reference TUIC server, so
//! this is not a proof of interoperability with another implementation.

mod test_host;

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use pod_quic::noq;
use pod_quic::noq::crypto::rustls::QuicServerConfig;
use podnet::roads::tuic::{self, Link, PENDING_MAX};
use tokio::net::UdpSocket;

const UUID: &str = "123e4567-e89b-12d3-a456-426614174000";
const PASSWORD: &str = "test-password";
const ALPN: &[u8] = b"h3";

fn crypto() -> Arc<QuicServerConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let key = rcgen::KeyPair::generate().unwrap();
    let mut params = rcgen::CertificateParams::new(vec!["localhost".to_string()]).unwrap();
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "localhost");
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

/// The server: one connection, its authentication, and the echo of each
/// packet stream and each TCP stream.
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
    if bytes[..2] != [5, 0] || bytes[2..18] != *uuid.as_bytes() || bytes[18..] != token {
        return;
    }
    let _auth = auth;
    let packets = {
        let conn = conn.clone();
        tokio::spawn(async move {
            while let Ok(mut stream) = conn.accept_uni().await {
                let conn = conn.clone();
                tokio::spawn(async move {
                    let Ok(cmd) = stream.read_to_end(64 * 1024).await else {
                        return;
                    };
                    if let Ok(mut out) = conn.open_uni().await {
                        let _ = out.write_all(&cmd).await;
                        let _ = out.finish();
                    }
                });
            }
        })
    };
    let tcp = {
        let conn = conn.clone();
        tokio::spawn(async move {
            while let Ok((mut send, mut recv)) = conn.accept_bi().await {
                tokio::spawn(async move {
                    let mut head = [0u8; 2];
                    if recv.read_exact(&mut head).await.is_err() {
                        return;
                    }
                    let extra = match head[1] {
                        0x00 => {
                            let mut len = [0u8; 1];
                            if recv.read_exact(&mut len).await.is_err() {
                                return;
                            }
                            usize::from(len[0])
                        }
                        0x01 => 4,
                        0x02 => 16,
                        _ => return,
                    };
                    let mut rest = vec![0u8; extra + 2];
                    if recv.read_exact(&mut rest).await.is_err() {
                        return;
                    }
                    let _ = tokio::io::copy(&mut recv, &mut send).await;
                    let _ = send.finish();
                });
            }
        })
    };
    let _ = conn.closed().await;
    packets.abort();
    tcp.abort();
}

/// A link of the loopback with the certificate of the test.
fn link(server: SocketAddr) -> Link {
    Link::parse(&format!(
        "tuic://{UUID}:{PASSWORD}@{server}?sni=localhost&allow_insecure=1"
    ))
    .unwrap()
}

/// A relay of datagrams with a loss of `drop` of each 100. The first
/// datagram from a new source names the client; each later datagram of the
/// client goes to the server, and each datagram of the server goes back.
async fn lossy(server: SocketAddr, drop: u64) -> (SocketAddr, Arc<AtomicU64>, Arc<AtomicU64>) {
    let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
    let addr = socket.local_addr().unwrap();
    let dropped = Arc::new(AtomicU64::new(0));
    let passed = Arc::new(AtomicU64::new(0));
    let (d, p) = (dropped.clone(), passed.clone());
    tokio::spawn(async move {
        let mut client: Option<SocketAddr> = None;
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut buf = vec![0u8; 65535];
        loop {
            let Ok((n, from)) = socket.recv_from(&mut buf).await else {
                return;
            };
            if from == server {
                if let Some(to) = client {
                    let _ = socket.send_to(&buf[..n], to).await;
                }
                continue;
            }
            client = Some(from);
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            if (seed >> 33) % 100 < drop {
                d.fetch_add(1, Ordering::Relaxed);
                continue;
            }
            p.fetch_add(1, Ordering::Relaxed);
            let _ = socket.send_to(&buf[..n], server).await;
        }
    });
    (addr, dropped, passed)
}

/// The gate: 200 datagrams, 5 percent of the packets lost, each echo back,
/// and the queue inside its bound.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn udp_echo_survives_five_percent_loss_within_the_queue_bound() {
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
    let (front, dropped, passed) = lossy(server, 5).await;
    let local = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let relay_addr = local.local_addr().unwrap();
    let relay = tokio::spawn(async move {
        tuic::udp_relay(&link(front), "echo.test", 7, local, Duration::from_secs(6)).await
    });
    let sender = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    sender.connect(relay_addr).await.unwrap();
    let count = 200u16;
    for seq in 0..count {
        let mut datagram = seq.to_be_bytes().to_vec();
        datagram.extend_from_slice(&[seq as u8; 30]);
        sender.send(&datagram).await.unwrap();
    }
    let mut seen = Vec::new();
    let mut buf = vec![0u8; 64];
    while seen.len() < usize::from(count) {
        let got = tokio::time::timeout(Duration::from_secs(10), sender.recv(&mut buf))
            .await
            .expect("each echo in 10 s")
            .unwrap();
        assert_eq!(got, 32, "the echo of one datagram");
        let seq = u16::from_be_bytes([buf[0], buf[1]]);
        assert_eq!(&buf[2..got], &[seq as u8; 30], "the bytes of the echo");
        seen.push(seq);
    }
    seen.sort_unstable();
    assert_eq!(
        seen,
        (0..count).collect::<Vec<_>>(),
        "each datagram came back once"
    );
    let stats = tokio::time::timeout(Duration::from_secs(12), relay)
        .await
        .expect("the relay ends")
        .unwrap()
        .expect("the relay");
    let lost = dropped.load(Ordering::Relaxed);
    let ok = passed.load(Ordering::Relaxed);
    let percent = lost as f64 * 100.0 / (lost + ok).max(1) as f64;
    assert!(lost > 0, "the link dropped no datagram: {lost}/{ok}");
    assert!(
        (1.0..15.0).contains(&percent),
        "the loss of the link: {percent:.1} percent"
    );
    assert_eq!(stats.sent, u64::from(count), "each datagram was queued");
    assert_eq!(stats.received, u64::from(count), "each echo came back");
    assert!(
        stats.queue.max_pending <= PENDING_MAX,
        "the queue: {:?}",
        stats.queue
    );
    println!(
        "tuic udp: sent={} received={} link_dropped={lost} link_passed={ok} loss={percent:.1}% queue={:?}",
        stats.sent, stats.received, stats.queue
    );
    serving.abort();
}
