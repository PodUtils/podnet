//! The TUN device of the netns backend: its setup inside the namespace,
//! and the pump of its descriptor. The pump holds the IP packets that the
//! kernel sends (`rx`) and the packets that wait for the kernel (`tx`). The
//! backend moves them to and from the stack of `pod-netstack`.

#[allow(unused_imports)]
use crate::prelude::*;

// pod-hold (PodUtils/podcrates) makes the TUN in the namespace: its
// address, its MTU and the default route, with ioctls. TUNSETIFF has the
// ioctl encoding of the target (powerpc puts the direction at bit 29).
pub(crate) use pod_hold::netns::{IFF_NO_PI, IFF_TUN};
pub(crate) const TUNSETIFF: libc::c_ulong = pod_hold::abi::TUNSETIFF as libc::c_ulong;

pub(crate) fn bringup_loopback() -> Result<()> {
    Ok(pod_hold::netns::bringup_loopback()?)
}

/// Create and configure the TUN device inside the namespace. Returns its fd.
pub(crate) fn create_tun(name: &str) -> Result<OwnedFd> {
    Ok(pod_hold::netns::create_tun(&hold_tun(name))?)
}

/// The TUN `name` of podnet (also the program end of the veth wire), for
/// pod-hold.
pub(crate) fn hold_tun(name: &str) -> pod_hold::netns::Tun {
    pod_hold::netns::Tun {
        name: name.into(),
        addr: TUN_ADDR,
        gateway: TUN_GW,
        prefix: TUN_PREFIX,
        mtu: TUN_MTU,
    }
}

// ── the pump of the wire ────────────────────────────────────────────────────

// The wire of the namespace (T-174), of pod-hold: the TUN (each read and
// write is one IP packet) or the veth pair (each read and write is one
// Ethernet frame of a packet socket, `super::veth`).
pub(crate) use pod_hold::netns::Wire;

/// The pump of the wire: the TUN or the veth wire. `rx` and `tx` hold IP
/// packets for both wires.
pub(crate) struct Tun {
    pub(crate) fd: RawFd,
    pub(crate) mtu: usize,
    pub(crate) wire: Wire,
    pub(crate) rx: VecDeque<Vec<u8>>,
    pub(crate) tx: VecDeque<Vec<u8>>,
}

impl Tun {
    /// The device on `fd`: the veth wire when `fd` is a socket, else the TUN.
    pub(crate) fn new(fd: RawFd, mtu: usize) -> Self {
        // The read loop drains until EAGAIN, so the fd must not block.
        unsafe { libc::fcntl(fd, libc::F_SETFL, libc::O_NONBLOCK) };
        let wire = match super::veth::is_veth(fd) {
            true => Wire::Veth,
            false => Wire::Tun,
        };
        Self {
            fd,
            mtu,
            wire,
            rx: VecDeque::new(),
            tx: VecDeque::new(),
        }
    }

    /// Read whatever the kernel has queued into `rx`.
    pub(crate) fn read_ready(&mut self) {
        if self.wire == Wire::Veth {
            return self.read_frames();
        }
        loop {
            let mut buf = vec![0u8; self.mtu];
            let n = unsafe { libc::read(self.fd, buf.as_mut_ptr() as *mut _, self.mtu) };
            if n <= 0 {
                break;
            }
            buf.truncate(n as usize);
            self.rx.push_back(buf);
        }
    }

    /// Read the frames of the veth wire: the IP packets go to `rx`, and an
    /// ARP reply goes out at once. A frame larger than the MTU (a segment
    /// of an offload) is read whole and dropped.
    fn read_frames(&mut self) {
        let mut buf = vec![0u8; Wire::Veth.read_buffer(self.mtu)];
        loop {
            let n = unsafe { libc::read(self.fd, buf.as_mut_ptr() as *mut _, buf.len()) };
            if n <= 0 {
                break;
            }
            match super::veth::take_frame(&buf[..n as usize], self.mtu) {
                super::veth::Frame::Ip(packet) => self.rx.push_back(packet),
                super::veth::Frame::Reply(frame) => {
                    self.write_one(&frame);
                }
                super::veth::Frame::Drop => {}
            }
        }
    }

    /// Write one packet or frame. False when the kernel takes no more.
    fn write_one(&self, pkt: &[u8]) -> bool {
        let mut off = 0;
        while off < pkt.len() {
            let n =
                unsafe { libc::write(self.fd, pkt[off..].as_ptr() as *const _, pkt.len() - off) };
            if n <= 0 {
                return false;
            }
            off += n as usize;
        }
        true
    }

    pub(crate) fn flush(&mut self) {
        if self.wire == Wire::Veth {
            while let Some(pkt) = self.tx.pop_front() {
                let Some(frame) = super::veth::frame_for(&pkt) else {
                    continue;
                };
                if !self.write_one(&frame) {
                    return;
                }
            }
            return;
        }
        while let Some(pkt) = self.tx.pop_front() {
            let mut off = 0;
            while off < pkt.len() {
                let n = unsafe {
                    libc::write(self.fd, pkt[off..].as_ptr() as *const _, pkt.len() - off)
                };
                if n <= 0 {
                    return;
                }
                off += n as usize;
            }
        }
    }
}
