//! The connection of a transit: this end's hints, then a race of each way
//! to the other end. Each end listens for a direct connection (unless it
//! must not), dials each direct hint of the other end, and after
//! [`RELAY_DELAY`] (at once when no direct hint exists) dials each transit
//! relay of the two ends. Each connection that opens runs the handshake of
//! [`super::transit`]; the sender takes the first one that passes and says
//! `go` on it, and `nevermind` on each later one.
//!
//! The dials go through the way of the transfer (its proxy and resolver).
//! A WebSocket relay goes through `pod-ws`, and its messages carry the
//! same bytes as a TCP relay (`pod_session::ws_link`).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use pod_core::BoxedIo;
use pod_ws::url::WsUrl;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinSet;

use super::messages::{Hint, RelayEnd};
use super::transit::{self, Records, Role};

/// The wait before the relays, when a direct hint exists: transit.py's.
pub const RELAY_DELAY: Duration = Duration::from_secs(2);
/// The bound of each dial and each handshake.
pub const STEP: Duration = Duration::from_secs(20);

/// A transit relay of this end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Relay {
    /// `tcp:HOST:PORT`, as magic-wormhole writes its relay.
    Tcp { host: String, port: u16 },
    /// `ws://` or `wss://`.
    WebSocket(String),
}

impl Relay {
    /// `tcp:HOST:PORT`, `ws://...` or `wss://...`.
    pub fn parse(text: &str) -> Result<Relay, String> {
        if text.starts_with("ws://") || text.starts_with("wss://") {
            WsUrl::parse(text)?;
            return Ok(Relay::WebSocket(text.to_string()));
        }
        let rest = text.strip_prefix("tcp:").ok_or_else(|| {
            format!("transit relay {text:?}: give tcp:HOST:PORT, ws://HOST[:PORT]/PATH or wss://HOST[:PORT]/PATH")
        })?;
        let (host, port) = rest
            .rsplit_once(':')
            .ok_or_else(|| format!("transit relay {text:?}: no port"))?;
        let port = port
            .parse()
            .map_err(|_| format!("transit relay {text:?}: a bad port"))?;
        let host = host.trim_start_matches('[').trim_end_matches(']');
        if host.is_empty() {
            return Err(format!("transit relay {text:?}: no host"));
        }
        Ok(Relay::Tcp {
            host: host.to_string(),
            port,
        })
    }

    /// The relay hint that names it.
    pub fn hint(&self) -> Hint {
        let end = match self {
            Relay::Tcp { host, port } => RelayEnd::Tcp {
                host: host.clone(),
                port: *port,
            },
            Relay::WebSocket(url) => RelayEnd::WebSocket { url: url.clone() },
        };
        Hint::Relay {
            name: None,
            ends: vec![end],
        }
    }
}

/// Where this end listens for a direct connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Listen {
    /// No listener: the other end must reach this one through a relay, or
    /// this end dials the other.
    No,
    /// Each address of the host, on a free port: the hints are the
    /// addresses of the interfaces, not the loopback.
    Any,
    /// One address: the hint is that address (a loopback address too).
    At(SocketAddr),
}

/// This end's listener and hints.
pub struct Ours {
    listener: Option<TcpListener>,
    pub hints: Vec<Hint>,
}

/// Listen as `listen` says, and give the hints of the listener and of
/// `relays`.
pub async fn prepare(listen: &Listen, relays: &[Relay]) -> std::io::Result<Ours> {
    let (listener, mut hints) = match listen {
        Listen::No => (None, Vec::new()),
        Listen::Any => {
            // A host that refuses a bind of AF_INET (`docs/target-environment.md`)
            // has no listener: the other end or a relay carries the transfer.
            let any = SocketAddr::from((Ipv6Addr::UNSPECIFIED, 0));
            let listener = match TcpListener::bind(any).await {
                Ok(listener) => Some(listener),
                Err(_) => TcpListener::bind(SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)))
                    .await
                    .ok(),
            };
            match listener {
                Some(listener) => {
                    let port = listener.local_addr()?.port();
                    let hints = addresses().into_iter().map(|ip| direct(ip, port)).collect();
                    (Some(listener), hints)
                }
                None => {
                    tracing::info!(
                        "transfer: no listener for a direct connection (the bind was refused)"
                    );
                    (None, Vec::new())
                }
            }
        }
        Listen::At(at) => {
            let listener = TcpListener::bind(at).await?;
            let bound = listener.local_addr()?;
            let hints = if bound.ip().is_unspecified() {
                addresses()
                    .into_iter()
                    .map(|ip| direct(ip, bound.port()))
                    .collect()
            } else {
                vec![direct(bound.ip(), bound.port())]
            };
            (Some(listener), hints)
        }
    };
    hints.extend(relays.iter().map(Relay::hint));
    Ok(Ours { listener, hints })
}

