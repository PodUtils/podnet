//! GitHub #485: a cone NAT and a symmetric NAT between three real network
//! namespaces, and the direct and relayed paths of `podnet link p2p`.
//!
//! The lab has three namespaces: a client `c` on 10.10.1.2/24, a router `r`
//! with 10.10.1.1/24 and 10.10.2.1/24 on two veth pairs, and a server `s` on
//! 10.10.2.2/24. The router forwards and NATs with iptables (this host has
//! no nft binary).
//!
//! * Cone: the router gives every datagram of the client one fixed public
//!   mapping, 10.10.2.1:40100, and a DNAT sends each packet that arrives at
//!   that mapping back to the client. The client sends the first datagram
//!   to the server, the server answers from another source port, and the
//!   answer arrives. `podnet link p2p --direct 10.10.2.1:40100` reaches the
//!   client's peer and prints `path direct`; it allocates no relay.
//! * Symmetric: the router uses `MASQUERADE --random-fully`, so a mapping
//!   is per destination: the client's two probes from one source port reach
//!   the server as two different public ports. The server answers the
//!   mapping that it learned, from a different source port, and no answer
//!   arrives. `podnet link p2p --direct 10.10.2.1:MAPPING` is refused, and
//!   the TURN relay in the router carries the stream: `path relay` with
//!   `direct refused`.
//!
//! The test is not ignored. It skips with a named reason only when the
//! process has no CAP_NET_ADMIN; `scripts/ci/root-netns-tests.sh` runs it as
//! root, where the capability is present.

mod test_host;

#[path = "../src/networks/turn_mock.rs"]
mod mock;

use std::io::Write as _;
use std::net::{SocketAddr, TcpListener, UdpSocket};
use std::os::fd::AsRawFd;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// The client, the router's two addresses and the server.
const CLIENT: &str = "10.10.1.2";
const ROUTER_CLIENT: &str = "10.10.1.1";
const ROUTER: &str = "10.10.2.1";
const SERVER: &str = "10.10.2.2";
/// The fixed public UDP mapping of the cone NAT.
const CONE_PORT: u16 = 40100;
/// The client's UDP source port and its TCP peer port.
const CLIENT_UDP: u16 = 5555;
const CLIENT_PEER: u16 = 5556;
/// The two destinations of the probes, and the server's answer port.
const PROBE_A: u16 = 9001;
const PROBE_B: u16 = 9003;
const ANSWER: u16 = 9002;

/// True when this process holds CAP_NET_ADMIN (bit 12).
fn cap_net_admin() -> bool {
    let Ok(status) = std::fs::read_to_string("/proc/self/status") else {
        return false;
    };
    let Some(line) = status.lines().find(|line| line.starts_with("CapEff:")) else {
        return false;
    };
    let Ok(bits) = u64::from_str_radix(line[7..].trim(), 16) else {
        return false;
    };
    bits & (1 << 12) != 0
}

