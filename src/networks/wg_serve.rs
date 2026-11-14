//! `podnet wg serve` (T-125): a userspace WireGuard responder for a list of
//! peers.
//!
//! The file is a wg-quick file ([`podnet_vnet::wg_quick`]): `[Interface]`
//! with `PrivateKey`, `ListenPort` and `Address` (the IPv4 net of the
//! peers; the server is its first host), and one `[Peer]` for each peer with
//! `PublicKey`, `AllowedIPs` and an optional `PresharedKey`.
//!
//! The handshakes, the cookies and the transport messages are
//! `pod_wg::Responder`. The inner packets go to a guest of the router
//! (`crate::router::guest`) over a socket pair: its stack of `pod-netstack`
//! ends each TCP and UDP flow of a peer, and dials the target through the
//! way of `--egress`. The server makes no TUN device and needs no
//! capability.
//!
//! Cryptokey routing: a packet of a peer must have a source that routes
//! back to that peer (the longest prefix of the `AllowedIPs` of all peers),
//! else it is dropped. A packet to a peer goes to
//! the peer whose `AllowedIPs` has the longest prefix that holds its
//! destination. The net of `Address`, the loopback and the link-local
//! addresses are not targets (the screen of the guest). IPv4 only inside the
//! tunnel.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pod_wg::{Incoming, Peer, Responder};
use podnet_vnet::wg_quick;

use crate::network::{Class, Failure, Stage, Way};
use crate::router::guest::wire::{seqpacket_pair, PacketFd, PACKET_MAX};
use crate::router::guest::{GuestNet, GuestStats, Opened, RouterGuest, WireSpec};

/// The MTU of the tunnel when the file gives none.
const MTU: usize = 1420;
/// More initiations than this in one second put the server under load:
/// each initiation then needs the mac2 of a cookie.
const LOAD: u32 = 64;

/// A peer of the file: its keys and its IPv4 `AllowedIPs`.
struct FilePeer {
    keys: Peer,
    allowed: Vec<(Ipv4Addr, u8)>,
}

/// What the server takes from the file.
pub struct ServeFile {
    private: [u8; 32],
    port: u16,
    net: GuestNet,
    mtu: usize,
    peers: Vec<FilePeer>,
}

impl ServeFile {
    /// The number of peers.
    pub fn peers(&self) -> usize {
        self.peers.len()
    }

    /// The UDP port of `ListenPort` (0: a port of the system).
    pub fn port(&self) -> u16 {
        self.port
    }
}

fn usage(path: &Path, text: String) -> Failure {
    Failure::new(
        Stage::Connect,
        Class::Usage,
        format!("{}: {text}", path.display()),
    )
}

fn mask(prefix: u8) -> u32 {
    u32::MAX.checked_shl(32 - u32::from(prefix)).unwrap_or(0)
}

/// True when `ip` is in `net/prefix`.
fn holds((net, prefix): (Ipv4Addr, u8), ip: Ipv4Addr) -> bool {
    u32::from(ip) & mask(prefix) == u32::from(net) & mask(prefix)
}

/// Read and check the file. A fault is one line that names the file.
pub fn read(path: &Path) -> Result<ServeFile, Failure> {
    let conf = wg_quick::read_file(path).map_err(|e| usage(path, format!("{e:#}")))?;
    check(&conf).map_err(|text| usage(path, text))
}

/// Check a parsed file. A fault is one line.
pub fn check(conf: &wg_quick::WgQuick) -> Result<ServeFile, String> {
    let iface = &conf.interface;
    let port = iface
        .listen_port
        .ok_or("the [Interface] has no ListenPort")?;
    let address = iface
        .addresses
        .iter()
        .find(|c| c.addr.is_ipv4())
        .ok_or("the [Interface] has no IPv4 Address (A.B.C.D/N)")?;
    let net: GuestNet = format!("{}/{}", address.addr, address.prefix)
        .parse()
        .map_err(|e| format!("Address: {e:#}"))?;
    if conf.peers.is_empty() {
        return Err("the file has no [Peer]".into());
    }
    let mut peers: Vec<FilePeer> = Vec::new();
    for (i, p) in conf.peers.iter().enumerate() {
        let n = i + 1;
        if peers.iter().any(|q| q.keys.public == p.public_key) {
            return Err(format!("[Peer] {n}: the PublicKey of an earlier [Peer]"));
        }
        let mut allowed = Vec::new();
        for cidr in &p.allowed_ips {
            let IpAddr::V4(ip) = cidr.addr else { continue };
            if cidr.prefix < net.prefix || !net.contains(ip) {
                return Err(format!(
                    "[Peer] {n}: AllowedIPs {ip}/{} is not inside the net {}/{} of Address",
                    cidr.prefix, net.network, net.prefix
                ));
            }
            allowed.push((ip, cidr.prefix));
        }
        if allowed.is_empty() {
            return Err(format!("[Peer] {n}: no IPv4 AllowedIPs"));
        }
        let keys = Peer {
            public: p.public_key,
            preshared: p.preshared_key.as_ref().map(|k| k.0),
        };
        peers.push(FilePeer { keys, allowed });
    }
    Ok(ServeFile {
        private: iface.private_key.0,
        port,
        net,
        mtu: iface.mtu.map_or(MTU, usize::from).clamp(576, 9000),
        peers,
    })
}

