//! The local SOCKS5 door at the opener of a link (T-291): `podnet link dial
//! --service socks5 --listen ADDRESS`. A local client speaks SOCKS5 to the
//! door; the door speaks to the service `socks5` of the peer over the link
//! (a unix socket). The origins are on the loopback and the tests skip
//! without one.

mod test_host;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::time::Duration;

use serde_json::Value;

/// One install: a state directory and a run directory of its own.
struct Host {
    dir: tempfile::TempDir,
}

impl Host {
    fn new() -> Host {
        Host {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_podnet"));
        cmd.args(args)
            .env("PODNET_STATE_DIR", self.path("state"))
            .env("PODNET_RUN_DIR", self.path("run"))
            .env_remove("PODNET_LINK_CONTROL")
            .env_remove("RUST_LOG");
        cmd
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command(args).output().unwrap()
    }

    fn fingerprint(&self) -> String {
        let out = self.run(&["link", "status", "--create", "--json"]);
        let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
        doc["identity"]["fingerprint"].as_str().unwrap().to_string()
    }
}

struct Proc(Child);

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn start(cmd: &mut Command) -> (Proc, String) {
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.as_mut().unwrap())
        .read_line(&mut line)
        .unwrap();
    if line.is_empty() {
        let mut err = String::new();
        let _ = child.stderr.take().unwrap().read_to_string(&mut err);
        let _ = child.kill();
        panic!("no first line: {err}");
    }
    (Proc(child), line)
}

/// A listener with the service `socks5` and a stranger-proof allow list.
struct Pair {
    server: Host,
    client: Host,
    server_fp: String,
    _listener: Proc,
}

fn pair() -> Pair {
    let (server, client) = (Host::new(), Host::new());
    let (sfp, cfp) = (server.fingerprint(), client.fingerprint());
    let allow = server.path("allow");
    std::fs::write(&allow, &cfp).unwrap();
    let sock = server.path("l.sock");
    let (l, line) = start(&mut server.command(&[
        "link",
        "listen",
        sock.to_str().unwrap(),
        "--allow",
        allow.to_str().unwrap(),
        "--service",
        "socks5",
        "--json",
    ]));
    let ready: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(ready["services"]["socks5"], "socks5", "{ready}");
    Pair {
        server,
        client,
        server_fp: sfp,
        _listener: l,
    }
}

impl Pair {
    /// The door of the client install: the process and its address.
    fn door(&self, extra: &[&str]) -> (Proc, std::net::SocketAddr) {
        let sock = self.server.path("l.sock");
        let mut args = vec![
            "link",
            "dial",
            sock.to_str().unwrap(),
            "--peer",
            &self.server_fp,
            "--service",
            "socks5",
            "--listen",
            "127.0.0.1:0",
        ];
        args.extend_from_slice(extra);
        let (proc, line) = start(&mut self.client.command(&args));
        let addr = line
            .split_whitespace()
            .find_map(|w| w.parse::<std::net::SocketAddr>().ok())
            .unwrap_or_else(|| panic!("no address in {line:?}"));
        (proc, addr)
    }
}

fn echo() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for conn in listener.incoming().flatten() {
            std::thread::spawn(move || {
                let mut conn = conn;
                let mut buf = [0u8; 4096];
                while let Ok(n) = conn.read(&mut buf) {
                    if n == 0 || conn.write_all(&buf[..n]).is_err() {
                        break;
                    }
                }
            });
        }
    });
    port
}

fn udp_echo() -> u16 {
    let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let port = socket.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let mut buf = [0u8; 65535];
        while let Ok((n, from)) = socket.recv_from(&mut buf) {
            let _ = socket.send_to(&buf[..n], from);
        }
    });
    port
}

fn client(door: std::net::SocketAddr) -> TcpStream {
    let s = TcpStream::connect(door).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
    s
}

/// The greeting, then a request; the reply as (code, address).
fn ask(s: &mut TcpStream, command: u8, atyp: u8, addr: &[u8], port: u16) -> (u8, SocketAddr) {
    let mut wire = vec![5, 1, 0, 5, command, 0, atyp];
    wire.extend_from_slice(addr);
    wire.extend_from_slice(&port.to_be_bytes());
    s.write_all(&wire).unwrap();
    reply(s)
}

fn reply(s: &mut TcpStream) -> (u8, SocketAddr) {
    let mut head = [0u8; 2];
    s.read_exact(&mut head).unwrap_or_default();
    assert_eq!(head, [5, 0], "the method answer");
    read_reply(s)
}

fn read_reply(s: &mut TcpStream) -> (u8, SocketAddr) {
    let mut r = [0u8; 10];
    s.read_exact(&mut r).unwrap();
    (
        r[1],
        SocketAddr::from(([r[4], r[5], r[6], r[7]], u16::from_be_bytes([r[8], r[9]]))),
    )
}

#[test]
fn a_connect_through_the_local_door_reaches_an_origin_at_the_peer() {
    test_host::need_loopback!();
    let pair = pair();
    let port = echo();
    let (_door, addr) = pair.door(&[]);
    let mut s = client(addr);
    let (code, _) = ask(&mut s, 1, 1, &[127, 0, 0, 1], port);
    assert_eq!(code, 0);
    s.write_all(b"through the door").unwrap();
    let mut got = [0u8; 16];
    s.read_exact(&mut got).unwrap();
    assert_eq!(&got, b"through the door");
    // A second client has its own stream.
    let mut t = client(addr);
    let (code, _) = ask(&mut t, 1, 1, &[127, 0, 0, 1], port);
    assert_eq!(code, 0);
}

