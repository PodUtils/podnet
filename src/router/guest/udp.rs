//! The UDP path of a guest wire (T-248). The stack of `pod-netstack` takes
//! the datagrams of the guest with one socket for each destination port
//! (`bind_udp_any`). Since v0.28.0 the stack has a backlog for each socket
//! (`udp_packets`, `udp_payload`), counts a datagram that it loses
//! (`NetStats::udp_rx_dropped`, `udp_tx_dropped`, `udp_oversize`; the guest
//! counters `stack_udp_*`), and refuses a send over `max_datagram` with
//! `InvalidInput`. A socket serves every flow of its port, so the router keeps
//! its own queue for each flow: a slow flow must not hold the others.
//!
//! * A task for each port reads the socket at once. It never waits for a
//!   flow.
//! * A flow is a guest address, a destination address and a socket from
//!   [`crate::network::Dial::datagrams`] of the way that the rules choose. A
//!   flow takes one member of a group and never moves.
//! * A flow has a queue of [`QUEUE`] datagrams. A datagram for a full queue is
//!   dropped and counted (`udp_queue_dropped`).
//! * A datagram to the guest over `UdpSocket::max_datagram` is dropped and
//!   counted (`udp_oversize`). With the feature `ipv4-fragmentation` of
//!   `pod-netstack` (`Cargo.toml`) the limit is 65507 bytes.
//! * A flow ends after [`IDLE`] with no datagram. A flow whose way fails or
//!   refuses keeps its place for [`FAIL_HOLD`] and drops (and counts) its
//!   datagrams.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pod_netstack::UdpSocket;
use tokio::sync::mpsc;

use super::pump::Pump;
use super::share::{Held, Kind};
use super::RouterGuest;
use crate::network::{recv_from, send_to, Datagrams, Failure, Target};

/// The datagrams that wait for the socket of one flow.
pub const QUEUE: usize = 32;
/// The datagrams of one port that wait for the stack.
pub const REPLY_QUEUE: usize = 64;
/// The UDP flows of a guest at one time by default
/// ([`super::FlowLimits`]).
pub const MAX_FLOWS: usize = 512;
/// The ports of a guest with a socket at one time.
pub const MAX_PORTS: usize = 256;
/// The time with no datagram after which a flow ends.
pub const IDLE: Duration = Duration::from_secs(60);
/// The time that a flow with a failed way drops its datagrams.
pub const FAIL_HOLD: Duration = Duration::from_secs(5);

type Key = (SocketAddr, SocketAddr);
type Flows = Arc<Mutex<HashMap<Key, mpsc::Sender<Vec<u8>>>>>;

/// A datagram for the guest.
struct Reply {
    data: Vec<u8>,
    guest: SocketAddr,
    source: IpAddr,
}

struct Port {
    task: tokio::task::JoinHandle<()>,
    /// The time of the last datagram, in ms since `epoch`.
    last: Arc<AtomicU64>,
}

pub struct State {
    ports: HashMap<u16, Port>,
    flows: Flows,
    epoch: Instant,
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

impl State {
    pub fn new() -> State {
        State {
            ports: HashMap::new(),
            flows: Arc::new(Mutex::new(HashMap::new())),
            epoch: Instant::now(),
        }
    }

    fn now_ms(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64
    }
}

impl Drop for State {
    fn drop(&mut self) {
        for port in self.ports.values() {
            port.task.abort();
        }
    }
}

impl Pump {
    /// Make sure that a socket takes the datagrams for `port`. False when
    /// the datagram cannot go on.
    pub(super) async fn ensure_udp(&mut self, port: u16) -> bool {
        let stats = self.guest.stats.clone();
        if let Some(open) = self.udp.ports.get(&port) {
            open.last.store(self.udp.now_ms(), Ordering::Relaxed);
            return true;
        }
        if self.udp.ports.len() >= MAX_PORTS {
            stats.add(&stats.udp_refused);
            return false;
        }
        let socket = match self.stack.bind_udp_any(port).await {
            Ok(socket) => socket,
            Err(e) => {
                tracing::debug!("no UDP socket for port {port}: {e}");
                stats.add(&stats.dropped);
                return false;
            }
        };
        let (replies, inbox) = mpsc::channel(REPLY_QUEUE);
        let last = Arc::new(AtomicU64::new(self.udp.now_ms()));
        let ctx = Ctx {
            guest: self.guest.clone(),
            flows: self.udp.flows.clone(),
            replies: replies.clone(),
            epoch: self.udp.epoch,
        };
        let task = tokio::spawn(port_task(socket, inbox, ctx, last.clone()));
        self.udp.ports.insert(port, Port { task, last });
        true
    }