/// The state that the tasks share: the responder and the last address of
/// each peer.
struct State {
    responder: Responder,
    endpoints: Vec<Option<SocketAddr>>,
    /// The initiations of this second: (its start, the count).
    load: (Instant, u32),
}

type Shared = Arc<Mutex<State>>;

fn lock(state: &Shared) -> std::sync::MutexGuard<'_, State> {
    state.lock().unwrap_or_else(|e| e.into_inner())
}

/// A server that runs: its UDP address, and its tasks.
pub struct WgServer {
    local: SocketAddr,
    guest: Arc<RouterGuest>,
    tasks: tokio::task::JoinSet<String>,
}

impl WgServer {
    /// The address of the UDP socket.
    pub fn local_addr(&self) -> SocketAddr {
        self.local
    }

    /// The status document: the address, the net, the way and the counters
    /// of the guest.
    pub fn status(&self) -> serde_json::Value {
        let mut status = self.guest.status();
        status["listen"] = self.local.to_string().into();
        status
    }

    /// Wait until a task of the server ends: the text says why.
    pub async fn wait(&mut self) -> String {
        match self.tasks.join_next().await {
            Some(Ok(why)) => why,
            Some(Err(e)) => format!("a task of the server ended: {e}"),
            None => "the server has no task".into(),
        }
    }
}

/// The peer for a packet to `dst`: the longest prefix of `AllowedIPs`.
fn route(peers: &[Vec<(Ipv4Addr, u8)>], dst: Ipv4Addr) -> Option<usize> {
    let mut best: Option<(usize, u8)> = None;
    for (i, allowed) in peers.iter().enumerate() {
        for &(net, prefix) in allowed {
            if holds((net, prefix), dst) && best.is_none_or(|(_, p)| prefix > p) {
                best = Some((i, prefix));
            }
        }
    }
    best.map(|(i, _)| i)
}

/// The IPv4 source and destination of `packet`.
fn addresses(packet: &[u8]) -> Option<(Ipv4Addr, Ipv4Addr)> {
    if packet.len() < 20 || packet[0] >> 4 != 4 {
        return None;
    }
    let ip = |at: usize| Ipv4Addr::new(packet[at], packet[at + 1], packet[at + 2], packet[at + 3]);
    Some((ip(12), ip(16)))
}

/// The checks of a packet of a peer and of a packet to a peer, for the fuzz
/// target `wg_quick` (`crate::fuzzing`): the admit rule of [`incoming`] and
/// the route of [`outgoing`]. The peer of a route is a peer of `file`.
pub(crate) fn fuzz_packet(file: &ServeFile, packet: &[u8]) {
    let allowed: Vec<Vec<(Ipv4Addr, u8)>> = file.peers.iter().map(|p| p.allowed.clone()).collect();
    if let Some((src, dst)) = addresses(packet) {
        if let Some(peer) = route(&allowed, src) {
            assert!(allowed[peer].iter().any(|&a| holds(a, src)));
        }
        if let Some(peer) = route(&allowed, dst) {
            assert!(peer < allowed.len());
            assert!(allowed[peer].iter().any(|&a| holds(a, dst)));
        }
    }
}

/// Serve `file` on its `ListenPort` of each IPv4 address; each inner flow
/// goes out through `way`.
pub async fn serve(file: ServeFile, label: &str, way: Way) -> anyhow::Result<WgServer> {
    let bind = SocketAddr::from((Ipv4Addr::UNSPECIFIED, file.port));
    serve_on(file, bind, label, way).await
}

/// [`serve`] on the UDP address `bind`.
pub async fn serve_on(
    file: ServeFile,
    bind: SocketAddr,
    label: &str,
    way: Way,
) -> anyhow::Result<WgServer> {
    use anyhow::Context as _;
    let sock = tokio::net::UdpSocket::bind(bind)
        .await
        .with_context(|| format!("ListenPort: udp:{bind}"))
        .map_err(crate::net::bind_fault)?;
    let sock = Arc::new(sock);
    let local = sock.local_addr()?;
    let allowed: Arc<Vec<Vec<(Ipv4Addr, u8)>>> =
        Arc::new(file.peers.iter().map(|p| p.allowed.clone()).collect());
    let keys = file.peers.iter().map(|p| p.keys.clone()).collect();
    let state: Shared = Arc::new(Mutex::new(State {
        responder: Responder::new(file.private, keys),
        endpoints: vec![None; file.peers.len()],
        load: (Instant::now(), 0),
    }));
    let guest = Arc::new(RouterGuest {
        wire: WireSpec::Held(format!("wg:{label}")),
        net: file.net,
        mtu: file.mtu,
        via: way.dial.label(),
        rules: None,
        dial: way.dial.clone(),
        default: way,
        stats: Arc::new(GuestStats::default()),
    });
    let (ours, theirs) = seqpacket_pair()?;
    let wire = Arc::new(PacketFd::new(ours)?);
    let mut tasks = tokio::task::JoinSet::new();
    let served = guest.clone();
    tasks.spawn(async move {
        match served.serve(Opened::Packets(theirs)).await {
            Ok(()) => "the stack of the server ended".into(),
            Err(e) => format!("the stack of the server: {e:#}"),
        }
    });
    tasks.spawn(incoming(
        sock.clone(),
        state.clone(),
        wire.clone(),
        allowed.clone(),
    ));
    tasks.spawn(outgoing(sock.clone(), state.clone(), wire, allowed));
    tasks.spawn(timers(sock, state));
    Ok(WgServer {
        local,
        guest,
        tasks,
    })
}

