//! The veth wire of the netns backend (T-174). When the namespace cannot
//! open `/dev/net/tun`, the child makes a veth pair in the namespace: the
//! program end has the address and the default route of the TUN, and an
//! `AF_PACKET` socket on the wire end carries its frames to the stack. The
//! wire translates between frames and IP packets: the stack keeps its IP
//! medium, and the wire answers ARP for the gateway.
//!
//! pod-hold (PodUtils/podcrates) has the wire: the setup of the pair (one
//! `RTM_NEWLINK`, the offloads off), the packet socket, the ARP answer and
//! the frames. podnet keeps its test hook and its names.

#[allow(unused_imports)]
use crate::prelude::*;

pub(crate) use pod_hold::netns::{frame_for, Frame};

/// The test hook of the wire: set and not empty, the child makes the veth
/// wire also when it can open `/dev/net/tun`.
pub(crate) const TEST_VETH_VARIABLE: &str = "PODNET_NETNS_TEST_VETH";

/// True when [`TEST_VETH_VARIABLE`] is set and not empty.
pub(crate) fn veth_forced() -> bool {
    std::env::var_os(TEST_VETH_VARIABLE).is_some_and(|v| !v.is_empty())
}

/// The wire of the namespace: the TUN, else the veth pair. The program end
/// of each is `name`. Return the descriptor that the stack reads.
pub(crate) fn open_wire(name: &str) -> Result<OwnedFd> {
    let choice = match veth_forced() {
        true => pod_hold::netns::WireChoice::Veth,
        false => pod_hold::netns::WireChoice::Auto,
    };
    Ok(pod_hold::netns::open_wire(&hold_tun(name), choice)?.0)
}

/// Make the veth pair `guest` and `wire` in the namespace. Configure
/// `guest` as the TUN (address, MTU, up, default route), bring up `wire`,
/// and return a packet socket bound to `wire`.
pub(crate) fn create_veth(guest: &str, wire: &str) -> Result<OwnedFd> {
    Ok(pod_hold::netns::create_veth(&hold_tun(guest), wire)?)
}

/// True when `fd` is a socket (the veth wire), not the TUN.
pub(crate) fn is_veth(fd: RawFd) -> bool {
    matches!(pod_hold::netns::Wire::of(fd), Ok(Wire::Veth))
}

/// Take one frame of the program end.
pub(crate) fn take_frame(frame: &[u8], mtu: usize) -> Frame {
    pod_hold::netns::take_frame(frame, TUN_GW, mtu)
}
