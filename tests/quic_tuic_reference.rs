//! The reference part of the TUIC gate of PodUtils/podnet#486 (QUIC-8): the
//! client of `src/roads/tuic` carries a UDP echo at 5 percent loss of the
//! QUIC datagrams and a TCP stream against `tuic-server`, the reference
//! implementation of `tuic-protocol/tuic` (`tuic-server` 2.0.0-dev10, the
//! `tuic-server-x86_64-linux-musl` release asset).
//!
//! The test needs the release binary and its path in `TUIC_SERVER`. Without
//! it the test prints a named skip; `scripts/ci/reference-tests.sh` refuses
//! to run the gate at all without the binary.
//!
//! ```text
//! TUIC_SERVER=/path/tuic-server cargo test -p podnet \
//!   --test quic_tuic_reference -- --nocapture
//! ```
//!
//! The certificate, the configuration, the UDP echo, the TCP echo and the
//! lossy link are local: no public network is used. The server binary is the
//! reference implementation, so this test does show interoperability; the
//! stand-in test `quic_tuic` covers the same path with no binary.

mod test_host;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use podnet::roads::tuic::{self, Link, PENDING_MAX};
use tokio::net::{TcpListener, UdpSocket};
use tokio::process::Command;

const UUID: &str = "123e4567-e89b-12d3-a456-426614174000";
const PASSWORD: &str = "reference-password";
const ALPN: &str = "h3";

/// The reference binary that `var` names, or None with a named skip.
fn reference(var: &str, what: &str) -> Option<PathBuf> {
    let path = match std::env::var_os(var) {
        Some(path) => PathBuf::from(path),
        None => {
            test_host::skip_line(&format!(
                "{var} names no {what}; the reference-server gate cannot run"
            ));
            return None;
        }
    };
    if !path.is_file() {
        test_host::skip_line(&format!(
            "{var} names no file at {}; the reference-server gate cannot run",
            path.display()
        ));
        return None;
    }
    Some(path)
}

/// A free local port of the loopback: a bound socket that is then closed.
fn free_port() -> u16 {
    let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    socket.local_addr().unwrap().port()
}

/// The certificate and the key of the server, as PEM files.
fn certificate(dir: &std::path::Path) -> (PathBuf, PathBuf) {
    let key = rcgen::KeyPair::generate().unwrap();
    let mut params = rcgen::CertificateParams::new(vec!["localhost".to_string()]).unwrap();
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "localhost");
    let cert = params.self_signed(&key).unwrap();
    let cert_path = dir.join("cert.pem");
    let key_path = dir.join("key.pem");
    std::fs::write(&cert_path, cert.pem()).unwrap();
    std::fs::write(&key_path, key.serialize_pem()).unwrap();
    (cert_path, key_path)
}

/// A UDP echo of the loopback: each datagram comes back to its sender.
async fn udp_echo() -> SocketAddr {
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = socket.local_addr().unwrap();
    tokio::spawn(async move {
        let mut buf = vec![0u8; 65535];
        while let Ok((n, from)) = socket.recv_from(&mut buf).await {
            let _ = socket.send_to(&buf[..n], from).await;
        }
    });
    addr
}

/// A TCP echo of the loopback.
async fn tcp_echo() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let (mut read, mut write) = stream.split();
                let _ = tokio::io::copy(&mut read, &mut write).await;
            });
        }
    });
    addr
}

/// A relay of the loopback datagrams with a loss of `drop` of each 100. The
/// first datagram of a source that is not the server names the client; each
/// later datagram of the client goes to the server, and each datagram of the
/// server goes back to the client.
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

