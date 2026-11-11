//! The userspace IP stack: `pod-netstack` (PodUtils/podcrates) at its old
//! path.
//!
//! A [`NetStack`] owns a private IPv4/IPv6 address space and implements the
//! TCP and UDP state machines for it. Its link layer is an in-process packet
//! queue ([`crate::device::LinkDevice`]), so the stack never issues a
//! kernel `AF_INET` call. The policy of this crate is its hooks: the
//! [`Acl`](crate::policy::Acl) is the `admit` of [`StackConfig`] and the
//! [`Resolver`](crate::dns::Resolver) its `gateway_service` (at
//! `10.66.0.1:53`). The §9.1 proof of `net doctor` stays here.

use std::io;
use std::net::IpAddr;

use smoltcp::iface::{Config as IfaceConfig, Interface, SocketSet};
use smoltcp::socket::tcp;
use smoltcp::time::Instant as SmolInstant;
use smoltcp::wire::{HardwareAddress, IpAddress, IpCidr, IpEndpoint, IpListenEndpoint};

use crate::addr::VirtAddr;
use crate::device::LinkDevice;

/// The datagrams of one virtual UDP socket, at its old name.
pub use pod_netstack::stack::UdpDatagrams as VnetDatagrams;
pub use pod_netstack::stack::*;

fn from_smoltcp(ip: IpAddress) -> IpAddr {
    IpAddr::from(ip)
}

mod proof;
pub use proof::{loopback_proof, loopback_round_trip, ProofReport};

/// Small helper namespace so the integration helper does not need a public
/// alias for `Vec<u8>`.
pub mod net {
    use std::net::IpAddr;

    /// The default local address this crate uses for loopback tests.
    pub const LOCAL_IP: IpAddr = IpAddr::V4(std::net::Ipv4Addr::new(10, 66, 0, 2));
}

#[cfg(test)]
mod tests;