    /// Close the socket of each port with no flow and no datagram for
    /// [`IDLE`].
    pub(super) fn udp_housekeeping(&mut self) {
        if self.udp.ports.is_empty() {
            return;
        }
        let now = self.udp.now_ms();
        let used: std::collections::HashSet<u16> = self
            .udp
            .flows
            .lock()
            .map(|flows| flows.keys().map(|(_, dst)| dst.port()).collect())
            .unwrap_or_default();
        self.udp.ports.retain(|port, open| {
            let idle =
                now.saturating_sub(open.last.load(Ordering::Relaxed)) >= IDLE.as_millis() as u64;
            let keep = used.contains(port) || !idle;
            if !keep {
                open.task.abort();
            }
            keep
        });
    }
}

/// What the tasks of a port and of its flows share.
#[derive(Clone)]
struct Ctx {
    guest: Arc<RouterGuest>,
    flows: Flows,
    replies: mpsc::Sender<Reply>,
    epoch: Instant,
}

impl Ctx {
    fn now_ms(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64
    }
}

/// Read the socket of `port` at once, and send the replies of its flows.
async fn port_task(
    mut socket: UdpSocket,
    mut inbox: mpsc::Receiver<Reply>,
    ctx: Ctx,
    last: Arc<AtomicU64>,
) {
    let mut buf = vec![0u8; 65536];
    loop {
        tokio::select! {
            got = socket.recv_from_to(&mut buf) => {
                let Ok((n, from, to)) = got else { return };
                last.store(ctx.now_ms(), Ordering::Relaxed);
                deliver(&ctx, from, to, buf[..n].to_vec());
            }
            Some(reply) = inbox.recv() => {
                last.store(ctx.now_ms(), Ordering::Relaxed);
                let stats = &ctx.guest.stats;
                if reply.data.len() > socket.max_datagram(reply.guest.ip()) {
                    stats.add(&stats.udp_oversize);
                    continue;
                }
                match socket.send_to_from(&reply.data, reply.guest, reply.source).await {
                    Ok(_) => stats.add(&stats.udp_down),
                    Err(e) => tracing::debug!("a datagram to the guest: {e}"),
                }
            }
        }
    }
}

/// Give a datagram of the guest to its flow. The flow is made when it is
/// new. This function never waits.
fn deliver(ctx: &Ctx, from: SocketAddr, to: SocketAddr, data: Vec<u8>) {
    let stats = &ctx.guest.stats;
    let key = (from, to);
    let mut flows = ctx.flows.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(queue) = flows.get(&key) {
        match queue.try_send(data) {
            Ok(()) => return,
            Err(mpsc::error::TrySendError::Full(_)) => {
                stats.add(&stats.udp_queue_dropped);
                return;
            }
            // The flow ended: make it again below.
            Err(mpsc::error::TrySendError::Closed(returned)) => {
                flows.remove(&key);
                return deliver_new(ctx, &mut flows, key, returned);
            }
        }
    }
    deliver_new(ctx, &mut flows, key, data);
}

fn deliver_new(
    ctx: &Ctx,
    flows: &mut HashMap<Key, mpsc::Sender<Vec<u8>>>,
    key: Key,
    data: Vec<u8>,
) {
    let stats = &ctx.guest.stats;
    if flows.len() >= ctx.guest.shares.limits().udp {
        stats.add(&stats.udp_refused);
        return;
    }
    let IpAddr::V4(src) = key.0.ip() else {
        stats.add(&stats.dropped);
        return;
    };
    // One source group cannot take the flows of the others (T-345).
    let Some(place) = ctx.guest.shares.take(src, Kind::Udp, stats) else {
        return;
    };
    let (queue, inbox) = mpsc::channel(QUEUE);
    let _ = queue.try_send(data);
    flows.insert(key, queue);
    stats.add(&stats.udp_flows);
    tokio::spawn(flow(ctx.clone(), key, inbox, place));
}

/// The way and the socket of a flow to `dst`.
async fn open(guest: &RouterGuest, dst: SocketAddr) -> Result<Arc<dyn Datagrams>, Failure> {
    guest.way_for(&Target::Addr(dst))?.dial.datagrams().await
}

/// One flow: send its datagrams out, and bring the answers of `dst` back.
async fn flow(ctx: Ctx, key: Key, mut inbox: mpsc::Receiver<Vec<u8>>, place: Held) {
    let (guest, dst) = key;
    let stats = ctx.guest.stats.clone();
    match open(&ctx.guest, dst).await {
        Err(e) => {
            tracing::debug!("UDP flow {guest} to {dst}: {e}");
            let end = tokio::time::Instant::now() + FAIL_HOLD;
            while let Ok(Some(_)) = tokio::time::timeout_at(end, inbox.recv()).await {
                stats.add(&stats.udp_refused);
            }
        }
        Ok(socket) => {
            let last = Arc::new(AtomicU64::new(ctx.now_ms()));
            let down = tokio::spawn(answers(
                ctx.clone(),
                socket.clone(),
                guest,
                dst,
                last.clone(),
            ));
            loop {
                match tokio::time::timeout(IDLE, inbox.recv()).await {
                    Ok(Some(data)) => {
                        last.store(ctx.now_ms(), Ordering::Relaxed);
                        match send_to(&*socket, &data, dst).await {
                            Ok(()) => stats.add(&stats.udp_up),
                            Err(e) => tracing::debug!("UDP flow {guest} to {dst}: {e}"),
                        }
                    }
                    Ok(None) => break,
                    Err(_) => {
                        let idle = ctx.now_ms().saturating_sub(last.load(Ordering::Relaxed));
                        if idle >= IDLE.as_millis() as u64 {
                            break;
                        }
                    }
                }
            }
            down.abort();
        }
    }
    if let Ok(mut flows) = ctx.flows.lock() {
        flows.remove(&key);
    }
    drop(place);
}

/// The datagrams that come back from `dst`, for the guest. An answer from
/// another address is dropped.
async fn answers(
    ctx: Ctx,
    socket: Arc<dyn Datagrams>,
    guest: SocketAddr,
    dst: SocketAddr,
    last: Arc<AtomicU64>,
) {
    let stats = ctx.guest.stats.clone();
    let mut buf = vec![0u8; 65536];
    while let Ok((n, from)) = recv_from(&*socket, &mut buf).await {
        if from != dst {
            stats.add(&stats.dropped);
            continue;
        }
        last.store(ctx.now_ms(), Ordering::Relaxed);
        let reply = Reply {
            data: buf[..n].to_vec(),
            guest,
            source: dst.ip(),
        };
        if ctx.replies.try_send(reply).is_err() {
            stats.add(&stats.udp_queue_dropped);
        }
    }
}
