//! A guest wire for the tests of T-176: a router stack that answers for each
//! destination (`any_ip`) and a guest stack, joined by a pump that can hold a
//! SYN. Both are `pod-netstack`.

#![allow(dead_code)]

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pod_netstack::{NetStack, PacketQueue, StackConfig, Subnet};
use tokio::task::JoinHandle;

pub const GUEST: Ipv4Addr = Ipv4Addr::new(10, 66, 0, 4);

pub fn at(ip: &str, port: u16) -> SocketAddr {
    SocketAddr::new(ip.parse::<IpAddr>().unwrap(), port)
}

pub fn router_config() -> StackConfig {
    StackConfig {
        any_ip: true,
        ..StackConfig::default()
    }
}

/// A guest whose subnet is the whole space, so that it dials each address
/// over the wire.
pub fn guest_config() -> StackConfig {
    StackConfig {
        subnet: Subnet {
            v4_prefix: 0,
            v6_prefix: 0,
            ..Subnet::default()
        },
        local_v4: GUEST,
        ..StackConfig::default()
    }
}

/// True for a TCP segment with SYN and no ACK in an IPv4 packet.
pub fn is_syn(packet: &[u8]) -> bool {
    if packet.len() < 20 || packet[0] >> 4 != 4 || packet[9] != 6 {
        return false;
    }
    let ihl = usize::from(packet[0] & 15) * 4;
    packet.len() > ihl + 13 && packet[ihl + 13] & 0x12 == 0x02
}

/// True for a TCP segment with RST in an IPv4 packet.
pub fn is_rst(packet: &[u8]) -> bool {
    if packet.len() < 20 || packet[0] >> 4 != 4 || packet[9] != 6 {
        return false;
    }
    let ihl = usize::from(packet[0] & 15) * 4;
    packet.len() > ihl + 13 && packet[ihl + 13] & 0x04 != 0
}

/// The RST segments that crossed the wire, in each direction.
#[derive(Default)]
pub struct Resets {
    pub up: AtomicUsize,
    pub down: AtomicUsize,
}

/// The SYNs that the pump holds, and whether it holds them now.
#[derive(Default)]
pub struct Hold {
    pub on: bool,
    pub held: Vec<Vec<u8>>,
}

/// The wire between a guest and a router: the pump of the guest holds SYNs
/// when `hold.on` is set (the `HeldSyn` of the netns backend).
pub struct Wire {
    pub hold: Arc<Mutex<Hold>>,
    pub resets: Arc<Resets>,
    pumps: Vec<JoinHandle<()>>,
    to_router: PacketQueue,
}

impl Wire {
    pub fn join(router: &NetStack, guest: &NetStack) -> Self {
        let hold = Arc::new(Mutex::new(Hold::default()));
        let resets = Arc::new(Resets::default());
        let to_router = router.link_in();
        let up = {
            let (from, to, hold, resets) = (
                guest.link_out(),
                to_router.clone(),
                hold.clone(),
                resets.clone(),
            );
            tokio::spawn(async move {
                loop {
                    let packet = from.recv().await;
                    if is_rst(&packet) {
                        resets.up.fetch_add(1, Ordering::Relaxed);
                    }
                    let mut hold = hold.lock().unwrap();
                    if hold.on && is_syn(&packet) {
                        hold.held.push(packet);
                    } else {
                        to.push(packet);
                    }
                }
            })
        };
        let down = {
            let (from, to, resets) = (router.link_out(), guest.link_in(), resets.clone());
            tokio::spawn(async move {
                loop {
                    let packet = from.recv().await;
                    if is_rst(&packet) {
                        resets.down.fetch_add(1, Ordering::Relaxed);
                    }
                    to.push(packet);
                }
            })
        };
        Self {
            hold,
            resets,
            pumps: vec![up, down],
            to_router,
        }
    }

    /// Send each held SYN to the router.
    pub fn release(&self) -> usize {
        let mut hold = self.hold.lock().unwrap();
        let held = std::mem::take(&mut hold.held);
        for packet in &held {
            self.to_router.push(packet.clone());
        }
        held.len()
    }

    pub fn held(&self) -> usize {
        self.hold.lock().unwrap().held.len()
    }
}

impl Drop for Wire {
    fn drop(&mut self) {
        for pump in &self.pumps {
            pump.abort();
        }
    }
}

pub async fn within<T>(secs: u64, what: &str, future: impl std::future::Future<Output = T>) -> T {
    match tokio::time::timeout(Duration::from_secs(secs), future).await {
        Ok(value) => value,
        Err(_) => panic!("{what}: no result in {secs} s"),
    }
}
