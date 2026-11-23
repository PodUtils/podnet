//! The reference part of the Juicity gate of PodUtils/podnet#486 (QUIC-10):
//! the client of `src/roads/juicity.rs` completes a UDP session and a TCP
//! session against `juicity-server`, the reference implementation.
//!
//! The test needs the release binary of `juicity/juicity` (v0.5.0,
//! `juicity-linux-x86_64.zip`) and its path in `JUICITY_SERVER`. Without it
//! the test prints a named skip; `scripts/ci/reference-tests.sh` refuses to
//! run the gate at all without the binary.
//!
//! ```text
//! JUICITY_SERVER=/path/juicity-server cargo test -p podnet \
//!   --test quic_juicity_reference -- --nocapture
//! ```
//!
//! The certificate, the configuration, the UDP echo and the TCP echo are
//! local: no public network is used. The server binary is the reference
//! implementation, so this test does show interoperability; the stand-in
//! test `quic_juicity` covers the same paths with no binary.

mod test_host;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use podnet::roads::juicity::{self, Link};
use tokio::net::{TcpListener, UdpSocket};
use tokio::process::Command;

const UUID: &str = "123e4567-e89b-12d3-a456-426614174000";
const PASSWORD: &str = "reference-password";

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
    let params = rcgen::CertificateParams::new(vec!["localhost".to_string()]).unwrap();
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_reference_juicity_server_carries_udp_and_tcp() {
    let Some(server_bin) = reference("JUICITY_SERVER", "juicity-server") else {
        return;
    };
    let dir = std::env::temp_dir().join(format!("podnet-juicity-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (cert, key) = certificate(&dir);
    let port = free_port();
    let config = dir.join("server.json");
    std::fs::write(
        &config,
        format!(
            "{{\n  \"listen\": \"127.0.0.1:{port}\",\n  \"users\": {{\"{UUID}\": \"{PASSWORD}\"}},\n  \"certificate\": \"{}\",\n  \"private_key\": \"{}\",\n  \"congestion_control\": \"bbr\",\n  \"log_level\": \"info\"\n}}\n",
            cert.display(),
            key.display()
        ),
    )
    .unwrap();
    let mut server = Command::new(&server_bin)
        .args(["run", "-c"])
        .arg(&config)
        .kill_on_drop(true)
        .spawn()
        .expect("juicity-server starts");
    let udp = udp_echo().await;
    let tcp = tcp_echo().await;
    let link = Link::parse(&format!(
        "juicity://{UUID}:{PASSWORD}@127.0.0.1:{port}?sni=localhost&allow_insecure=1"
    ))
    .unwrap();
    let mut client = None;
    for _ in 0..50 {
        match juicity::Juicity::connect(&link).await {
            Ok(got) => {
                client = Some(got);
                break;
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
    let client = client.unwrap_or_else(|| {
        panic!("the reference server at 127.0.0.1:{port} answered no handshake")
    });
    let count = 20u8;
    let flow = client
        .udp(&udp.ip().to_string(), udp.port())
        .await
        .expect("the UDP flow");
    for seq in 0..count {
        let mut datagram = vec![seq];
        datagram.extend_from_slice(&[seq; 31]);
        flow.send(&datagram).await.unwrap();
    }
    let mut seen = Vec::new();
    while seen.len() < usize::from(count) {
        let echo = tokio::time::timeout(Duration::from_secs(5), flow.recv())
            .await
            .expect("each UDP echo in 5 s")
            .expect("the UDP flow ended");
        assert_eq!(echo.len(), 32);
        assert_eq!(echo[1..], [echo[0]; 31]);
        seen.push(echo[0]);
    }
    seen.sort_unstable();
    assert_eq!(
        seen,
        (0..count).collect::<Vec<_>>(),
        "each datagram came back once"
    );
    let (mut send, mut recv) = client
        .tcp(&tcp.ip().to_string(), tcp.port())
        .await
        .expect("the TCP stream");
    send.write_all(b"podnet juicity reference").await.unwrap();
    let mut back = [0u8; 24];
    tokio::time::timeout(Duration::from_secs(5), recv.read_exact(&mut back))
        .await
        .expect("the TCP echo in 5 s")
        .expect("the TCP echo");
    assert_eq!(&back, b"podnet juicity reference");
    // The 30-stream bound: the stream past the current connection's 30
    // opens another connection, and each stream still echoes.
    let before = client.connections().await;
    for seq in 0..juicity::STREAM_LIMIT {
        let (mut send, mut recv) = client
            .tcp(&tcp.ip().to_string(), tcp.port())
            .await
            .expect("a TCP stream of the 30-stream run");
        send.write_all(&[seq as u8]).await.unwrap();
        let mut one = [0u8; 1];
        tokio::time::timeout(Duration::from_secs(5), recv.read_exact(&mut one))
            .await
            .expect("each echo in 5 s")
            .expect("the echo");
        assert_eq!(one[0], seq as u8);
    }
    assert_eq!(
        client.connections().await,
        before + 1,
        "the stream past the bound opens a second connection"
    );
    println!(
        "juicity reference: server={} udp={udp} tcp={tcp} datagrams={count} tcp_bytes=24 streams={} connections={} both echoed",
        server_bin.display(),
        juicity::STREAM_LIMIT + 2,
        client.connections().await
    );
    let _ = server.kill().await;
    let _ = std::fs::remove_dir_all(&dir);
}
