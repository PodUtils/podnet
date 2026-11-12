//! WireGuard as a network: `wg:FILE` (T-117).
//!
//! `FILE` is a wg-quick file with one peer ([`podnet_vnet::wg_quick`]). The
//! network does the handshake with `pod-wg`, puts the tunnel on a stack of
//! `pod-netstack`, and dials TCP and UDP from the address of the file. It
//! makes no TUN device and needs no capability.
//!
//! The UDP of the tunnel goes out as datagrams of a carrier: the UDP of
//! this host, or the datagrams of the way below the network (`Ctx::below`,
//! for example a layer of a `Chain` that carries UDP). The handshake is the
//! probe of the carrier: a path with no UDP gives no answer, and the error
//! says so.

use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime};

use pod_core::datagrams::{recv_from, send_to};
use pod_netstack::{NetStack, StackConfig, Subnet};
use pod_wg::{Cookie, Keys, Received, Stamps, Tunnel, REKEY_TIMEOUT};
use podnet_vnet::wg_quick;
use tokio::time::Instant as Deadline;

use crate::network::{
    BoxFuture, Class, Connected, Datagrams, Dial, Failure, Link, Stage, SystemDatagrams, Target,
};

/// The MTU of the tunnel when the file gives none.
const MTU: usize = 1420;
/// The handshake tries, [`REKEY_TIMEOUT`] apart.
const TRIES: u32 = 3;

fn fail(class: Class, text: String) -> Failure {
    Failure::new(Stage::Connect, class, text)
}

/// What the network takes from the file.
struct Config {
    keys: Keys,
    endpoint: (String, u16),
    v4: Option<std::net::Ipv4Addr>,
    v6: Option<std::net::Ipv6Addr>,
    mtu: usize,
}

/// Read and check the file. A fault is one line that names the file.
fn read(path: &Path) -> Result<Config, Failure> {
    let usage = |e: String| fail(Class::Usage, format!("wg:{}: {e}", path.display()));
    let conf = wg_quick::read_file(path).map_err(|e| usage(format!("{e:#}")))?;
    let peer = conf.single_peer().map_err(|e| usage(format!("{e:#}")))?;
    if peer.preshared_key.is_some() {
        return Err(usage(
            "the peer has a PresharedKey, and podnet does not take it yet: remove it from both ends".into(),
        ));
    }
    let endpoint = peer
        .endpoint
        .as_ref()
        .ok_or_else(|| usage("the peer has no Endpoint".into()))?;
    let (mut v4, mut v6) = (None, None);
    for cidr in &conf.interface.addresses {
        match cidr.addr {
            IpAddr::V4(a) => v4 = v4.or(Some(a)),
            IpAddr::V6(a) => v6 = v6.or(Some(a)),
        }
    }
    if v4.is_none() && v6.is_none() {
        return Err(usage("the [Interface] has no Address".into()));
    }
    Ok(Config {
        keys: Keys {
            private: conf.interface.private_key.0,
            peer_public: peer.public_key,
            reserved: peer.reserved,
        },
        endpoint: (endpoint.host.clone(), endpoint.port),
        v4,
        v6,
        mtu: conf.interface.mtu.map_or(MTU, usize::from).clamp(576, 9000),
    })
}

/// Check the file of a spec (the parse of the registry).
pub(super) fn check(path: &Path) -> Result<(), Failure> {
    read(path).map(drop)
}

/// The UDP of the tunnel: a carrier and the address of the peer.
struct Sock {
    carrier: Arc<dyn Datagrams>,
    peer: SocketAddr,
}

impl Sock {
    async fn send(&self, message: &[u8]) -> std::io::Result<()> {
        send_to(&*self.carrier, message, self.peer).await
    }

    /// The next datagram from the peer.
    async fn recv(&self, buf: &mut [u8]) -> std::io::Result<usize> {
        loop {
            let (n, from) = recv_from(&*self.carrier, buf).await?;
            if from == self.peer {
                return Ok(n);
            }
        }
    }
}

type Shared = Arc<Mutex<Tunnel>>;

fn lock(state: &Shared) -> std::sync::MutexGuard<'_, Tunnel> {
    state.lock().unwrap_or_else(|e| e.into_inner())
}

