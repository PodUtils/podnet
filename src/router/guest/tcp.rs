//! The TCP path of a guest wire (T-248). The stack of `pod-netstack` ends the
//! handshake before the app sees the flow, so the dial happens at the wire:
//!
//! 1. The pump holds each new SYN, keyed by its tuple. A copy of the SYN (a
//!    retransmit of the guest) is dropped while the dial runs.
//! 2. The dial runs in a task of its own, through the rules and the groups
//!    of the guest. It never blocks the pump or the accept.
//! 3. A dial that succeeds gives the SYN to the stack, and the upstream waits
//!    for its stream. A dial that fails answers the SYN with a RST: the
//!    `connect` of the guest fails.
//! 4. A SYN for a tuple that the router holds (an open flow, a released SYN)
//!    goes to the stack: a guest that forgot a flow gets one challenge ACK,
//!    resets the old flow, and sends its SYN again (1 s).

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pod_netstack::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::time::Instant;

use super::pump::{Pump, SynKey};
use super::{RouterGuest, DIAL_TIMEOUT};
use crate::network::{Class, Connected, Failure, Stage, Target};

/// The time that a released SYN waits for its stream.
const RELEASED_TTL: Duration = Duration::from_secs(30);
/// The time that the listener of an idle port stays.
const PORT_IDLE: Duration = Duration::from_secs(5);

type Dialed = (SynKey, Result<Connected, Failure>);

pub struct State {
    /// The SYNs that wait for their dial.
    pub(super) held: HashMap<SynKey, Vec<u8>>,
    /// The upstreams of released SYNs, until their stream is accepted.
    predialed: HashMap<SynKey, (Connected, Instant)>,
    /// The flows of the guest, until they end.
    open: Arc<Mutex<HashSet<SynKey>>>,
    /// The destination ports with a listener, and the task that takes the
    /// streams of each.
    ports: HashMap<u16, (tokio::task::JoinHandle<()>, Instant)>,
    dials_tx: mpsc::UnboundedSender<Dialed>,
    pub(super) dials: mpsc::UnboundedReceiver<Dialed>,
    accepted_tx: mpsc::UnboundedSender<TcpStream>,
    pub(super) accepted: mpsc::UnboundedReceiver<TcpStream>,
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

impl State {
    pub fn new() -> State {
        let (dials_tx, dials) = mpsc::unbounded_channel();
        let (accepted_tx, accepted) = mpsc::unbounded_channel();
        State {
            held: HashMap::new(),
            predialed: HashMap::new(),
            open: Arc::new(Mutex::new(HashSet::new())),
            ports: HashMap::new(),
            dials_tx,
            dials,
            accepted_tx,
            accepted,
        }
    }

    fn known(&self, key: &SynKey) -> bool {
        self.predialed.contains_key(key) || self.open.lock().is_ok_and(|open| open.contains(key))
    }
}

impl Drop for State {
    fn drop(&mut self) {
        for (task, _) in self.ports.values() {
            task.abort();
        }
    }
}

impl Pump {
    /// A SYN of the guest: hold it and dial, unless the tuple is known.
    pub(super) fn syn(&mut self, key: SynKey, packet: Vec<u8>) {
        let stats = self.guest.stats.clone();
        if self.tcp.known(&key) {
            self.link_in.push(packet);
            return;
        }
        if self.tcp.held.contains_key(&key) {
            // A retransmit while the dial runs: the first SYN is held.
            return;
        }
        if self.tcp_flows() >= super::MAX_FLOWS {
            // The stack drops a SYN past its limit the same way: the guest
            // sends it again.
            stats.add(&stats.tcp_syn_dropped);
            return;
        }
        self.tcp.held.insert(key, packet);
        stats.add(&stats.tcp_dialed);
        let guest = self.guest.clone();
        let done = self.tcp.dials_tx.clone();
        tokio::spawn(async move {
            let target = Target::Addr(SocketAddr::from((key.2, key.3)));
            let result = dial(&guest, &target).await;
            let _ = done.send((key, result));
        });
    }

    /// A dial ended: give the SYN to the stack, or answer it with a RST.
    pub(super) async fn dial_done(&mut self, (key, result): Dialed) {
        let Some(packet) = self.tcp.held.remove(&key) else {
            return;
        };
        let stats = self.guest.stats.clone();
        let listening = result.is_ok() && self.listen_on(key.3).await;
        let result = match result {
            Ok(upstream) if !listening => {
                drop(upstream);
                Err(Failure::new(
                    Stage::Connect,
                    Class::Unavailable,
                    "the stack has no listener for the port",
                ))
            }
            other => other,
        };
        match result {
            Ok(upstream) => {
                self.tcp.predialed.insert(key, (upstream, Instant::now()));
                self.link_in.push(packet);
            }
            Err(e) => {
                stats.add(&stats.tcp_refused);
                tracing::debug!("guest {}:{} to {}:{}: {e}", key.0, key.1, key.2, key.3);
                if let Some(rst) = pod_hold::packet::rst_reply_for_syn(&packet) {
                    self.link_out.push(rst);
                }
            }
        }
    }