/// The gate against the reference server: the queue carries each datagram at
/// 5 percent loss of the link, and a TCP stream echoes.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_reference_tuic_server_carries_udp_at_loss_and_tcp() {
    let Some(server_bin) = reference("TUIC_SERVER", "tuic-server") else {
        return;
    };
    let version = Command::new(&server_bin)
        .arg("--version")
        .output()
        .await
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".into());
    let dir = std::env::temp_dir().join(format!("podnet-tuic-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (cert, key) = certificate(&dir);
    let port = free_port();
    // The server drops a loopback target by default; the test's target is
    // the loopback, so both guards are off.
    let config = dir.join("config.toml");
    std::fs::write(
        &config,
        format!(
            "log_level = \"info\"\nserver = \"127.0.0.1:{port}\"\ndata_dir = {data:?}\n\n\
             [users]\n\"{UUID}\" = {password:?}\n\n\
             [tls]\ncertificate = {cert:?}\nprivate_key = {key:?}\nalpn = [\"{ALPN}\"]\nhostname = \"localhost\"\n\n\
             [experimental]\ndrop_loopback = false\ndrop_private = false\n",
            data = dir.display().to_string(),
            password = PASSWORD,
            cert = cert.display().to_string(),
            key = key.display().to_string(),
        ),
    )
    .unwrap();
    let log = std::fs::File::create(dir.join("tuic.log")).unwrap();
    let mut server = Command::new(&server_bin)
        .args(["-c"])
        .arg(&config)
        .stdout(Stdio::from(log.try_clone().unwrap()))
        .stderr(Stdio::from(log))
        .kill_on_drop(true)
        .spawn()
        .expect("tuic-server starts");
    // The server is ready once its QUIC answers the probe's packet.
    let server_addr = SocketAddr::from(([127, 0, 0, 1], port));
    let until = std::time::Instant::now() + Duration::from_secs(20);
    while pod_quic::probe::probe(&[server_addr], Duration::from_millis(200))
        .await
        .is_err()
    {
        if std::time::Instant::now() >= until {
            let said = std::fs::read_to_string(dir.join("tuic.log")).unwrap_or_default();
            panic!("tuic-server did not answer over QUIC: {said}");
        }
    }
    let udp_target = udp_echo().await;
    let tcp_target = tcp_echo().await;
    let (front, dropped, passed) = lossy(server_addr, 5).await;
    let link = Link::parse(&format!(
        "tuic://{UUID}:{PASSWORD}@{front}?sni=localhost&allow_insecure=1"
    ))
    .unwrap();
    let tcp_link = Link::parse(&format!(
        "tuic://{UUID}:{PASSWORD}@{front}?sni=localhost&allow_insecure=1"
    ))
    .unwrap();
    // TCP: one bidirectional stream, the proxy header, then the bytes.
    let client = tuic::Tuic::connect(&tcp_link)
        .await
        .expect("the TCP client");
    let (mut send, mut recv) = client
        .tcp(&tcp_target.ip().to_string(), tcp_target.port())
        .await
        .expect("the TCP stream");
    send.write_all(b"podnet tuic reference").await.unwrap();
    let mut back = [0u8; 21];
    tokio::time::timeout(Duration::from_secs(5), recv.read_exact(&mut back))
        .await
        .expect("the TCP echo in 5 s")
        .expect("the TCP echo");
    assert_eq!(&back, b"podnet tuic reference");
    // UDP: the bounded queue of the road, through the lossy link, to a local
    // echo. The queue is the one the gate names.
    let local = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let relay_addr = local.local_addr().unwrap();
    let relay = tokio::spawn(async move {
        tuic::udp_relay(
            &link,
            &udp_target.ip().to_string(),
            udp_target.port(),
            local,
            Duration::from_secs(8),
        )
        .await
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
        let got = tokio::time::timeout(Duration::from_secs(15), sender.recv(&mut buf))
            .await
            .expect("each echo in 15 s")
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
        "tuic reference: server={} version={version} udp={udp_target} tcp={tcp_target} \
         datagrams={count} tcp_bytes=21 link_dropped={lost} link_passed={ok} loss={percent:.1}% \
         queue={:?}",
        server_bin.display(),
        stats.queue
    );
    let _ = server.kill().await;
    let _ = std::fs::remove_dir_all(&dir);
}
