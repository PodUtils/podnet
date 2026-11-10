//! The userspace IP stack.
//!
//! A [`NetStack`] owns a private IPv4/IPv6 address space and implements the
//! TCP and UDP state machines for it. Its link layer is an in-process packet
//! queue ([`crate::device::LinkDevice`]), so the stack never issues a
//! kernel `AF_INET` call. Two modes:
//!
//! * **loopback** — the link's transmit path feeds its own receive path, so a
//!   process can open a listener and a client against itself. This is the
//!   document's §9.1 proof and the basis of deterministic tests.
//! * **over a link** — an external pump moves framed packets between
//!   [`NetStack::link_in`] / [`NetStack::link_out`] and a real byte stream: a
//!   unix socket, a QUIC connection, or a `podnet` WebSocket forward.
//!
//! The clock is monotonic and real (`std::time::Instant`), and the reactor
//! blocks on the device's waker or the interface's next deadline instead of
//! spinning, which is what §11 asked for.

use std::collections::HashMap;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use futures::task::AtomicWaker;
use smoltcp::iface::{Config as IfaceConfig, Interface, SocketHandle, SocketSet};
use smoltcp::socket::{tcp, udp};
use smoltcp::time::Instant as SmolInstant;
use smoltcp::wire::{HardwareAddress, IpAddress, IpCidr, IpEndpoint, IpListenEndpoint};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::{mpsc, oneshot, Notify};

use crate::addr::{VirtAddr, VirtualSubnet};
use crate::device::{DeviceStats, DeviceStatsSnapshot, LinkDevice, PacketQueue};
use crate::dns::Resolver;
use crate::policy::Acl;

/// A monotonic millisecond clock for smoltcp.
#[derive(Clone, Debug)]
pub struct Clock {
    start: std::time::Instant,
    base_ms: i64,
}

impl Default for Clock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock {
    pub fn new() -> Self {
        Self {
            start: std::time::Instant::now(),
            base_ms: 0,
        }
    }

    /// A clock whose first reading is `base_ms`, for deterministic tests.
    pub fn starting_at(base_ms: i64) -> Self {
        Self {
            start: std::time::Instant::now(),
            base_ms,
        }
    }

    pub fn now_ms(&self) -> i64 {
        self.base_ms + self.start.elapsed().as_millis() as i64
    }

    pub fn now(&self) -> SmolInstant {
        SmolInstant::from_millis(self.now_ms())
    }
}

/// Stack configuration.
#[derive(Clone)]
pub struct StackConfig {
    pub subnet: VirtualSubnet,
    /// The address this stack owns (and answers for) in the v4 space.
    pub local_v4: Ipv4Addr,
    /// The address this stack owns in the v6 space.
    pub local_v6: Ipv6Addr,
    pub mtu: usize,
    pub tcp_buffer: usize,
    pub udp_packets: usize,
    pub udp_payload: usize,
    /// Listeners keep at most this many listening sockets, which bounds
    /// simultaneous half-open inbound connections.
    pub accept_pool: usize,
    pub max_connections: usize,
    /// Connect-time access policy.
    pub acl: Option<Arc<Acl>>,
    /// A resolver served at the gateway's port 53. When set, the stack owns the
    /// gateway address and answers A/AAAA queries for the names in the table.
    pub dns: Option<Arc<Resolver>>,
    /// Seed for smoltcp's port/sequence randomisation.
    pub random_seed: u64,
    /// Upper bound on the reactor's sleep between polls.
    pub max_poll_interval: Duration,
}

impl Default for StackConfig {
    fn default() -> Self {
        Self {
            subnet: VirtualSubnet::default(),
            local_v4: Ipv4Addr::new(10, 66, 0, 2),
            local_v6: "fd00:66::2".parse().unwrap(),
            mtu: 1280,
            tcp_buffer: 64 * 1024,
            udp_packets: 64,
            udp_payload: 64 * 1024,
            accept_pool: 8,
            max_connections: 4096,
            acl: None,
            dns: None,
            random_seed: seed_from_time(),
            max_poll_interval: Duration::from_millis(100),
        }
    }
}

fn seed_from_time() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9e37_79b9_7f4a_7c15)
}

/// Something worth observing happened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StackEvent {
    ListenerBound {
        addr: VirtAddr,
    },
    Accepted {
        id: u32,
        local: VirtAddr,
        remote: VirtAddr,
    },
    Connected {
        id: u32,
        local: VirtAddr,
        remote: VirtAddr,
    },
    Refused {
        remote: VirtAddr,
        reason: String,
    },
    Closed {
        id: u32,
    },
    Datagram {
        from: VirtAddr,
        to: VirtAddr,
        len: usize,
    },
}