/// The datagrams of the peers: the handshakes, and the packets to the
/// stack.
async fn incoming(
    sock: Arc<tokio::net::UdpSocket>,
    state: Shared,
    wire: Arc<PacketFd>,
    allowed: Arc<Vec<Vec<(Ipv4Addr, u8)>>>,
) -> String {
    let mut buf = vec![0u8; PACKET_MAX];
    loop {
        let (n, from) = match sock.recv_from(&mut buf).await {
            Ok(got) => got,
            Err(e) => return format!("the UDP socket: {e}"),
        };
        let now = Instant::now();
        let got = {
            let mut state = lock(&state);
            let under_load = state.under_load(&buf[..n], now);
            let got = state.responder.receive(&buf[..n], from, under_load, now);
            // An authenticated message gives the address of its peer.
            if let Incoming::Response { peer, .. }
            | Incoming::Packet { peer, .. }
            | Incoming::Keepalive { peer } = &got
            {
                state.endpoints[*peer] = Some(from);
            }
            got
        };
        match got {
            Incoming::Response { message, .. } | Incoming::CookieReply(message) => {
                let _ = sock.send_to(&message, from).await;
            }
            Incoming::Packet { peer, packet } => {
                // The source must route back to this peer: a source that
                // the AllowedIPs of another peer hold with a longer prefix
                // belongs to that peer (T-342).
                let admitted =
                    addresses(&packet).is_some_and(|(src, _)| route(&allowed, src) == Some(peer));
                if !admitted {
                    tracing::debug!(
                        peer,
                        "a packet with a source that does not route to its peer is dropped"
                    );
                    continue;
                }
                if let Err(e) = wire.send(&packet).await {
                    return format!("the wire to the stack: {e}");
                }
            }
            Incoming::Keepalive { .. } => {}
            Incoming::Dropped(e) => {
                tracing::debug!(%from, error = %e, "a WireGuard message is dropped")
            }
        }
    }
}

impl State {
    /// Count an initiation; true when this second has more than [`LOAD`].
    fn under_load(&mut self, message: &[u8], now: Instant) -> bool {
        if message.first() != Some(&pod_wg::MSG_INITIATION) {
            return false;
        }
        if now.saturating_duration_since(self.load.0) >= Duration::from_secs(1) {
            self.load = (now, 0);
        }
        self.load.1 += 1;
        self.load.1 > LOAD
    }
}

/// The packets of the stack: each to the peer of its destination.
async fn outgoing(
    sock: Arc<tokio::net::UdpSocket>,
    state: Shared,
    wire: Arc<PacketFd>,
    allowed: Arc<Vec<Vec<(Ipv4Addr, u8)>>>,
) -> String {
    let mut buf = vec![0u8; PACKET_MAX];
    loop {
        let n = match wire.recv(&mut buf).await {
            Ok(0) => return "the wire to the stack closed".into(),
            Ok(n) => n,
            Err(e) => return format!("the wire to the stack: {e}"),
        };
        let Some((_, dst)) = addresses(&buf[..n]) else {
            continue;
        };
        let Some(peer) = route(&allowed, dst) else {
            continue;
        };
        let sealed = {
            let mut state = lock(&state);
            let Some(to) = state.endpoints[peer] else {
                continue;
            };
            match state.responder.send(peer, &[&buf[..n]], Instant::now()) {
                Ok(messages) => (to, messages),
                Err(e) => {
                    tracing::debug!(peer, error = %e, "no packet to the peer");
                    continue;
                }
            }
        };
        for message in sealed.1 {
            let _ = sock.send_to(&message, sealed.0).await;
        }
    }
}

/// The timers of the responder: the keepalives, and the end of old
/// sessions.
async fn timers(sock: Arc<tokio::net::UdpSocket>, state: Shared) -> String {
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        let out: Vec<(SocketAddr, Vec<u8>)> = {
            let mut state = lock(&state);
            let due = state.responder.tick(Instant::now());
            due.into_iter()
                .filter_map(|(peer, m)| state.endpoints[peer].map(|to| (to, m)))
                .collect()
        };
        for (to, message) in out {
            let _ = sock.send_to(&message, to).await;
        }
    }
}