/// A command that must succeed, with its stderr in the panic.
fn ok(cmd: &mut Command) -> Output {
    let out = cmd.output().expect("the command runs");
    assert!(
        out.status.success(),
        "{cmd:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

/// `ip` with the arguments of the test.
fn ip(args: &[&str]) -> Output {
    ok(Command::new("ip").args(args))
}

/// `iptables -t TABLE` in `ns`.
fn iptables(ns: &str, table: &str, args: &[&str]) {
    let mut cmd = Command::new("ip");
    cmd.args(["netns", "exec", ns, "iptables", "-t", table])
        .args(args);
    ok(&mut cmd);
}

/// The two veth pairs, the three namespaces and their addresses. The name
/// of each namespace, interface and port is unique to this process, so two
/// test binaries of one host do not collide.
struct Lab {
    r: String,
    c: String,
    s: String,
    r0: String,
    r1: String,
}

impl Drop for Lab {
    fn drop(&mut self) {
        for ns in [&self.s, &self.c, &self.r] {
            let _ = Command::new("ip").args(["netns", "del", ns]).output();
        }
    }
}

impl Lab {
    fn build(tag: &str) -> Lab {
        let pid = std::process::id();
        let lab = Lab {
            r: format!("pn{pid}{tag}r"),
            c: format!("pn{pid}{tag}c"),
            s: format!("pn{pid}{tag}s"),
            r0: format!("r0{pid}{tag}"),
            r1: format!("r1{pid}{tag}"),
        };
        std::fs::create_dir_all("/run/netns").expect("a place for the namespaces");
        for ns in [&lab.s, &lab.c, &lab.r] {
            let _ = Command::new("ip").args(["netns", "del", ns]).output();
            ip(&["netns", "add", ns]);
        }
        let c0 = format!("c0{pid}{tag}");
        let s0 = format!("s0{pid}{tag}");
        ip(&["link", "add", &c0, "type", "veth", "peer", "name", &lab.r0]);
        ip(&["link", "add", &lab.r1, "type", "veth", "peer", "name", &s0]);
        ip(&["link", "set", &c0, "netns", &lab.c]);
        ip(&["link", "set", &lab.r0, "netns", &lab.r]);
        ip(&["link", "set", &lab.r1, "netns", &lab.r]);
        ip(&["link", "set", &s0, "netns", &lab.s]);
        for (ns, device, address) in [
            (&lab.c, &c0, format!("{CLIENT}/24")),
            (&lab.r, &lab.r0, format!("{ROUTER_CLIENT}/24")),
            (&lab.r, &lab.r1, format!("{ROUTER}/24")),
            (&lab.s, &s0, format!("{SERVER}/24")),
        ] {
            ip(&["-n", ns, "addr", "add", &address, "dev", device]);
            ip(&["-n", ns, "link", "set", device, "up"]);
            ip(&["-n", ns, "link", "set", "lo", "up"]);
        }
        ip(&[
            "-n",
            &lab.c,
            "route",
            "add",
            "default",
            "via",
            ROUTER_CLIENT,
        ]);
        ip(&["-n", &lab.s, "route", "add", "default", "via", ROUTER]);
        ip(&[
            "netns",
            "exec",
            &lab.r,
            "sysctl",
            "-qw",
            "net.ipv4.ip_forward=1",
        ]);
        for (from, to) in [(&lab.r0, &lab.r1), (&lab.r1, &lab.r0)] {
            iptables(
                &lab.r,
                "filter",
                &["-A", "FORWARD", "-i", from, "-o", to, "-j", "ACCEPT"],
            );
        }
        ip(&[
            "netns", "exec", &lab.c, "ping", "-c", "1", "-W", "2", SERVER,
        ]);
        lab
    }

    /// The full-cone mapping: one public port for every datagram of the
    /// client, and a DNAT from that port back to the client.
    fn cone(&self) {
        let client = format!("{CLIENT}/32");
        let to = format!("{ROUTER}:{CONE_PORT}");
        let port = CONE_PORT.to_string();
        iptables(
            &self.r,
            "nat",
            &[
                "-A",
                "POSTROUTING",
                "-o",
                &self.r1,
                "-s",
                &client,
                "-p",
                "udp",
                "-j",
                "SNAT",
                "--to-source",
                &to,
            ],
        );
        iptables(
            &self.r,
            "nat",
            &[
                "-A",
                "PREROUTING",
                "-i",
                &self.r1,
                "-p",
                "udp",
                "--dport",
                &port,
                "-j",
                "DNAT",
                "--to-destination",
                &format!("{CLIENT}:{CLIENT_UDP}"),
            ],
        );
        iptables(
            &self.r,
            "nat",
            &[
                "-A",
                "PREROUTING",
                "-i",
                &self.r1,
                "-p",
                "tcp",
                "--dport",
                &port,
                "-j",
                "DNAT",
                "--to-destination",
                &format!("{CLIENT}:{CLIENT_PEER}"),
            ],
        );
    }

    /// The symmetric mapping: a random public port for each destination.
    fn symmetric(&self) {
        let client = format!("{CLIENT}/32");
        iptables(
            &self.r,
            "nat",
            &[
                "-A",
                "POSTROUTING",
                "-o",
                &self.r1,
                "-s",
                &client,
                "-j",
                "MASQUERADE",
                "--random-fully",
            ],
        );
    }
}

/// Run `f` on a thread that has entered the network namespace `ns`.
fn spawn_in<T, F>(ns: &str, f: F) -> std::thread::JoinHandle<T>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let ns = ns.to_string();
    std::thread::spawn(move || {
        let file = std::fs::File::open(format!("/run/netns/{ns}")).expect("the namespace");
        // SAFETY: setns of this thread to the network namespace of `ns`.
        let rc = unsafe { libc::setns(file.as_raw_fd(), libc::CLONE_NEWNET) };
        assert_eq!(rc, 0, "setns: {}", std::io::Error::last_os_error());
        f()
    })
}

/// The two probe sources that the server saw, and the answers that the
/// client received.
struct Probe {
    sources: Vec<SocketAddr>,
    replies: usize,
}

/// The client sends first from one source port to two destinations, and the
/// server answers each source from [`ANSWER`], a different port.
fn probe(lab: &Lab) -> Probe {
    let (ready_tx, ready_rx) = mpsc::channel();
    let (server_tx, server_rx) = mpsc::channel();
    let server = spawn_in(&lab.s, move || {
        let a = UdpSocket::bind((SERVER, PROBE_A)).expect("the first probe port");
        let b = UdpSocket::bind((SERVER, PROBE_B)).expect("the second probe port");
        let answer = UdpSocket::bind((SERVER, ANSWER)).expect("the answer port");
        ready_tx.send(()).expect("the client waits");
        let mut sources = Vec::new();
        for socket in [&a, &b] {
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("a probe time limit");
            let mut buf = [0u8; 64];
            let (n, from) = socket.recv_from(&mut buf).expect("a probe");
            sources.push(from);
            answer
                .send_to(&buf[..n], from)
                .expect("an answer to the mapping");
        }
        server_tx.send(sources).expect("the test waits");
    });
    ready_rx
        .recv_timeout(Duration::from_secs(30))
        .expect("the probe server comes up");
    let client = spawn_in(&lab.c, move || {
        let socket = UdpSocket::bind((CLIENT, CLIENT_UDP)).expect("the client's source");
        socket
            .set_read_timeout(Some(Duration::from_millis(2000)))
            .expect("the client's time limit");
        for port in [PROBE_A, PROBE_B] {
            socket
                .send_to(b"probe", (SERVER, port))
                .expect("the first datagram");
        }
        let mut replies = 0;
        let mut buf = [0u8; 64];
        while socket.recv_from(&mut buf).is_ok() {
            replies += 1;
        }
        replies
    });
    let sources = server_rx
        .recv_timeout(Duration::from_secs(30))
        .expect("the server saw both probes");
    let replies = client.join().expect("the client ends");
    server.join().expect("the server ends");
    Probe { sources, replies }
}

/// The TCP peer of the client: it accepts each stream and holds it.
fn tcp_peer(lab: &Lab) -> SocketAddr {
    let (tx, rx) = mpsc::channel();
    let _ = spawn_in(&lab.c, move || {
        let listener = TcpListener::bind((CLIENT, CLIENT_PEER)).expect("the client's TCP peer");
        tx.send(()).expect("the test waits");
        let mut kept = Vec::new();
        for stream in listener.incoming() {
            let Ok(stream) = stream else { return };
            kept.push(stream);
        }
    });
    rx.recv_timeout(Duration::from_secs(30))
        .expect("the TCP peer comes up");
    format!("{CLIENT}:{CLIENT_PEER}").parse().unwrap()
}

/// The TURN stand-in in the router, and the requests that it took.
struct Relay {
    addr: SocketAddr,
    control: mock::Shared,
}

impl Relay {
    fn start(lab: &Lab) -> Relay {
        let (tx, rx) = mpsc::channel();
        let _ = spawn_in(&lab.r, move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("the runtime of the stand");
            runtime.block_on(async move {
                let server = mock::start_on(format!("{ROUTER_CLIENT}:0").parse().unwrap()).await;
                tx.send((server.addr, server.control.clone()))
                    .expect("the test waits");
                std::future::pending::<()>().await;
            });
        });
        let (addr, control) = rx
            .recv_timeout(Duration::from_secs(30))
            .expect("the TURN stand comes up");
        Relay { addr, control }
    }

    /// The `CONNECT`s that the relay carried.
    fn connects(&self) -> usize {
        self.control
            .lock()
            .unwrap()
            .seen
            .iter()
            .filter(|(method, code)| *method == pod_turn::stun::CONNECT && *code == 0)
            .count()
    }
}