#[test]
fn a_connect_to_a_closed_port_gets_a_failure_reply() {
    test_host::need_loopback!();
    let pair = pair();
    let closed = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let (_door, addr) = pair.door(&[]);
    let mut s = client(addr);
    let (code, _) = ask(&mut s, 1, 1, &[127, 0, 0, 1], closed);
    assert_ne!(code, 0);
}

#[test]
fn a_name_goes_to_the_peer_and_resolves_there_or_here() {
    test_host::need_loopback!();
    test_host::need_localhost!();
    let pair = pair();
    let port = echo();
    for extra in [&[][..], &["--resolve-at", "local"][..]] {
        let (_door, addr) = pair.door(extra);
        let mut s = client(addr);
        let mut name = vec![9u8];
        name.extend_from_slice(b"localhost");
        let (code, _) = ask(&mut s, 1, 3, &name, port);
        assert_eq!(code, 0, "{extra:?}");
        s.write_all(b"by name").unwrap();
        let mut got = [0u8; 7];
        s.read_exact(&mut got).unwrap();
        assert_eq!(&got, b"by name");
    }
}

#[test]
fn a_bind_through_the_local_door_listens_at_the_peer() {
    test_host::need_loopback!();
    let pair = pair();
    let (_door, addr) = pair.door(&[]);
    let mut s = client(addr);
    let (code, listening) = ask(&mut s, 2, 1, &[127, 0, 0, 1], 0);
    assert_eq!(code, 0);
    let mut incoming = TcpStream::connect(listening).unwrap();
    let (code, from) = read_reply(&mut s);
    assert_eq!(code, 0);
    assert_eq!(from, incoming.local_addr().unwrap());
    incoming.write_all(b"server to client").unwrap();
    let mut got = [0u8; 16];
    s.read_exact(&mut got).unwrap();
    assert_eq!(&got, b"server to client");
    s.write_all(b"client to server").unwrap();
    incoming
        .set_read_timeout(Some(Duration::from_secs(20)))
        .unwrap();
    incoming.read_exact(&mut got).unwrap();
    assert_eq!(&got, b"client to server");
}

#[test]
fn a_udp_datagram_goes_through_the_local_door_to_an_echo_and_back() {
    test_host::need_loopback!();
    let pair = pair();
    let origin = udp_echo();
    let (_door, addr) = pair.door(&[]);
    let mut control = client(addr);
    let (code, relay) = ask(&mut control, 3, 1, &[0, 0, 0, 0], 0);
    assert_eq!(code, 0);
    let udp = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    udp.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
    let mut datagram = vec![0, 0, 0, 1, 127, 0, 0, 1];
    datagram.extend_from_slice(&origin.to_be_bytes());
    datagram.extend_from_slice(b"ping over the link");
    let relay = SocketAddr::from(([127, 0, 0, 1], relay.port()));
    udp.send_to(&datagram, relay).unwrap();
    let mut buf = [0u8; 2048];
    let (n, _) = udp.recv_from(&mut buf).unwrap();
    assert_eq!(&buf[..4], &[0, 0, 0, 1], "{:?}", &buf[..n]);
    assert_eq!(u16::from_be_bytes([buf[8], buf[9]]), origin);
    assert_eq!(&buf[10..n], b"ping over the link");
    drop(control);
}

#[test]
fn listen_needs_the_service_socks5() {
    let host = Host::new();
    let fp = host.fingerprint();
    let out = host.run(&[
        "link",
        "dial",
        "/nonexistent",
        "--peer",
        &fp,
        "--listen",
        "127.0.0.1:0",
    ]);
    assert_eq!(out.status.code(), Some(64), "{}", text(&out.stderr));
}

#[tokio::test]
async fn the_resolve_choice_decides_the_address_type_the_peer_gets() {
    use pod_core::{Dial, Target};
    use podnet::link::Settings;
    use podnet::link_door::{Resolve, Way};
    use podnet::link_service as service;
    test_host::need_localhost!();
    for (resolve, atyp) in [(Resolve::Peer, 3u8), (Resolve::Local, 1u8)] {
        let (opener, far) = podnet::link::unix_pair(Settings::default()).unwrap();
        // A peer that records the address type of the CONNECT, then refuses.
        let peer = tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let mut s = far.accept().await.unwrap();
            assert_eq!(service::accept(&mut s).await.unwrap(), "socks5");
            service::reply(&mut s, service::OPEN).await.unwrap();
            let mut greeting = [0u8; 3];
            s.read_exact(&mut greeting).await.unwrap();
            s.write_all(&[5, 0]).await.unwrap();
            let mut head = [0u8; 4];
            s.read_exact(&mut head).await.unwrap();
            assert_eq!(head[..2], [5, 1]);
            s.write_all(&[5, 1, 0, 1, 0, 0, 0, 0, 0, 0]).await.unwrap();
            (head[3], far)
        });
        let way = Way::new(std::sync::Arc::new(opener), resolve);
        let target = Target::Name {
            host: "localhost".into(),
            port: 80,
        };
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        assert!(way.dial(&target, deadline).await.is_err());
        let (got, _far) = peer.await.unwrap();
        assert_eq!(got, atyp, "{resolve:?}");
    }
}
