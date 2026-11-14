//! Tests of `podnet wg serve` (T-125) on the loopback: podnet's own
//! `wg:FILE` network is the client of each peer (podnet to podnet), and the
//! way out of the server is a dialer of the test that records each target.

use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine as _;
use pod_wg::standin;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use super::wg_serve::{read, serve_on, WgServer};
use super::*;
use crate::network::{BoxFuture, Connected, Ctx, Target};

/// A target inside the tunnel: the dialer of the test sends it to the echo.
const FAR: &str = "192.0.2.7:7";

fn b64(key: &[u8; 32]) -> String {
    base64::engine::general_purpose::STANDARD.encode(key)
}

/// A private key and its public key.
fn keypair() -> ([u8; 32], [u8; 32]) {
    let (keys, _) = standin::keys([0; 3]);
    (keys.private, keys.public())
}

/// The way out of the server: each TCP dial goes to `to`, and the target
/// is kept.
struct Recording {
    to: SocketAddr,
    seen: Arc<Mutex<Vec<String>>>,
}

impl Dial for Recording {
    fn label(&self) -> String {
        "recording".into()
    }

    fn dial<'a>(
        &'a self,
        target: &'a Target,
        _deadline: tokio::time::Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(async move {
            self.seen.lock().unwrap().push(target.to_string());
            let stream = tokio::net::TcpStream::connect(self.to)
                .await
                .map_err(|e| Failure::io(Stage::Connect, e))?;
            Ok(Connected::new(Box::new(stream)))
        })
    }
}

/// A TCP echo on the loopback.
async fn echo() -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut io, _)) = listener.accept().await {
            tokio::spawn(async move {
                let (mut r, mut w) = io.split();
                let _ = tokio::io::copy(&mut r, &mut w).await;
            });
        }
    });
    addr
}

/// One peer of the server file: its public key, its `AllowedIPs` and its
/// preshared key.
struct FilePeer<'a> {
    public: [u8; 32],
    allowed: &'a str,
    psk: Option<[u8; 32]>,
}

fn psk_line(psk: Option<[u8; 32]>) -> String {
    psk.map_or(String::new(), |k| format!("PresharedKey = {}\n", b64(&k)))
}