/// The handshake: three tries. A cookie reply gives its mac2 to the next try.
async fn handshake(keys: &Keys, sock: &Sock) -> Result<Tunnel, Failure> {
    let mut buf = vec![0u8; 2048];
    let mut stamps = Stamps::new(SystemTime::now(), Instant::now());
    let mut cookie: Option<Cookie> = None;
    let mut last = None;
    for _ in 0..TRIES {
        let mut init = pod_wg::initiate_from(
            keys,
            &mut pod_core::entropy::OsEntropy,
            stamps.next(Instant::now()),
        )
        .map_err(|e| fail(Class::Unavailable, format!("the WireGuard handshake: {e}")))?;
        if let Some(cookie) = &cookie {
            init.add_mac2(cookie, Instant::now());
        }
        sock.send(&init.message)
            .await
            .map_err(|e| Failure::io(Stage::Connect, e))?;
        let end = Deadline::now() + REKEY_TIMEOUT;
        while let Ok(read) = tokio::time::timeout_at(end, sock.recv(&mut buf)).await {
            let n = read.map_err(|e| Failure::io(Stage::Connect, e))?;
            match buf[..n].first() {
                Some(&pod_wg::MSG_RESPONSE) => match init.finish(keys, &buf[..n], Instant::now()) {
                    Ok(session) => {
                        return Ok(Tunnel::new(keys.clone(), session, stamps, Instant::now()))
                    }
                    Err(e) => last = Some(format!("the response does not verify: {e}")),
                },
                Some(&pod_wg::MSG_COOKIE) => {
                    match Cookie::open(keys, &init, &buf[..n], Instant::now()) {
                        Ok(fresh) => {
                            cookie = Some(fresh);
                            last = Some("the peer is under load (a cookie reply)".into());
                        }
                        Err(e) => last = Some(format!("the cookie reply does not verify: {e}")),
                    }
                }
                _ => {}
            }
        }
    }
    Err(fail(
        Class::Timeout,
        last.unwrap_or_else(|| {
            format!(
                "no WireGuard handshake answer from {} in {} s: the UDP path may be closed (a host with no UDP needs a carrier that carries UDP)",
                sock.peer,
                u64::from(TRIES) * REKEY_TIMEOUT.as_secs()
            )
        }),
    ))
}

/// The tunnel with its stack: the dialer of the network.
pub struct WgDialer {
    label: String,
    stack: NetStack,
    local: IpAddr,
    dead: Arc<AtomicBool>,
}

impl WgDialer {
    fn alive(&self) -> Result<(), Failure> {
        if self.dead.load(Ordering::Relaxed) {
            return Err(fail(
                Class::Unavailable,
                format!(
                    "{}: the WireGuard path is dead (no handshake answer)",
                    self.label
                ),
            ));
        }
        Ok(())
    }
}

impl Dial for WgDialer {
    fn label(&self) -> String {
        self.label.clone()
    }

    fn dial<'a>(
        &'a self,
        target: &'a Target,
        deadline: Deadline,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(async move {
            self.alive()?;
            let addr = match target {
                Target::Addr(addr) => *addr,
                Target::Name { host, .. } => {
                    return Err(Failure::new(
                        Stage::Dns,
                        Class::Unsupported,
                        format!("{host}: the WireGuard network takes the IP address of a target, and it does not resolve names"),
                    ))
                }
            };
            let run = async {
                let mut stream = self.stack.connect(addr).await?;
                stream.established().await?;
                Ok::<_, std::io::Error>(stream)
            };
            let stream = tokio::time::timeout_at(deadline, run)
                .await
                .map_err(|_| {
                    fail(
                        Class::Timeout,
                        format!("{addr}: no answer through {}", self.label),
                    )
                })?
                .map_err(|e| Failure::io(Stage::Connect, e))?;
            Ok(Connected {
                local: Some(stream.local_addr()),
                peer: Some(addr),
                io: Box::new(stream),
            })
        })
    }

    /// A UDP socket of the stack, on the address of the file.
    fn datagrams<'a>(&'a self) -> BoxFuture<'a, Result<Arc<dyn Datagrams>, Failure>> {
        Box::pin(async move {
            self.alive()?;
            let socket = self
                .stack
                .bind_udp(SocketAddr::new(self.local, 0))
                .await
                .map_err(|e| Failure::io(Stage::Connect, e))?;
            Ok(Arc::new(socket.into_datagrams()) as Arc<dyn Datagrams>)
        })
    }
}

/// The endpoint as an address: an IP literal, else the system resolver.
async fn resolve(host: &str, port: u16) -> Result<SocketAddr, Failure> {
    if let Ok(ip) = host.trim_matches(['[', ']']).parse::<IpAddr>() {
        return Ok(SocketAddr::new(ip, port));
    }
    let mut found = tokio::net::lookup_host((host, port))
        .await
        .map_err(|e| Failure::new(Stage::Dns, Class::Unavailable, format!("{host}: {e}")))?;
    found.next().ok_or_else(|| {
        Failure::new(
            Stage::Dns,
            Class::Unavailable,
            format!("{host} has no address"),
        )
    })
}