/// A password file for the account of the stand.
fn password_file(path: &Path) {
    let mut file = std::fs::File::create(path).expect("the password file");
    writeln!(file, "{}", mock::PASSWORD).expect("the password");
}

/// `podnet link p2p` in the server namespace, with the account of the stand.
fn p2p(
    lab: &Lab,
    direct: SocketAddr,
    peer: SocketAddr,
    relay: SocketAddr,
    password: &Path,
) -> Output {
    let mut cmd = Command::new("ip");
    cmd.args([
        "netns",
        "exec",
        &lab.s,
        env!("CARGO_BIN_EXE_podnet"),
        "link",
        "p2p",
        "--peer",
        &peer.to_string(),
        "--direct",
        &direct.to_string(),
        "--relay",
        &format!("turn://{relay}"),
        "--timeout",
        "1500",
        "--check",
    ]);
    for name in [
        "PODNET_TURN_USER",
        "PODNET_TURN_PASSWORD",
        "PODNET_TURN_PASSWORD_FILE",
        "PODNET_TURN_REALM",
        "PODNET_TURN_LIFETIME",
        "PODNET_TURN_RATE_LIMIT",
        "PODNET_PROXY",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
        "NODE_USE_ENV_PROXY",
        "RUST_LOG",
    ] {
        cmd.env_remove(name);
    }
    cmd.env("PODNET_TURN_USER", mock::USER)
        .env("PODNET_TURN_PASSWORD_FILE", password)
        .stdin(Stdio::null());
    cmd.output().expect("podnet runs in the server namespace")
}