/// Connection counters, readable while the reactor runs.
#[derive(Debug, Default)]
pub struct NetStats {
    pub accepted: AtomicU64,
    pub connected: AtomicU64,
    pub refused: AtomicU64,
    pub closed: AtomicU64,
    pub datagrams: AtomicU64,
    /// Inbound connections the accept queue could not take. Counted separately
    /// from `accepted` because a drop is not an acceptance, and a counter that
    /// silently under-reports is worse than no counter: an operator watching
    /// `accepted` would see a number lower than the peer's connection count
    /// with nothing indicating anything was lost.
    pub dropped: AtomicU64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NetStatsSnapshot {
    pub accepted: u64,
    pub connected: u64,
    pub refused: u64,
    pub closed: u64,
    pub datagrams: u64,
    pub dropped: u64,
}

impl NetStats {
    fn snapshot(&self) -> NetStatsSnapshot {
        NetStatsSnapshot {
            accepted: self.accepted.load(Ordering::Relaxed),
            connected: self.connected.load(Ordering::Relaxed),
            refused: self.refused.load(Ordering::Relaxed),
            closed: self.closed.load(Ordering::Relaxed),
            dropped: self.dropped.load(Ordering::Relaxed),
            datagrams: self.datagrams.load(Ordering::Relaxed),
        }
    }
}

fn to_smoltcp(ip: IpAddr) -> IpAddress {
    match ip {
        IpAddr::V4(ip) => IpAddress::Ipv4(ip),
        IpAddr::V6(ip) => IpAddress::Ipv6(ip),
    }
}

fn from_smoltcp(ip: IpAddress) -> IpAddr {
    match ip {
        IpAddress::Ipv4(ip) => IpAddr::V4(ip),
        IpAddress::Ipv6(ip) => IpAddr::V6(ip),
    }
}

fn endpoint_to_virt(endpoint: IpEndpoint) -> VirtAddr {
    VirtAddr::new(from_smoltcp(endpoint.addr), endpoint.port)
}

mod datagrams;
mod proof;
mod reactor;
mod service;
mod socket;
pub use datagrams::VnetDatagrams;
pub use proof::{loopback_proof, loopback_round_trip, ProofReport};
use reactor::*;
use socket::*;
pub use socket::{SocketCount, TcpListener, TcpStream, UdpSocket};

// ── async facade ────────────────────────────────────────────────────────────

/// The reactor task's runtime handle.
pub struct NetStack {
    cmd: mpsc::Sender<Command>,
    link_in: PacketQueue,
    link_out: PacketQueue,
    stats: Arc<NetStats>,
    device_stats: Arc<DeviceStats>,
    socket_count: Arc<SocketCount>,
    shutdown: Arc<Notify>,
    kick: Arc<Notify>,
    events_rx: Option<mpsc::UnboundedReceiver<StackEvent>>,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl NetStack {
    /// A stack whose link loops back into itself.
    pub fn loopback() -> Self {
        Self::new(StackConfig::default(), true)
    }

    /// A stack with a configurable loopback link.
    pub fn loopback_with(config: StackConfig) -> Self {
        Self::new(config, true)
    }

    /// A stack whose packets leave and enter through a real link.
    pub fn over_link(config: StackConfig) -> Self {
        Self::new(config, false)
    }

    fn new(config: StackConfig, loopback: bool) -> Self {
        let device = if loopback {
            LinkDevice::loopback(config.mtu)
        } else {
            LinkDevice::link(config.mtu)
        };
        // A packet to an address of this stack stays in it (issue 65).
        device.add_local(IpAddr::V4(config.local_v4));
        device.add_local(IpAddr::V6(config.local_v6));
        device.add_local(config.subnet.gateway(crate::addr::Family::V4));
        device.add_local(config.subnet.gateway(crate::addr::Family::V6));
        let link_in = device.inbound();
        let link_out = device.outbound();
        let device_stats = device.stats();
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let kick = Arc::new(Notify::new());
        let max_poll_interval = config.max_poll_interval;
        let socket_count = Arc::new(SocketCount::default());
        let core = Core::new(
            config,
            device,
            events_tx,
            kick.clone(),
            socket_count.clone(),
        );
        let stats = core.stats.clone();
        let (cmd_tx, cmd_rx) = mpsc::channel(64);
        let shutdown = Arc::new(Notify::new());
        let inbound_wait = link_in.clone();
        let shutdown_flag = shutdown.clone();
        let run_kick = kick.clone();
        let task = tokio::spawn(async move {
            run(
                core,
                cmd_rx,
                inbound_wait,
                shutdown_flag,
                run_kick,
                max_poll_interval,
            )
            .await;
        });
        Self {
            cmd: cmd_tx,
            link_in,
            link_out,
            stats,
            device_stats,
            socket_count,
            shutdown,
            kick,
            events_rx: Some(events_rx),
            task: Some(task),
        }
    }

    async fn send(&self, command: Command) -> io::Result<()> {
        self.cmd
            .send(command)
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "virtual stack is shut down"))
    }