/// The carrier: the datagrams of the way below, else a UDP socket of this host.
async fn carrier(
    below: Option<&Arc<dyn Dial>>,
    peer: SocketAddr,
) -> Result<Arc<dyn Datagrams>, Failure> {
    if let Some(below) = below {
        return below.datagrams().await;
    }
    let any: SocketAddr = if peer.is_ipv4() {
        (std::net::Ipv4Addr::UNSPECIFIED, 0).into()
    } else {
        (std::net::Ipv6Addr::UNSPECIFIED, 0).into()
    };
    let socket = SystemDatagrams::bind(any)
        .await
        .map_err(|e| Failure::io(Stage::Connect, e))?;
    Ok(Arc::new(socket))
}

/// Bring the network up: the handshake, the stack, and the tasks of the tunnel.
pub(super) async fn up(
    path: &Path,
    label: String,
    below: Option<&Arc<dyn Dial>>,
) -> Result<(Arc<WgDialer>, Link), Failure> {
    let config = read(path)?;
    let peer = resolve(&config.endpoint.0, config.endpoint.1).await?;
    let sock = Arc::new(Sock {
        carrier: carrier(below, peer).await?,
        peer,
    });
    let tunnel = handshake(&config.keys, &sock)
        .await
        .map_err(|e| fail(e.class, format!("{label}: {}", e.text())))?;
    tracing::debug!(%peer, "the WireGuard handshake is done");

    // The stack owns the address of the file. Its gateway is 0.0.0.1 and ::1,
    // which no target has, so no target of the peer is a local address.
    let local_v4 = config.v4.unwrap_or(std::net::Ipv4Addr::new(10, 66, 0, 2));
    let mut stack_config = StackConfig {
        subnet: Subnet {
            v4: std::net::Ipv4Addr::UNSPECIFIED,
            v4_prefix: 0,
            v6: std::net::Ipv6Addr::UNSPECIFIED,
            v6_prefix: 0,
        },
        local_v4,
        mtu: config.mtu,
        ..StackConfig::default()
    };
    if let Some(v6) = config.v6 {
        stack_config.local_v6 = v6;
    }
    let local = if config.v4.is_some() {
        IpAddr::V4(local_v4)
    } else {
        IpAddr::V6(stack_config.local_v6)
    };
    let stack = NetStack::over_link(stack_config);
    let state: Shared = Arc::new(Mutex::new(tunnel));
    let dead = Arc::new(AtomicBool::new(false));
    let tasks = tasks(&stack, &sock, &state, &dead);
    let dialer = Arc::new(WgDialer {
        label: label.clone(),
        stack,
        local,
        dead,
    });
    let status = serde_json::json!({
        "scheme": "wg",
        "spec": label,
        "peer": peer.to_string(),
        "carrier": below.map_or("udp".to_string(), |b| b.label()),
        "address": local.to_string(),
    });
    let link = Link::new(Some(crate::network::Way::destination(dialer.clone())))
        .with_status(status)
        .on_close(Box::new(move || {
            Box::pin(async move {
                for task in tasks {
                    task.abort();
                }
            })
        }));
    Ok((dialer, link))
}

/// The three tasks of the tunnel: packets in, packets out, and the timers.
fn tasks(
    stack: &NetStack,
    sock: &Arc<Sock>,
    state: &Shared,
    dead: &Arc<AtomicBool>,
) -> Vec<tokio::task::JoinHandle<()>> {
    let (link_in, link_out) = (stack.link_in(), stack.link_out());
    let (rx_sock, rx_state) = (sock.clone(), state.clone());
    let incoming = tokio::spawn(async move {
        let mut buf = vec![0u8; 2048];
        loop {
            let Ok(n) = rx_sock.recv(&mut buf).await else {
                return;
            };
            let received = lock(&rx_state).receive(&buf[..n], Instant::now());
            match received {
                Received::Packet(packet) => {
                    link_in.push(packet.to_vec());
                }
                Received::Dropped(e) => {
                    tracing::debug!(error = %e, "a WireGuard message is dropped")
                }
                _ => {}
            }
        }
    });
    let (tx_sock, tx_state) = (sock.clone(), state.clone());
    let outgoing = tokio::spawn(async move {
        loop {
            let packet = link_out.recv().await;
            let out = lock(&tx_state).send(&[packet], Instant::now());
            let Ok(out) = out else { continue };
            for message in out.handshake.into_iter().chain(out.messages) {
                let _ = tx_sock.send(&message).await;
            }
        }
    });
    let (tick_sock, tick_state, dead) = (sock.clone(), state.clone(), dead.clone());
    let timers = tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(1));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            let messages = lock(&tick_state).tick(Instant::now());
            match messages {
                Ok(messages) => {
                    for message in messages {
                        let _ = tick_sock.send(&message).await;
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "the WireGuard path is dead");
                    dead.store(true, Ordering::Relaxed);
                    return;
                }
            }
        }
    });
    vec![incoming, outgoing, timers]
}

/// The file of a spec `wg:FILE`.
pub(super) fn file_of(spec: &str) -> Option<PathBuf> {
    spec.strip_prefix("wg:").map(PathBuf::from)
}
