//! The pump of a guest wire (T-248). It reads each packet of the guest and
//! screens it, holds each new SYN while the router dials (`tcp`), gives the
//! UDP ports to the sockets of the stack (`udp`), and gives the rest to the
//! link of the stack. A writer task sends the packets of the stack to the
//! wire.

use std::net::Ipv4Addr;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Result};
use pod_netstack::smoltcp::phy::{Checksum, ChecksumCapabilities};
use pod_netstack::{NetStack, PacketQueue, StackConfig, Subnet};
use tokio::time::MissedTickBehavior;

use super::wire::{Listener, Opened, PacketFd, PACKET_MAX};
use super::{tcp, udp, RouterGuest, MAX_FLOWS};

/// The addresses of a TCP connection of the guest: (source, source port,
/// destination, destination port).
pub type SynKey = (Ipv4Addr, u16, Ipv4Addr, u16);

/// The time between two passes of the housekeeping, and the longest time
/// that a full queue of the stack stops the read of the wire.
const TICK: Duration = Duration::from_millis(50);

/// What the screen decides for one packet of the guest.
#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Give the packet to the stack.
    Pass,
    /// A TCP SYN with no ACK: hold it while the router dials.
    Syn(SynKey),
    /// A UDP datagram, or its first fragment, for this port.
    Udp(u16),
    /// Drop the packet.
    Drop,
}

pub struct Pump {
    pub(super) guest: Arc<RouterGuest>,
    pub(super) stack: NetStack,
    pub(super) link_in: PacketQueue,
    pub(super) link_out: PacketQueue,
    pub(super) tcp: tcp::State,
    pub(super) udp: udp::State,
}

impl Pump {
    /// The pump of `guest`. Call it inside a runtime of tokio: the stack
    /// spawns its reactor.
    pub fn new(guest: Arc<RouterGuest>) -> Result<Pump> {
        let gateway = guest.net.gateway();
        // The program end can send a partial checksum (an offload that stays
        // on): compute the checksums, do not check them.
        let mut checksum = ChecksumCapabilities::default();
        checksum.ipv4 = Checksum::Tx;
        checksum.tcp = Checksum::Tx;
        checksum.udp = Checksum::Tx;
        checksum.icmpv4 = Checksum::Tx;
        let config = StackConfig {
            subnet: Subnet {
                v4: gateway,
                v4_prefix: guest.net.prefix,
                ..Subnet::default()
            },
            local_v4: gateway,
            mtu: guest.mtu,
            random_seed: seed(),
            any_ip: true,
            max_connections: MAX_FLOWS + 64,
            checksum,
            ..StackConfig::default()
        };
        let stack = NetStack::over_link(config);
        Ok(Pump {
            link_in: stack.link_in(),
            link_out: stack.link_out(),
            stack,
            tcp: tcp::State::new(),
            udp: udp::State::new(),
            guest,
        })
    }

    /// Serve the wire until it fails.
    pub async fn run(mut self, opened: Opened) -> Result<()> {
        match opened {
            Opened::Packets(fd) => {
                self.session(Arc::new(PacketFd::new(fd)?), None).await?;
                bail!("the wire {} closed", self.guest.wire)
            }
            Opened::Listener(fd) => {
                let listener = Listener::new(fd)?;
                let mut io = listener.accept().await?;
                loop {
                    // A new connection replaces the old one.
                    io = match self.session(Arc::new(io), Some(&listener)).await {
                        Ok(Some(next)) => next,
                        other => {
                            if let Err(e) = other {
                                tracing::debug!(wire = %self.guest.wire, "the guest left: {e}");
                            }
                            listener.accept().await?
                        }
                    };
                }
            }
        }
    }

    /// Serve one connection of the wire until it ends, or until another
    /// guest connects (then that connection is returned).
    async fn session(
        &mut self,
        io: Arc<PacketFd>,
        listener: Option<&Listener>,
    ) -> std::io::Result<Option<PacketFd>> {
        self.link_out.clear();
        let out = self.link_out.clone();
        let writer = {
            let io = io.clone();
            tokio::spawn(async move {
                loop {
                    let packet = out.recv().await;
                    if io.send(&packet).await.is_err() {
                        return;
                    }
                }
            })
        };
        let result = self.read_loop(&io, listener).await;
        writer.abort();
        result
    }