    /// Open a listener on any address of `port`, when there is none. False
    /// when the stack refuses.
    async fn listen_on(&mut self, port: u16) -> bool {
        if let Some((_, used)) = self.tcp.ports.get_mut(&port) {
            *used = Instant::now();
            return true;
        }
        let mut listener: TcpListener = match self.stack.listen_any(port).await {
            Ok(listener) => listener,
            Err(e) => {
                tracing::debug!("no listener for port {port}: {e}");
                return false;
            }
        };
        let tx = self.tcp.accepted_tx.clone();
        // The task never waits for a dial: a flow is not lost after its
        // handshake.
        let task = tokio::spawn(async move {
            while let Ok(stream) = listener.accept().await {
                if tx.send(stream).is_err() {
                    break;
                }
            }
        });
        self.tcp.ports.insert(port, (task, Instant::now()));
        true
    }

    /// A stream that the stack accepted: start its flow.
    pub(super) fn accept(&mut self, stream: TcpStream) {
        let (std::net::IpAddr::V4(src), std::net::IpAddr::V4(dst)) =
            (stream.peer_addr().ip(), stream.local_addr().ip())
        else {
            stream.abort();
            return;
        };
        let key = (
            src,
            stream.peer_addr().port(),
            dst,
            stream.local_addr().port(),
        );
        let upstream = self
            .tcp
            .predialed
            .remove(&key)
            .map(|(upstream, _)| upstream);
        if let Ok(mut open) = self.tcp.open.lock() {
            open.insert(key);
        }
        let stats = self.guest.stats.clone();
        stats.tcp_open.fetch_add(1, Ordering::Relaxed);
        let guest = self.guest.clone();
        let open = self.tcp.open.clone();
        tokio::spawn(async move {
            flow(&guest, stream, upstream, key).await;
            if let Ok(mut open) = open.lock() {
                open.remove(&key);
            }
            guest.stats.tcp_open.fetch_sub(1, Ordering::Relaxed);
        });
    }

    pub(super) fn tcp_housekeeping(&mut self) {
        self.tcp
            .predialed
            .retain(|_, (_, at)| at.elapsed() < RELEASED_TTL);
        // Close the listener of each port with no held SYN, no released SYN
        // and no flow, so that the table of the stack does not grow with the
        // ports that the guest used.
        let mut busy: std::collections::HashSet<u16> = self.tcp.held.keys().map(|k| k.3).collect();
        busy.extend(self.tcp.predialed.keys().map(|k| k.3));
        if let Ok(open) = self.tcp.open.lock() {
            busy.extend(open.iter().map(|k| k.3));
        }
        self.tcp.ports.retain(|port, (task, used)| {
            let keep = busy.contains(port) || used.elapsed() < PORT_IDLE;
            if !keep {
                task.abort();
            }
            keep
        });
    }
}

/// Dial `target` through the way of the guest, within [`DIAL_TIMEOUT`].
async fn dial(guest: &RouterGuest, target: &Target) -> Result<Connected, Failure> {
    let deadline = Instant::now() + DIAL_TIMEOUT;
    match tokio::time::timeout_at(deadline, guest.dial.dial(target, deadline)).await {
        Ok(result) => result,
        Err(_) => Err(Failure::new(
            Stage::Connect,
            Class::Timeout,
            format!("{target}: the dial took more than {DIAL_TIMEOUT:?}"),
        )),
    }
}

/// Copy between the stream of the guest and its upstream until both sides
/// end. When the upstream fails, the guest gets a RST. A stream with no
/// released SYN (a tuple that the router already held) dials now: the
/// handshake of the guest has ended, so a failed dial resets the flow.
async fn flow(
    guest: &RouterGuest,
    mut stream: TcpStream,
    upstream: Option<Connected>,
    key: SynKey,
) {
    let upstream = match upstream {
        Some(upstream) => Ok(upstream),
        None => dial(guest, &Target::Addr(SocketAddr::from((key.2, key.3)))).await,
    };
    let mut upstream = match upstream {
        Ok(upstream) => upstream,
        Err(e) => {
            tracing::debug!("guest {}:{} to {}:{}: {e}", key.0, key.1, key.2, key.3);
            guest.stats.add(&guest.stats.tcp_refused);
            stream.abort();
            return;
        }
    };
    match tokio::io::copy_bidirectional(&mut stream, &mut upstream.io).await {
        // The drop of the stream sends each queued byte, then the FIN.
        Ok(_) => {}
        // The guest reset the flow, or the upstream failed: reset the guest.
        Err(_) => stream.abort(),
    }
}
