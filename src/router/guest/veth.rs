//! The veth wire of a guest (T-248): `veth:NAME`. The operator makes a veth
//! pair, puts one end in the guest (a container, a namespace) and leaves the
//! end `NAME` in the namespace of the router, with no address. The router
//! sets `NAME` up and reads its Ethernet frames with an `AF_PACKET` socket.
//!
//! The frames are those of the veth wire of `pod-hold`: the router answers
//! ARP for its address in the guest net (the first host) with the MAC address
//! `GATEWAY_MAC`, so the kernel of the router drops the frames of the guest to
//! the gateway, and only the packet socket reads them. The router learns the
//! MAC address of each guest address from its frames, and sends a packet to a
//! guest address with no MAC address to nowhere: the guest opens each flow,
//! so its address is known first.
//!
//! An address keeps the first MAC address that it came with while it is in
//! use (T-347): a frame of a known address from another MAC address is
//! dropped, until the address is idle for [`MAC_IDLE`]. So a guest on a
//! shared wire cannot take the address of another guest. The table holds
//! [`MACS_MAX`] addresses; at the limit an idle address goes out, and a new
//! address waits when no address is idle.
//!
//! A guest kernel with segmentation offloads on (the default of veth) sends
//! one frame of up to 64 KiB for many TCP segments, with a partial checksum.
//! The wire takes frames up to [`PACKET_MAX`], and the stack does not check
//! the checksums of the guest.

use std::collections::HashMap;
use std::io;
use std::net::Ipv4Addr;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use pod_hold::netns::{frame_for, take_frame, Frame, ETH_HEADER};

use super::wire::PACKET_MAX;
use super::GuestNet;

/// The most guest addresses that a wire keeps a MAC address for. At the
/// limit, an idle address goes out; a new address waits when none is idle.
pub const MACS_MAX: usize = 4096;
/// The time with no frame after which the MAC address of a guest address
/// can change, and its place can go to another address.
pub const MAC_IDLE: Duration = Duration::from_secs(60);
/// The longest frame that a read takes.
pub const FRAME_MAX: usize = PACKET_MAX + ETH_HEADER;

/// What one frame of the guest gives.
#[derive(Debug, PartialEq, Eq)]
pub enum Take {
    /// An IP packet of this length, in the buffer of the caller.
    Packet(usize),
    /// A frame to send back at once (the ARP reply for the gateway).
    Reply(Vec<u8>),
    /// Not for the router.
    Drop,
}

/// The frames of a veth wire: the ARP answer and the MAC addresses of the
/// guests.
pub struct Frames {
    net: GuestNet,
    /// The MAC address of each guest address, and the time of its last
    /// frame.
    macs: Mutex<HashMap<Ipv4Addr, ([u8; 6], Instant)>>,
    /// The buffer of a read: one reader at a time.
    pub(super) frame: Mutex<Vec<u8>>,
}

impl Frames {
    pub fn new(net: GuestNet) -> Frames {
        Frames {
            net,
            macs: Mutex::new(HashMap::new()),
            frame: Mutex::new(vec![0; FRAME_MAX]),
        }
    }

    /// Learn the MAC address of `ip` from a frame at `now`. False when the
    /// frame must be dropped: `ip` is in use with another MAC address, or
    /// the table is full of addresses in use.
    pub(super) fn learn_at(&self, ip: Ipv4Addr, mac: &[u8], now: Instant) -> bool {
        let Ok(mac) = <[u8; 6]>::try_from(mac) else {
            return true;
        };
        if !self.net.contains(ip) || mac[0] & 1 == 1 {
            return true;
        }
        let idle = |at: &Instant| now.saturating_duration_since(*at) >= MAC_IDLE;
        let mut macs = self.macs.lock().unwrap_or_else(|e| e.into_inner());
        let full = macs.len() >= MACS_MAX;
        match macs.get_mut(&ip) {
            Some((known, at)) if *known == mac => {
                *at = now;
                return true;
            }
            Some((_, at)) if !idle(at) => {
                tracing::debug!(
                    "veth: {ip} is in use by another MAC address: the frame is dropped"
                );
                return false;
            }
            Some(_) => {}
            None if full => {
                macs.retain(|_, (_, at)| !idle(at));
                if macs.len() >= MACS_MAX {
                    tracing::debug!("veth: {MACS_MAX} addresses are in use: {ip} waits");
                    return false;
                }
            }
            None => {}
        }
        macs.insert(ip, (mac, now));
        true
    }

    /// The MAC address of the guest address `ip`, when the wire knows it.
    pub fn mac_of(&self, ip: Ipv4Addr) -> Option<[u8; 6]> {
        let macs = self.macs.lock().unwrap_or_else(|e| e.into_inner());
        macs.get(&ip).map(|(mac, _)| *mac)
    }

