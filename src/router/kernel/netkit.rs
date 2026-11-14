//! The parts of the namespace tests (T-120, T-248): network namespaces held
//! by a descriptor, veth pairs across two of them, addresses, routes, and a
//! capture of the TCP SYNs that a namespace receives. Each request is
//! rtnetlink; no `ip` program. The tests run in `unshare -rn`
//! ([`super::tests::rerun_in_namespace`]).

use std::net::Ipv4Addr;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

use super::{attr, ifindex, message, new_route_request, Socket, AF_INET, MAIN};
use super::{NLM_F_ACK, NLM_F_CREATE, NLM_F_EXCL};

const RTM_NEWLINK: u16 = 16;
const RTM_NEWADDR: u16 = 20;
const RTM_DELADDR: u16 = 21;
const IFLA_IFNAME: u16 = 3;
const IFLA_MTU: u16 = 4;
const IFLA_LINKINFO: u16 = 18;
const IFLA_NET_NS_FD: u16 = 28;
const IFLA_INFO_KIND: u16 = 1;
const IFLA_INFO_DATA: u16 = 2;
const VETH_INFO_PEER: u16 = 1;
const PACKET_OUTGOING: u8 = 4;

/// A network namespace, held open by its descriptor.
pub(crate) struct Ns(OwnedFd);

fn thread_ns() -> OwnedFd {
    let path = c"/proc/thread-self/ns/net";
    // SAFETY: a valid path; constant flags.
    let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
    assert!(
        fd >= 0,
        "open {path:?}: {}",
        std::io::Error::last_os_error()
    );
    // SAFETY: a new descriptor that nothing else owns.
    unsafe { OwnedFd::from_raw_fd(fd) }
}

impl Ns {
    /// A new namespace with its loopback up.
    pub(crate) fn new() -> Ns {
        let ns = std::thread::spawn(|| {
            // SAFETY: unshare of the network namespace of this thread only.
            let rc = unsafe { libc::unshare(libc::CLONE_NEWNET) };
            assert_eq!(rc, 0, "unshare: {}", std::io::Error::last_os_error());
            up("lo");
            Ns(thread_ns())
        });
        ns.join().unwrap()
    }

    /// Run `f` on a thread in this namespace. A socket that `f` makes stays
    /// in this namespace.
    pub(crate) fn run<T: Send>(&self, f: impl FnOnce() -> T + Send) -> T {
        std::thread::scope(|s| {
            s.spawn(|| {
                // SAFETY: setns of this thread to a namespace descriptor.
                let rc = unsafe { libc::setns(self.0.as_raw_fd(), libc::CLONE_NEWNET) };
                assert_eq!(rc, 0, "setns: {}", std::io::Error::last_os_error());
                f()
            })
            .join()
            .unwrap()
        })
    }
}

fn nested(b: &mut Vec<u8>, kind: u16, inner: impl FnOnce(&mut Vec<u8>)) {
    let start = b.len();
    b.extend_from_slice(&[0; 4]);
    inner(b);
    let len = (b.len() - start) as u16;
    b[start..start + 2].copy_from_slice(&len.to_ne_bytes());
    b[start + 2..start + 4].copy_from_slice(&kind.to_ne_bytes());
}

fn ack(what: &str, msg: &[u8]) {
    let code = Socket::open().unwrap().ack(msg).unwrap();
    assert_eq!(
        code,
        0,
        "{what}: {}",
        std::io::Error::from_raw_os_error(code)
    );
}

/// Make the veth pair `here` (in the namespace of the calling thread) and
/// `there` (in `ns`). Both ends are down.
pub(crate) fn veth(here: &str, there: &str, ns: &Ns) {
    let mut b = vec![0u8; 16];
    attr(&mut b, IFLA_IFNAME, format!("{here}\0").as_bytes());
    nested(&mut b, IFLA_LINKINFO, |b| {
        attr(b, IFLA_INFO_KIND, b"veth");
        nested(b, IFLA_INFO_DATA, |b| {
            nested(b, VETH_INFO_PEER, |b| {
                b.extend_from_slice(&[0; 16]);
                attr(b, IFLA_IFNAME, format!("{there}\0").as_bytes());
                let fd = ns.0.as_raw_fd() as u32;
                attr(b, IFLA_NET_NS_FD, &fd.to_ne_bytes());
            });
        });
    });
    let flags = NLM_F_ACK | NLM_F_CREATE | NLM_F_EXCL;
    ack(
        &format!("veth {here} {there}"),
        &message(RTM_NEWLINK, flags, b),
    );
}

fn link_body(dev: &str) -> Vec<u8> {
    let mut b = vec![0u8; 16];
    b[4..8].copy_from_slice(&ifindex(dev).unwrap().to_ne_bytes());
    b
}

/// Set `dev` up.
pub(crate) fn up(dev: &str) {
    let mut b = link_body(dev);
    b[8..12].copy_from_slice(&(libc::IFF_UP as u32).to_ne_bytes());
    b[12..16].copy_from_slice(&(libc::IFF_UP as u32).to_ne_bytes());
    ack(&format!("up {dev}"), &message(RTM_NEWLINK, NLM_F_ACK, b));
}

/// Set the MTU of `dev`.
pub(crate) fn mtu(dev: &str, mtu: u32) {
    let mut b = link_body(dev);
    attr(&mut b, IFLA_MTU, &mtu.to_ne_bytes());
    ack(&format!("mtu {dev}"), &message(RTM_NEWLINK, NLM_F_ACK, b));
}