/// One case of the lab.
struct Case {
    text: String,
    mapped: Vec<u16>,
    replies: usize,
    connects: usize,
    direct: SocketAddr,
    relay: SocketAddr,
}

/// Build the lab, send the first datagrams, and run `podnet link p2p`.
fn case(tag: &str, cone: bool) -> Case {
    let lab = Lab::build(tag);
    if cone {
        lab.cone();
    } else {
        lab.symmetric();
    }
    let peer = tcp_peer(&lab);
    let relay = Relay::start(&lab);
    let probe = probe(&lab);
    assert_eq!(probe.sources.len(), 2, "{:?}", probe.sources);
    assert!(
        probe
            .sources
            .iter()
            .all(|source| source.ip().to_string() == ROUTER),
        "the server saw the router as the source: {:?}",
        probe.sources
    );
    let mapped: Vec<u16> = probe.sources.iter().map(SocketAddr::port).collect();
    let direct: SocketAddr = if cone {
        format!("{ROUTER}:{CONE_PORT}").parse().unwrap()
    } else {
        SocketAddr::new(ROUTER.parse::<std::net::IpAddr>().unwrap(), mapped[0])
    };
    let dir = tempfile::tempdir().expect("a directory of the test");
    let password = dir.path().join("turn-password");
    password_file(&password);
    let out = p2p(&lab, direct, peer, relay.addr, &password);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.status.code(), Some(0), "{text}");
    Case {
        text,
        mapped,
        replies: probe.replies,
        connects: relay.connects(),
        direct,
        relay: relay.addr,
    }
}

/// Two real NATs, the direct path from one and the relay from the other.
#[test]
fn three_namespaces_show_direct_first_and_relay_fallback() {
    if !cap_net_admin() {
        test_host::skip_line(
            "CAP_NET_ADMIN is absent; the three-namespace NAT lab needs the capability",
        );
        return;
    }
    let cone = case("c", true);
    assert_eq!(
        cone.mapped,
        vec![CONE_PORT, CONE_PORT],
        "a cone NAT gives one mapping for every destination"
    );
    assert_eq!(cone.replies, 2, "both answers from another source arrived");
    assert!(cone.text.contains("path        direct"), "{}", cone.text);
    assert!(!cone.text.contains("path        relay"), "{}", cone.text);
    assert_eq!(
        cone.connects, 0,
        "a direct path takes no relay: {}",
        cone.text
    );
    println!(
        "cone NAT: client {CLIENT}:{CLIENT_UDP} sent first to {SERVER}:{PROBE_A},{PROBE_B}; \
         mappings {:?}; answers arrived {}/2; direct {}; relay {}; link p2p:\n{}",
        cone.mapped, cone.replies, cone.direct, cone.relay, cone.text
    );

    let symmetric = case("s", false);
    assert_eq!(
        symmetric.mapped.len(),
        2,
        "the server saw two mappings: {:?}",
        symmetric.mapped
    );
    assert_ne!(
        symmetric.mapped[0], symmetric.mapped[1],
        "a symmetric NAT gives one mapping per destination: {:?}",
        symmetric.mapped
    );
    assert_eq!(
        symmetric.replies, 0,
        "an answer to the learned mapping from another source did not arrive"
    );
    assert!(
        symmetric.text.contains("path        relay"),
        "{}",
        symmetric.text
    );
    assert!(
        symmetric.text.contains("direct      refused"),
        "{}",
        symmetric.text
    );
    assert_eq!(
        symmetric.connects, 1,
        "the relay carried one connection: {}",
        symmetric.text
    );
    println!(
        "symmetric NAT: client {CLIENT}:{CLIENT_UDP} sent first to {SERVER}:{PROBE_A},{PROBE_B}; \
         mappings {:?}; answers arrived {}/2; direct {}; relay {}; link p2p:\n{}",
        symmetric.mapped, symmetric.replies, symmetric.direct, symmetric.relay, symmetric.text
    );
}