    /// Take one frame of the guest. An IP packet goes into `out`.
    pub fn take(&self, frame: &[u8], out: &mut [u8]) -> Take {
        match take_frame(frame, self.net.gateway(), PACKET_MAX) {
            Frame::Ip(packet) => {
                if packet[0] >> 4 == 4 && packet.len() >= 20 {
                    let src = Ipv4Addr::new(packet[12], packet[13], packet[14], packet[15]);
                    if !self.learn_at(src, &frame[6..12], Instant::now()) {
                        return Take::Drop;
                    }
                }
                if packet.len() > out.len() {
                    return Take::Drop;
                }
                out[..packet.len()].copy_from_slice(&packet);
                Take::Packet(packet.len())
            }
            Frame::Reply(reply) => {
                // The reply goes to the sender: its MAC and its address.
                let ip = Ipv4Addr::new(reply[38], reply[39], reply[40], reply[41]);
                if !self.learn_at(ip, &reply[0..6], Instant::now()) {
                    return Take::Drop;
                }
                Take::Reply(reply)
            }
            Frame::Drop => Take::Drop,
        }
    }

    /// The frame of one IPv4 packet of the stack; `None` when the wire does
    /// not know the MAC address of its destination.
    pub fn frame(&self, packet: &[u8]) -> Option<Vec<u8>> {
        if packet.len() < 20 || packet[0] >> 4 != 4 {
            return None;
        }
        let dst = Ipv4Addr::new(packet[16], packet[17], packet[18], packet[19]);
        let mac = self.mac_of(dst)?;
        let mut frame = frame_for(packet)?;
        frame[0..6].copy_from_slice(&mac);
        Some(frame)
    }
}

fn last_os(what: &str) -> io::Error {
    let e = io::Error::last_os_error();
    io::Error::new(e.kind(), format!("{what}: {e}"))
}

const ETH_P_ALL: u16 = 0x0003;
const PACKET_IGNORE_OUTGOING: libc::c_int = 23;

/// Set the interface `name` up and open a packet socket of its frames (not
/// blocking).
pub fn open_veth(name: &str) -> io::Result<OwnedFd> {
    let c_name = std::ffi::CString::new(name)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "a name with a NUL"))?;
    // SAFETY: a C string that lives for the call.
    let index = unsafe { libc::if_nametoindex(c_name.as_ptr()) };
    if index == 0 {
        return Err(last_os(&format!("the interface {name:?}")));
    }
    let proto = ETH_P_ALL.to_be();
    // SAFETY: a socket with constant arguments.
    let fd = unsafe {
        libc::socket(
            libc::AF_PACKET,
            libc::SOCK_RAW | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
            libc::c_int::from(proto),
        )
    };
    if fd < 0 {
        return Err(last_os("packet socket"));
    }
    // SAFETY: a new descriptor that nothing else owns.
    let owned = unsafe { OwnedFd::from_raw_fd(fd) };
    let mut ifr = pod_hold::netns::ifreq_for(name);
    // SAFETY: SIOCGIFFLAGS and SIOCSIFFLAGS with a valid ifreq.
    unsafe {
        if libc::ioctl(fd, libc::SIOCGIFFLAGS as _, &mut ifr as *mut libc::ifreq) < 0 {
            return Err(last_os("ioctl SIOCGIFFLAGS"));
        }
        ifr.ifr_ifru.ifru_flags |= libc::IFF_UP as libc::c_short;
        if libc::ioctl(fd, libc::SIOCSIFFLAGS as _, &mut ifr as *mut libc::ifreq) < 0 {
            return Err(last_os("ioctl SIOCSIFFLAGS"));
        }
    }
    // SAFETY: a zeroed sockaddr_ll is valid.
    let mut addr: libc::sockaddr_ll = unsafe { std::mem::zeroed() };
    addr.sll_family = libc::AF_PACKET as libc::c_ushort;
    addr.sll_protocol = proto;
    addr.sll_ifindex = index as libc::c_int;
    // SAFETY: `addr` is a valid sockaddr_ll.
    let rc = unsafe {
        libc::bind(
            owned.as_raw_fd(),
            std::ptr::from_ref(&addr).cast(),
            std::mem::size_of::<libc::sockaddr_ll>() as libc::socklen_t,
        )
    };
    if rc < 0 {
        return Err(last_os("bind the packet socket"));
    }
    // The frames that the router sends do not come back. A kernel before
    // 4.20 has no such option; `take_frame` drops them too.
    let one: libc::c_int = 1;
    // SAFETY: an int that lives for the call.
    unsafe {
        libc::setsockopt(
            owned.as_raw_fd(),
            libc::SOL_PACKET,
            PACKET_IGNORE_OUTGOING,
            std::ptr::from_ref(&one).cast(),
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        )
    };
    Ok(owned)
}