fn addr_body(dev: &str, ip: Ipv4Addr, prefix: u8) -> Vec<u8> {
    let mut b = vec![AF_INET, prefix, 0, 0];
    b.extend_from_slice(&ifindex(dev).unwrap().to_ne_bytes());
    attr(&mut b, 2, &ip.octets());
    attr(&mut b, 1, &ip.octets());
    b
}

/// Add `ip/prefix` to `dev`.
pub(crate) fn addr(dev: &str, ip: Ipv4Addr, prefix: u8) {
    let flags = NLM_F_ACK | NLM_F_CREATE | NLM_F_EXCL;
    let msg = message(RTM_NEWADDR, flags, addr_body(dev, ip, prefix));
    ack(&format!("address {ip} {dev}"), &msg);
}

/// Delete `ip/prefix` of `dev` (and its subnet route).
pub(crate) fn del_addr(dev: &str, ip: Ipv4Addr, prefix: u8) {
    let msg = message(RTM_DELADDR, NLM_F_ACK, addr_body(dev, ip, prefix));
    ack(&format!("delete address {ip} {dev}"), &msg);
}

/// `default via GATEWAY dev DEV` in the table `main`.
pub(crate) fn default_via(gateway: Ipv4Addr, dev: &str) {
    let hop = [(ifindex(dev).unwrap(), Some(gateway), 1)];
    ack("default route", &new_route_request(MAIN, &hop));
}

/// `sysctl net.ipv4.ip_forward=1` in the namespace of the calling thread.
pub(crate) fn forward_on() {
    std::fs::write("/proc/sys/net/ipv4/ip_forward", "1").unwrap();
}

/// A TCP SYN that a capture saw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Syn {
    pub(crate) src: Ipv4Addr,
    pub(crate) dst: Ipv4Addr,
    pub(crate) dport: u16,
    pub(crate) mss: Option<u16>,
    /// The interface that received it.
    pub(crate) ifindex: u32,
}

/// A packet socket of each IPv4 packet that the namespace of the calling
/// thread receives, with a read time out of 300 ms.
pub(crate) fn capture() -> OwnedFd {
    let proto = (libc::ETH_P_IP as u16).to_be() as i32;
    // SAFETY: a socket with constant arguments.
    let fd = unsafe {
        libc::socket(
            libc::AF_PACKET,
            libc::SOCK_DGRAM | libc::SOCK_CLOEXEC,
            proto,
        )
    };
    assert!(
        fd >= 0,
        "packet socket: {}",
        std::io::Error::last_os_error()
    );
    // SAFETY: a new descriptor that nothing else owns.
    let owned = unsafe { OwnedFd::from_raw_fd(fd) };
    let tv = libc::timeval {
        tv_sec: 0,
        tv_usec: 300_000,
    };
    // SAFETY: a timeval that lives for the call.
    unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_RCVTIMEO,
            std::ptr::from_ref(&tv).cast(),
            std::mem::size_of::<libc::timeval>() as libc::socklen_t,
        )
    };
    owned
}

/// The TCP SYNs (no ACK) to port `dport` that `capture` holds now.
pub(crate) fn syns(capture: &OwnedFd, dport: u16) -> Vec<Syn> {
    let mut out = Vec::new();
    let mut buf = vec![0u8; 65536];
    loop {
        // SAFETY: a zeroed sockaddr_ll is valid.
        let mut from: libc::sockaddr_ll = unsafe { std::mem::zeroed() };
        let mut len = std::mem::size_of::<libc::sockaddr_ll>() as libc::socklen_t;
        // SAFETY: `buf` and `from` have room for the read.
        let n = unsafe {
            libc::recvfrom(
                capture.as_raw_fd(),
                buf.as_mut_ptr().cast(),
                buf.len(),
                0,
                std::ptr::from_mut(&mut from).cast(),
                &mut len,
            )
        };
        if n <= 0 {
            return out;
        }
        if from.sll_pkttype == PACKET_OUTGOING {
            continue;
        }
        if let Some(mut syn) = parse_syn(&buf[..n as usize], dport) {
            syn.ifindex = from.sll_ifindex as u32;
            out.push(syn);
        }
    }
}

fn parse_syn(p: &[u8], dport: u16) -> Option<Syn> {
    if p.len() < 20 || p[0] >> 4 != 4 || p[9] != 6 {
        return None;
    }
    let ihl = usize::from(p[0] & 0x0f) * 4;
    let tcp = p.get(ihl..)?;
    if tcp.len() < 20 || u16::from_be_bytes([tcp[2], tcp[3]]) != dport {
        return None;
    }
    // SYN set, ACK clear.
    if tcp[13] & 0x12 != 0x02 {
        return None;
    }
    let data = usize::from(tcp[12] >> 4) * 4;
    let mut options = tcp.get(20..data)?;
    let mut mss = None;
    while let Some(&kind) = options.first() {
        match kind {
            0 => break,
            1 => options = &options[1..],
            _ => {
                let len = usize::from(*options.get(1)?);
                if len < 2 || len > options.len() {
                    break;
                }
                if kind == 2 && len == 4 {
                    mss = Some(u16::from_be_bytes([options[2], options[3]]));
                }
                options = &options[len..];
            }
        }
    }
    Some(Syn {
        src: Ipv4Addr::new(p[12], p[13], p[14], p[15]),
        dst: Ipv4Addr::new(p[16], p[17], p[18], p[19]),
        dport,
        mss,
        ifindex: 0,
    })
}