fn direct(ip: IpAddr, port: u16) -> Hint {
    Hint::Direct {
        host: ip.to_string(),
        port,
    }
}

/// The addresses of the interfaces of the host: no loopback and no
/// link-local address, which another host cannot dial.
pub fn addresses() -> Vec<IpAddr> {
    let mut out = Vec::new();
    let mut list: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: getifaddrs fills `list` with a list that freeifaddrs frees;
    // each node is read while the list lives.
    unsafe {
        if libc::getifaddrs(&mut list) != 0 {
            return out;
        }
        let mut at = list;
        while !at.is_null() {
            let addr = (*at).ifa_addr;
            if !addr.is_null() {
                match i32::from((*addr).sa_family) {
                    libc::AF_INET => {
                        let v4 = &*(addr as *const libc::sockaddr_in);
                        out.push(IpAddr::V4(Ipv4Addr::from(u32::from_be(v4.sin_addr.s_addr))));
                    }
                    libc::AF_INET6 => {
                        let v6 = &*(addr as *const libc::sockaddr_in6);
                        out.push(IpAddr::V6(Ipv6Addr::from(v6.sin6_addr.s6_addr)));
                    }
                    _ => {}
                }
            }
            at = (*at).ifa_next;
        }
        libc::freeifaddrs(list);
    }
    out.retain(|ip| !ip.is_loopback() && !link_local(ip));
    out.dedup();
    out
}

fn link_local(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_link_local(),
        IpAddr::V6(v6) => (v6.segments()[0] & 0xffc0) == 0xfe80,
    }
}

/// How the transit reaches out: the way of the transfer.
#[derive(Clone)]
pub struct Way {
    pub way: pod_ws::Way,
    pub trust: pod_ws::Trust,
}

/// The connection that won, and what it is (`direct to HOST:PORT`, `relay
/// tcp:HOST:PORT`), within `wait`.
pub async fn connect(
    role: Role,
    key: &[u8; 32],
    ours: Ours,
    theirs: &[Hint],
    way: &Way,
    wait: Duration,
) -> Result<(Records<BoxedIo>, String), String> {
    let (tx, mut rx) = mpsc::channel::<(BoxedIo, String)>(4);
    let won = Arc::new(Mutex::new(false));
    let mut tasks = JoinSet::new();
    let key = *key;
    if let Some(listener) = ours.listener {
        let (tx, won) = (tx.clone(), won.clone());
        tasks.spawn(async move {
            let mut each = JoinSet::new();
            while let Ok((tcp, from)) = listener.accept().await {
                let (tx, won) = (tx.clone(), won.clone());
                each.spawn(negotiate(
                    Box::new(tcp),
                    format!("direct from {from}"),
                    role,
                    key,
                    won,
                    tx,
                ));
            }
        });
    }
    let mut relays: Vec<RelayEnd> = Vec::new();
    let mut directs = 0;
    for hint in ours.hints.iter().chain(theirs) {
        match hint {
            Hint::Relay { ends, .. } => relays.extend(ends.iter().cloned()),
            Hint::Direct { .. } => directs += 1,
        }
    }
    for hint in theirs {
        if let Hint::Direct { host, port } = hint {
            let (tx, won, way, host, port) =
                (tx.clone(), won.clone(), way.clone(), host.clone(), *port);
            tasks.spawn(async move {
                if let Ok(connected) = way.way.tcp(&host, port, STEP).await {
                    negotiate(
                        connected.io,
                        format!("direct to {host}:{port}"),
                        role,
                        key,
                        won,
                        tx,
                    )
                    .await;
                }
            });
        }
    }
    let delay = if directs > 0 {
        RELAY_DELAY
    } else {
        Duration::ZERO
    };
    let mut seen = Vec::new();
    for end in relays {
        if seen.contains(&end) {
            continue;
        }
        seen.push(end.clone());
        let (tx, won, way) = (tx.clone(), won.clone(), way.clone());
        tasks.spawn(async move {
            tokio::time::sleep(delay).await;
            let Some((mut io, what)) = through_relay(&end, &key, &way).await else {
                return;
            };
            let _ = io.flush().await;
            negotiate(io, what, role, key, won, tx).await;
        });
    }
    drop(tx);
    let first = tokio::time::timeout(wait, rx.recv()).await;
    tasks.abort_all();
    match first {
        Ok(Some((io, what))) => Ok((Records::new(io, &key, role), what)),
        Ok(None) => {
            Err("no way to the other end worked: no direct hint and no relay answered".into())
        }
        Err(_) => Err(format!(
            "no way to the other end worked within {} s",
            wait.as_secs()
        )),
    }
}