/// The file of the server: the net 10.9.0.0/24, a port of the system.
fn server_file(dir: &Path, private: &[u8; 32], peers: &[FilePeer<'_>]) -> PathBuf {
    let mut text = format!(
        "[Interface]\nPrivateKey = {}\nListenPort = 0\nAddress = 10.9.0.1/24\n",
        b64(private)
    );
    for p in peers {
        text += &format!(
            "\n[Peer]\nPublicKey = {}\nAllowedIPs = {}\n{}",
            b64(&p.public),
            p.allowed,
            psk_line(p.psk)
        );
    }
    let path = dir.join("server.conf");
    std::fs::write(&path, text).unwrap();
    path
}

/// The `wg:FILE` network of a peer of the server at `endpoint`.
fn client(
    dir: &Path,
    name: &str,
    private: &[u8; 32],
    address: &str,
    server: &[u8; 32],
    endpoint: SocketAddr,
    psk: Option<[u8; 32]>,
) -> SpecNetwork {
    let path = dir.join(format!("{name}.conf"));
    std::fs::write(
        &path,
        format!(
            "[Interface]\nPrivateKey = {}\nAddress = {address}/32\n\n[Peer]\nPublicKey = {}\nEndpoint = {endpoint}\nAllowedIPs = 0.0.0.0/0\n{}",
            b64(private),
            b64(server),
            psk_line(psk)
        ),
    )
    .unwrap();
    parse(&format!("wg:{}", path.display())).unwrap()
}

/// A server of `file` on the loopback, with the recording way to `to`.
async fn start(file: &Path, to: SocketAddr) -> (WgServer, Arc<Mutex<Vec<String>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let way = Way::destination(Arc::new(Recording {
        to,
        seen: seen.clone(),
    }));
    let bind: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let server = serve_on(read(file).unwrap(), bind, "server.conf", way)
        .await
        .unwrap();
    (server, seen)
}

/// Send `text` to [`FAR`] through `network`, and read the echo.
async fn round_trip(network: &SpecNetwork, text: &str) -> Result<String, String> {
    let link = network
        .up(&Ctx::default())
        .await
        .map_err(|e| format!("{e:#}"))?;
    let way = link.way.clone().unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let target = Target::Addr(FAR.parse().unwrap());
    let run = async {
        let mut io = way
            .dial
            .dial(&target, deadline)
            .await
            .map_err(|e| e.text().to_string())?
            .io;
        io.write_all(text.as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        let mut back = vec![0u8; text.len()];
        io.read_exact(&mut back).await.map_err(|e| e.to_string())?;
        Ok(String::from_utf8_lossy(&back).into_owned())
    };
    let got = tokio::time::timeout(Duration::from_secs(10), run)
        .await
        .unwrap_or_else(|_| Err("no echo in 10 s".into()));
    link.close().await;
    got
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wg_serve_accepts_a_peer_and_dials_its_flows() {
    crate::test_host::need_loopback!();
    let dir = tempfile::tempdir().unwrap();
    let (server_private, server_public) = keypair();
    let (a_private, a_public) = keypair();
    let peers = [FilePeer {
        public: a_public,
        allowed: "10.9.0.2/32",
        psk: None,
    }];
    let file = server_file(dir.path(), &server_private, &peers);
    let (server, seen) = start(&file, echo().await).await;
    let a = client(
        dir.path(),
        "a",
        &a_private,
        "10.9.0.2",
        &server_public,
        server.local_addr(),
        None,
    );
    assert_eq!(
        round_trip(&a, "through podnet wg serve").await.unwrap(),
        "through podnet wg serve"
    );
    // The flow went out through the way of the server, to its target.
    assert_eq!(*seen.lock().unwrap(), [FAR.to_string()]);
    let status = server.status();
    assert_eq!(status["via"], "recording", "{status}");
    assert_eq!(status["net"], "10.9.0.0/24", "{status}");
}

#[tokio::test]
async fn wg_serve_gives_no_answer_to_an_unknown_key() {
    crate::test_host::need_loopback!();
    let dir = tempfile::tempdir().unwrap();
    let (server_private, server_public) = keypair();
    let (a_private, a_public) = keypair();
    let peers = [FilePeer {
        public: a_public,
        allowed: "10.9.0.2/32",
        psk: None,
    }];
    let file = server_file(dir.path(), &server_private, &peers);
    let (server, _) = start(&file, echo().await).await;
    let sock = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let mut buf = [0u8; 256];
    // A key of the file gets a response.
    let listed = pod_wg::Keys {
        private: a_private,
        peer_public: server_public,
        reserved: [0; 3],
        preshared: None,
    };
    let init = pod_wg::initiate(&listed).unwrap();
    sock.send_to(&init.message, server.local_addr())
        .await
        .unwrap();
    let answer = tokio::time::timeout(Duration::from_secs(5), sock.recv_from(&mut buf))
        .await
        .expect("a response to a listed key")
        .unwrap();
    assert_eq!(buf[0], pod_wg::MSG_RESPONSE);
    assert_eq!(answer.0, pod_wg::RESPONSE_LEN);
    // A key that the file does not list gets nothing.
    let (stranger, _) = keypair();
    let unknown = pod_wg::Keys {
        private: stranger,
        peer_public: server_public,
        reserved: [0; 3],
        preshared: None,
    };
    let init = pod_wg::initiate(&unknown).unwrap();
    sock.send_to(&init.message, server.local_addr())
        .await
        .unwrap();
    let silent = tokio::time::timeout(Duration::from_secs(2), sock.recv_from(&mut buf)).await;
    assert!(silent.is_err(), "an unknown key got an answer");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wg_serve_serves_two_peers_at_once() {
    crate::test_host::need_loopback!();
    let dir = tempfile::tempdir().unwrap();
    let (server_private, server_public) = keypair();
    let (a_private, a_public) = keypair();
    let (b_private, b_public) = keypair();
    let psk = Some([0x3c; 32]);
    let peers = [
        FilePeer {
            public: a_public,
            allowed: "10.9.0.2/32",
            psk: None,
        },
        // The second peer has a preshared key (T-337) and a range.
        FilePeer {
            public: b_public,
            allowed: "10.9.0.8/29",
            psk,
        },
    ];
    let file = server_file(dir.path(), &server_private, &peers);
    let (server, seen) = start(&file, echo().await).await;
    let at = server.local_addr();
    let a = client(
        dir.path(),
        "a",
        &a_private,
        "10.9.0.2",
        &server_public,
        at,
        None,
    );
    let b = client(
        dir.path(),
        "b",
        &b_private,
        "10.9.0.9",
        &server_public,
        at,
        psk,
    );
    let text_a = "the first peer ".repeat(400);
    let text_b = "the second peer ".repeat(400);
    let (got_a, got_b) = tokio::join!(round_trip(&a, &text_a), round_trip(&b, &text_b));
    assert_eq!(got_a.unwrap(), text_a);
    assert_eq!(got_b.unwrap(), text_b);
    assert_eq!(seen.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn wg_serve_drops_a_source_outside_the_allowed_ips_of_its_peer() {
    crate::test_host::need_loopback!();
    let dir = tempfile::tempdir().unwrap();
    let (server_private, server_public) = keypair();
    let (a_private, a_public) = keypair();
    let peers = [FilePeer {
        public: a_public,
        allowed: "10.9.0.2/32",
        psk: None,
    }];
    let file = server_file(dir.path(), &server_private, &peers);
    let (server, seen) = start(&file, echo().await).await;
    // The key of the peer, with an address that its AllowedIPs do not hold.
    let a = client(
        dir.path(),
        "a",
        &a_private,
        "10.9.0.3",
        &server_public,
        server.local_addr(),
        None,
    );
    let err = round_trip(&a, "spoofed").await.unwrap_err();
    assert!(
        err.contains("no answer") || err.contains("no echo"),
        "{err}"
    );
    assert!(seen.lock().unwrap().is_empty());
}

#[test]
fn wg_serve_file_faults_are_one_usage_line() {
    let dir = tempfile::tempdir().unwrap();
    let (private, public) = keypair();
    let key = b64(&private);
    let peer = format!(
        "\n[Peer]\nPublicKey = {}\nAllowedIPs = 10.9.0.2/32\n",
        b64(&public)
    );
    for (text, word) in [
        (format!("[Interface]\nPrivateKey = {key}\nAddress = 10.9.0.1/24\n{peer}"), "no ListenPort"),
        (format!("[Interface]\nPrivateKey = {key}\nListenPort = 51820\n{peer}"), "no IPv4 Address"),
        (format!("[Interface]\nPrivateKey = {key}\nListenPort = 51820\nAddress = 10.9.0.1/24\n"), "no [Peer]"),
        (
            format!("[Interface]\nPrivateKey = {key}\nListenPort = 51820\nAddress = 10.9.0.1/24\n\n[Peer]\nPublicKey = {}\nAllowedIPs = 10.8.0.2/32\n", b64(&public)),
            "not inside the net",
        ),
        (format!("[Interface]\nPrivateKey = {key}\nListenPort = 51820\nAddress = 10.9.0.1/24\n{peer}{peer}"), "PublicKey of an earlier"),
        (format!("[Interface]\nPrivateKey = {key}\nListenPort = 51820\nAddress = 10.9.0.1/24\n{peer}PresharedKey = c2hvcnQ=\n"), "32 bytes"),
    ] {
        let path = dir.path().join("s.conf");
        std::fs::write(&path, &text).unwrap();
        let err = read(&path).err().expect(word);
        assert_eq!(err.class, Class::Usage, "{text}");
        assert!(err.text().contains(word), "{word}: {}", err.text());
        assert!(!err.text().contains('\n'), "{}", err.text());
    }
    let good = format!(
        "[Interface]\nPrivateKey = {key}\nListenPort = 51820\nAddress = 10.9.0.1/24\n{peer}"
    );
    let path = dir.path().join("s.conf");
    std::fs::write(&path, good).unwrap();
    let file = read(&path).unwrap();
    assert_eq!((file.peers(), file.port()), (1, 51820));
}

/// The address of this host on its default route, if it has one: the
/// screen of the server drops a target on the loopback.
fn host_address() -> Option<Ipv4Addr> {
    let sock = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("192.0.2.1:9").ok()?;
    match sock.local_addr().ok()?.ip() {
        std::net::IpAddr::V4(ip) if !ip.is_unspecified() && !ip.is_loopback() => Some(ip),
        _ => None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wg_serve_carries_the_udp_of_a_peer() {
    crate::test_host::need_loopback!();
    let Some(host) = host_address() else {
        eprintln!("skipped: this host has no IPv4 address on a default route");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let (server_private, server_public) = keypair();
    let (a_private, a_public) = keypair();
    let peers = [FilePeer {
        public: a_public,
        allowed: "10.9.0.2/32",
        psk: None,
    }];
    let file = server_file(dir.path(), &server_private, &peers);
    let bind: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let way = Way::destination(Arc::new(crate::dialers::Direct));
    let server = serve_on(read(&file).unwrap(), bind, "server.conf", way)
        .await
        .unwrap();
    // A UDP echo on each address of this host.
    let udp = tokio::net::UdpSocket::bind("0.0.0.0:0").await.unwrap();
    let port = udp.local_addr().unwrap().port();
    tokio::spawn(async move {
        let mut buf = [0u8; 2048];
        while let Ok((n, from)) = udp.recv_from(&mut buf).await {
            let _ = udp.send_to(&buf[..n], from).await;
        }
    });
    let a = client(
        dir.path(),
        "a",
        &a_private,
        "10.9.0.2",
        &server_public,
        server.local_addr(),
        None,
    );
    let link = a.up(&Ctx::default()).await.unwrap();
    let datagrams = link.way.clone().unwrap().dial.datagrams().await.unwrap();
    let to = SocketAddr::from((host, port));
    let mut buf = [0u8; 2048];
    let mut got = None;
    for _ in 0..5 {
        pod_core::datagrams::send_to(&*datagrams, b"a datagram", to)
            .await
            .unwrap();
        let read = pod_core::datagrams::recv_from(&*datagrams, &mut buf);
        if let Ok(Ok((n, from))) = tokio::time::timeout(Duration::from_secs(2), read).await {
            got = Some((buf[..n].to_vec(), from));
            break;
        }
    }
    let (back, from) = got.expect("the echo of the datagram");
    assert_eq!(back, b"a datagram");
    assert_eq!(from, to);
    link.close().await;
}