    /// Bind a virtual TCP listener.
    pub async fn listen(&self, addr: VirtAddr) -> io::Result<TcpListener> {
        let (reply, rx) = oneshot::channel();
        self.send(Command::Listen { addr, reply }).await?;
        rx.await
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "virtual stack is shut down"))?
    }

    /// Open a virtual TCP connection.
    pub async fn connect(&self, remote: VirtAddr) -> io::Result<TcpStream> {
        self.connect_from(remote, 0).await
    }

    pub async fn connect_from(&self, remote: VirtAddr, local_port: u16) -> io::Result<TcpStream> {
        let (reply, rx) = oneshot::channel();
        self.send(Command::Connect {
            remote,
            local_port,
            reply,
        })
        .await?;
        rx.await
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "virtual stack is shut down"))?
    }

    /// Bind a virtual UDP socket.
    pub async fn bind_udp(&self, addr: VirtAddr) -> io::Result<UdpSocket> {
        let (reply, rx) = oneshot::channel();
        self.send(Command::BindUdp { addr, reply }).await?;
        rx.await
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "virtual stack is shut down"))?
    }

    /// Packets arriving from the external link (written by the pump, read by
    /// the stack).
    pub fn link_in(&self) -> PacketQueue {
        self.link_in.clone()
    }

    /// Packets leaving the stack (read by the pump, written to the link).
    pub fn link_out(&self) -> PacketQueue {
        self.link_out.clone()
    }

    pub fn stats(&self) -> NetStatsSnapshot {
        self.stats.snapshot()
    }

    pub fn device_stats(&self) -> DeviceStatsSnapshot {
        self.device_stats.snapshot()
    }

    /// Sockets the reactor holds right now, and the largest number it has held
    /// at once.
    ///
    /// A `SocketSet` reuses a vacant slot on `add` but never reclaims one, so a
    /// set that is not pruned when a connection closes grows without bound. This
    /// makes that visible: `live` returns to its resting value after connections
    /// close if the set is being pruned, and climbs if it is not.
    pub fn socket_count(&self) -> (u64, u64) {
        (self.socket_count.live(), self.socket_count.peak())
    }

    /// Take the event stream, at most once.
    pub fn take_events(&mut self) -> Option<mpsc::UnboundedReceiver<StackEvent>> {
        self.events_rx.take()
    }

    /// Stop the reactor.
    pub async fn shutdown(&mut self) {
        let _ = self.cmd.try_send(Command::Shutdown);
        self.shutdown.notify_one();
        self.kick.notify_one();
        if let Some(task) = self.task.take() {
            let _ = tokio::time::timeout(Duration::from_secs(2), task).await;
        }
    }
}

impl Drop for NetStack {
    fn drop(&mut self) {
        // `notify_one`, not `notify_waiters`: it stores a permit, so a notify
        // that lands while the reactor is between polls is not lost.
        self.shutdown.notify_one();
        self.kick.notify_one();
    }
}

async fn run(
    mut core: Core,
    mut cmd_rx: mpsc::Receiver<Command>,
    inbound_wait: PacketQueue,
    shutdown: Arc<Notify>,
    kick: Arc<Notify>,
    max_poll_interval: Duration,
) {
    loop {
        core.poll_iface();
        core.service();
        core.poll_iface();

        // `next_delay` already clamps to the configured maximum, so clamping
        // again to a hardcoded 100ms would make `max_poll_interval` a no-op for
        // every value above 100ms: a stack configured to poll every 5s would
        // still poll every 100ms. The only floor applied here keeps a
        // zero-valued configuration from turning the loop into a busy spin.
        let floor = Duration::from_micros(200).min(max_poll_interval);
        let delay = core.next_delay().unwrap_or(max_poll_interval).max(floor);

        tokio::select! {
            biased;
            command = cmd_rx.recv() => {
                match command {
                    Some(Command::Shutdown) | None => break,
                    Some(command) => {
                        core.handle(command);
                        // The command may have created a socket; poll promptly.
                    }
                }
            }
            _ = kick.notified() => {}
            _ = inbound_wait.wait() => {}
            _ = tokio::time::sleep(delay) => {}
            _ = shutdown.notified() => break,
        }
    }
    // Wake anything still blocked on a channel.
    core.conns.clear();
    core.udp.clear();
}

// ── the synchronous §9.1 proof ──────────────────────────────────────────────

/// Small helper namespace so the integration helper does not need a public
/// alias for `Vec<u8>`.
pub mod net {
    use std::net::IpAddr;

    /// The default local address this crate uses for loopback tests.
    pub const LOCAL_IP: IpAddr = IpAddr::V4(std::net::Ipv4Addr::new(10, 66, 0, 2));
}

#[cfg(test)]
mod tests;