/// A connection to the relay `end` that the relay has paired: its
/// handshake sent and its `ok` read.
async fn through_relay(end: &RelayEnd, key: &[u8; 32], way: &Way) -> Option<(BoxedIo, String)> {
    let (mut io, what): (BoxedIo, String) = match end {
        RelayEnd::Tcp { host, port } => {
            let connected = way.way.tcp(host, *port, STEP).await.ok()?;
            (connected.io, format!("relay tcp:{host}:{port}"))
        }
        RelayEnd::WebSocket { url } => {
            let parsed = WsUrl::parse(url).ok()?;
            let session = pod_ws::url::dial(&parsed, &way.way, &way.trust, STEP, None)
                .await
                .ok()?;
            (
                Box::new(pod_session::ws_link::Link::start(session, Vec::new())),
                format!("relay {}", parsed.shown()),
            )
        }
    };
    let mut side = [0u8; 8];
    getrandom::fill(&mut side).ok()?;
    io.write_all(&transit::relay_handshake(key, &pod_wormhole::hex(&side)))
        .await
        .ok()?;
    io.flush().await.ok()?;
    let ok = tokio::time::timeout(STEP, transit::expect(&mut io, b"ok\n")).await;
    matches!(ok, Ok(Ok(true))).then_some((io, what))
}

/// The handshake of the ends on `io`; the connection goes to `tx` when it
/// wins.
async fn negotiate(
    mut io: BoxedIo,
    what: String,
    role: Role,
    key: [u8; 32],
    won: Arc<Mutex<bool>>,
    tx: mpsc::Sender<(BoxedIo, String)>,
) {
    let theirs = match role {
        Role::Sender => Role::Receiver,
        Role::Receiver => Role::Sender,
    };
    let steps = async {
        io.write_all(&transit::handshake(&key, role)).await.ok()?;
        io.flush().await.ok()?;
        if !transit::expect(&mut io, &transit::handshake(&key, theirs))
            .await
            .ok()?
        {
            return None;
        }
        match role {
            Role::Sender => {
                let mut first = won.lock().await;
                if *first {
                    let _ = io.write_all(b"nevermind\n").await;
                    return None;
                }
                *first = true;
                io.write_all(b"go\n").await.ok()?;
                io.flush().await.ok()?;
                Some(())
            }
            Role::Receiver => {
                let mut go = [0u8; 3];
                io.read_exact(&mut go).await.ok()?;
                (&go == b"go\n").then_some(())
            }
        }
    };
    if let Ok(Some(())) = tokio::time::timeout(STEP, steps).await {
        let _ = tx.send((io, what)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wormhole_transfer_a_relay_is_tcp_or_a_websocket() {
        assert_eq!(
            Relay::parse("tcp:transit.example:4001").unwrap(),
            Relay::Tcp {
                host: "transit.example".into(),
                port: 4001
            }
        );
        assert_eq!(
            Relay::parse("tcp:[::1]:4001").unwrap(),
            Relay::Tcp {
                host: "::1".into(),
                port: 4001
            }
        );
        assert_eq!(
            Relay::parse("wss://r.example/").unwrap(),
            Relay::WebSocket("wss://r.example/".into())
        );
        assert!(Relay::parse("transit.example:4001").is_err());
        assert!(Relay::parse("tcp:transit.example").is_err());
        assert!(Relay::parse("tcp::4001").is_err());
    }

    #[test]
    fn wormhole_transfer_no_hint_names_the_loopback() {
        assert!(addresses().iter().all(|ip| !ip.is_loopback()));
    }
}