    async fn read_loop(
        &mut self,
        io: &PacketFd,
        listener: Option<&Listener>,
    ) -> std::io::Result<Option<PacketFd>> {
        let mut buf = vec![0u8; PACKET_MAX];
        let mut tick = tokio::time::interval(TICK);
        tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                biased;
                Some(done) = self.tcp.dials.recv() => self.dial_done(done).await,
                Some(stream) = self.tcp.accepted.recv() => self.accept(stream),
                _ = tick.tick() => self.housekeeping(),
                next = next_guest(listener) => return next.map(Some),
                // The stack has no wait for room: a full queue stops the
                // read, and the guest or the kernel holds the packets.
                n = io.recv(&mut buf), if !self.link_in.is_full() => {
                    let n = n?;
                    if n == 0 {
                        return Ok(None);
                    }
                    self.packet(buf[..n].to_vec()).await;
                }
            }
        }
    }

    async fn packet(&mut self, packet: Vec<u8>) {
        match screen(&self.guest, &packet) {
            Verdict::Drop => self.guest.stats.add(&self.guest.stats.dropped),
            Verdict::Pass => {
                self.link_in.push(packet);
            }
            Verdict::Syn(key) => self.syn(key, packet),
            Verdict::Udp(port) => {
                if self.ensure_udp(port).await {
                    self.link_in.push(packet);
                }
            }
        }
    }

    fn housekeeping(&mut self) {
        self.tcp_housekeeping();
        self.udp_housekeeping();
    }

    /// The number of the TCP flows now: the held SYNs and the open flows.
    pub(super) fn tcp_flows(&self) -> usize {
        self.tcp.held.len() + self.guest.stats.tcp_open.load(Ordering::Relaxed) as usize
    }
}

/// Decide what to do with `packet` of the guest. Only IPv4 TCP and UDP go
/// on. The source must be in the net of the guest, and the destination must
/// be a unicast address outside it: the router does not route a flow back
/// into the net of the guest.
pub fn screen(guest: &RouterGuest, packet: &[u8]) -> Verdict {
    if packet.len() < 20 || packet[0] >> 4 != 4 {
        return Verdict::Drop;
    }
    let ihl = usize::from(packet[0] & 0x0f) * 4;
    let total = usize::from(u16::from_be_bytes([packet[2], packet[3]]));
    if ihl < 20 || total < ihl || total > packet.len() {
        return Verdict::Drop;
    }
    let src = Ipv4Addr::new(packet[12], packet[13], packet[14], packet[15]);
    let dst = Ipv4Addr::new(packet[16], packet[17], packet[18], packet[19]);
    let routable = !(dst.is_multicast()
        || dst.is_broadcast()
        || dst.is_unspecified()
        || dst.is_loopback()
        || dst.is_link_local());
    if !guest.net.contains(src) || guest.net.contains(dst) || !routable {
        return Verdict::Drop;
    }
    let offset = u16::from_be_bytes([packet[6], packet[7]]) & 0x1fff;
    match packet[9] {
        6 => match pod_hold::packet::parse_tcp_syn(packet) {
            Some(key) => Verdict::Syn(key),
            None => Verdict::Pass,
        },
        17 if offset == 0 && total >= ihl + 8 => {
            Verdict::Udp(u16::from_be_bytes([packet[ihl + 2], packet[ihl + 3]]))
        }
        // A later fragment of a datagram: the stack joins it.
        17 if offset != 0 => Verdict::Pass,
        _ => Verdict::Drop,
    }
}

fn seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9e37_79b9_7f4a_7c15)
}

/// The next guest that connects to a `unix:` wire; never ends for a wire
/// with no listener.
async fn next_guest(listener: Option<&Listener>) -> std::io::Result<PacketFd> {
    match listener {
        Some(listener) => listener.accept().await,
        None => std::future::pending().await,
    }
}
