//! The veth wire of the netns backend (T-174). When the namespace cannot
//! open `/dev/net/tun`, the child makes a veth pair in the namespace: the
//! program end has the address and the default route of the TUN, and an
//! `AF_PACKET` socket on the wire end carries its frames to the stack. The
//! wire translates between frames and IP packets: the stack keeps its IP
//! medium, and the wire answers ARP for the gateway.

#[allow(unused_imports)]
use crate::prelude::*;

/// The MAC address of the program end of the pair.
pub(crate) const GUEST_MAC: [u8; 6] = [0x02, 0x70, 0x6e, 0, 0, 0x02];
/// The MAC address that the stack uses for the gateway. It is not the
/// address of the wire end, so the kernel of the namespace drops the frames
/// to the gateway on the wire end, and only the packet socket reads them.
pub(crate) const GATEWAY_MAC: [u8; 6] = [0x02, 0x70, 0x6e, 0, 0, 0x01];
/// The name of the wire end in the namespace.
pub(crate) const WIRE_NAME: &str = "wire0";
/// The length of an Ethernet header.
pub(crate) const ETH_HEADER: usize = 14;

const ETH_P_IP: u16 = 0x0800;
const ETH_P_ARP: u16 = 0x0806;
const ETH_P_IPV6: u16 = 0x86dd;

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
    if veth_forced() {
        return create_veth(name, WIRE_NAME);
    }
    match create_tun(name) {
        Ok(fd) => Ok(fd),
        Err(tun) => create_veth(name, WIRE_NAME)
            .map_err(|veth| anyhow::anyhow!("{tun:#}; the veth wire: {veth:#}")),
    }
}

/// Make the veth pair `guest` and `wire` in the namespace. Configure
/// `guest` as the TUN (address, MTU, up, default route), bring up `wire`,
/// and return a packet socket bound to `wire`.
pub(crate) fn create_veth(guest: &str, wire: &str) -> Result<OwnedFd> {
    new_link(guest, wire, TUN_MTU as u32)?;
    let sock = control()?;
    configure(&sock, guest)?;
    // The frames of the program must have whole checksums and no segment
    // larger than the MTU: turn off the offloads of the program end. A
    // failure is not fatal: the stack does not check the checksums of this
    // wire, and a frame larger than the MTU is dropped.
    for cmd in [ETHTOOL_STXCSUM, ETHTOOL_STSO, ETHTOOL_SGSO] {
        let _ = ethtool_off(&sock, guest, cmd);
    }
    set_up(&sock, wire)?;
    packet_socket(wire)
}

// ── RTM_NEWLINK ─────────────────────────────────────────────────────────────

const RTM_NEWLINK: u16 = 16;
const NLMSG_ERROR: u16 = 2;
const NLM_F_REQUEST: u16 = 0x1;
const NLM_F_ACK: u16 = 0x4;
const NLM_F_EXCL: u16 = 0x200;
const NLM_F_CREATE: u16 = 0x400;
const IFLA_ADDRESS: u16 = 1;
const IFLA_IFNAME: u16 = 3;
const IFLA_MTU: u16 = 4;
const IFLA_LINKINFO: u16 = 18;
const IFLA_INFO_KIND: u16 = 1;
const IFLA_INFO_DATA: u16 = 2;
const VETH_INFO_PEER: u16 = 1;
/// The length of `struct ifinfomsg`.
const IFINFOMSG: usize = 16;

/// Add one netlink attribute, with its padding to 4 bytes.
fn attr(buf: &mut Vec<u8>, kind: u16, data: &[u8]) {
    buf.extend_from_slice(&((4 + data.len()) as u16).to_ne_bytes());
    buf.extend_from_slice(&kind.to_ne_bytes());
    buf.extend_from_slice(data);
    buf.resize(buf.len().next_multiple_of(4), 0);
}

/// Add one nested netlink attribute: `inner` writes its content.
fn nested(buf: &mut Vec<u8>, kind: u16, inner: impl FnOnce(&mut Vec<u8>)) {
    let start = buf.len();
    buf.extend_from_slice(&[0; 4]);
    inner(buf);
    let len = (buf.len() - start) as u16;
    buf[start..start + 2].copy_from_slice(&len.to_ne_bytes());
    buf[start + 2..start + 4].copy_from_slice(&kind.to_ne_bytes());
}

fn name_bytes(name: &str) -> Vec<u8> {
    let mut bytes = name.as_bytes().to_vec();
    bytes.push(0);
    bytes
}

/// The `RTM_NEWLINK` request of a veth pair `guest` (with [`GUEST_MAC`])
/// and `wire`, each with the MTU `mtu`.
pub(crate) fn newlink_request(guest: &str, wire: &str, mtu: u32) -> Vec<u8> {
    let mut msg = vec![0u8; 16 + IFINFOMSG];
    attr(&mut msg, IFLA_IFNAME, &name_bytes(guest));
    attr(&mut msg, IFLA_ADDRESS, &GUEST_MAC);
    attr(&mut msg, IFLA_MTU, &mtu.to_ne_bytes());
    nested(&mut msg, IFLA_LINKINFO, |b| {
        attr(b, IFLA_INFO_KIND, b"veth");
        nested(b, IFLA_INFO_DATA, |b| {
            nested(b, VETH_INFO_PEER, |b| {
                b.extend_from_slice(&[0; IFINFOMSG]);
                attr(b, IFLA_IFNAME, &name_bytes(wire));
                attr(b, IFLA_MTU, &mtu.to_ne_bytes());
            });
        });
    });
    let len = msg.len() as u32;
    let flags = NLM_F_REQUEST | NLM_F_ACK | NLM_F_EXCL | NLM_F_CREATE;
    msg[0..4].copy_from_slice(&len.to_ne_bytes());
    msg[4..6].copy_from_slice(&RTM_NEWLINK.to_ne_bytes());
    msg[6..8].copy_from_slice(&flags.to_ne_bytes());
    msg[8..12].copy_from_slice(&1u32.to_ne_bytes());
    msg
}

/// Make the veth pair with one `RTM_NEWLINK` request, and read its answer.
fn new_link(guest: &str, wire: &str, mtu: u32) -> Result<()> {
    // SAFETY: socket with constant arguments.
    let fd = unsafe {
        libc::socket(
            libc::AF_NETLINK,
            libc::SOCK_RAW | libc::SOCK_CLOEXEC,
            libc::NETLINK_ROUTE,
        )
    };
    if fd < 0 {
        bail!("netlink socket: {}", std::io::Error::last_os_error());
    }
    // SAFETY: a new descriptor that nothing else owns.
    let sock = unsafe { OwnedFd::from_raw_fd(fd) };
    let msg = newlink_request(guest, wire, mtu);
    // SAFETY: a zeroed sockaddr_nl is valid; the kernel is pid 0.
    let mut kernel: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
    kernel.nl_family = libc::AF_NETLINK as libc::sa_family_t;
    // SAFETY: `msg` and `kernel` live for the call.
    let sent = unsafe {
        libc::sendto(
            sock.as_raw_fd(),
            msg.as_ptr() as *const libc::c_void,
            msg.len(),
            0,
            &kernel as *const libc::sockaddr_nl as *const libc::sockaddr,
            std::mem::size_of::<libc::sockaddr_nl>() as libc::socklen_t,
        )
    };
    if sent < 0 {
        bail!("RTM_NEWLINK veth: {}", std::io::Error::last_os_error());
    }
    let mut answer = [0u8; 1024];
    // SAFETY: `answer` has room for the read.
    let n = unsafe {
        libc::recv(
            sock.as_raw_fd(),
            answer.as_mut_ptr() as *mut libc::c_void,
            answer.len(),
            0,
        )
    };
    if n < 0 {
        bail!(
            "RTM_NEWLINK veth answer: {}",
            std::io::Error::last_os_error()
        );
    }
    match netlink_error(&answer[..n as usize]) {
        Some(0) => Ok(()),
        Some(code) => bail!(
            "RTM_NEWLINK veth: {}",
            std::io::Error::from_raw_os_error(-code)
        ),
        None => bail!("RTM_NEWLINK veth: no acknowledgement"),
    }
}

/// The error code of a netlink acknowledgement (0 is success).
pub(crate) fn netlink_error(answer: &[u8]) -> Option<i32> {
    if answer.len() < 20 || u16::from_ne_bytes([answer[4], answer[5]]) != NLMSG_ERROR {
        return None;
    }
    Some(i32::from_ne_bytes([
        answer[16], answer[17], answer[18], answer[19],
    ]))
}

// ── the ioctls of the interfaces ────────────────────────────────────────────

const ETHTOOL_STXCSUM: u32 = 0x17;
const ETHTOOL_STSO: u32 = 0x1f;
const ETHTOOL_SGSO: u32 = 0x24;

/// A datagram socket for the ioctls of the interfaces.
fn control() -> Result<OwnedFd> {
    // SAFETY: socket with constant arguments.
    let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0) };
    if fd < 0 {
        bail!(
            "socket() for the interfaces: {}",
            std::io::Error::last_os_error()
        );
    }
    // SAFETY: a new descriptor that nothing else owns.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

fn ioctl(sock: &OwnedFd, cmd: libc::c_ulong, ifr: &mut libc::ifreq, what: &str) -> Result<()> {
    // SAFETY: a valid ifreq for the request.
    if unsafe { libc::ioctl(sock.as_raw_fd(), cmd as _, ifr as *mut libc::ifreq) } < 0 {
        bail!("ioctl {what}: {}", std::io::Error::last_os_error());
    }
    Ok(())
}

fn set_up(sock: &OwnedFd, name: &str) -> Result<()> {
    let mut ifr = pod_hold::netns::ifreq_for(name);
    ifr.ifr_ifru.ifru_flags = (libc::IFF_UP | libc::IFF_RUNNING) as libc::c_short;
    ioctl(sock, libc::SIOCSIFFLAGS as _, &mut ifr, "SIOCSIFFLAGS up")
}

/// Turn off one offload of `name` (an `ETHTOOL_S*` command).
fn ethtool_off(sock: &OwnedFd, name: &str, cmd: u32) -> Result<()> {
    let mut value = [cmd, 0u32];
    let mut ifr = pod_hold::netns::ifreq_for(name);
    ifr.ifr_ifru.ifru_data = value.as_mut_ptr() as *mut libc::c_char;
    ioctl(sock, libc::SIOCETHTOOL as _, &mut ifr, "SIOCETHTOOL")
}

/// Give `name` the address of the TUN, bring it up, and add the default
/// route through the gateway.
fn configure(sock: &OwnedFd, name: &str) -> Result<()> {
    let tun = pod_hold::netns::Tun {
        name: name.into(),
        addr: TUN_ADDR,
        gateway: TUN_GW,
        prefix: TUN_PREFIX,
        mtu: TUN_MTU,
    };
    for (cmd, addr, what) in [
        (libc::SIOCSIFADDR, tun.addr, "SIOCSIFADDR"),
        (libc::SIOCSIFNETMASK, tun.netmask(), "SIOCSIFNETMASK"),
    ] {
        let mut ifr = pod_hold::netns::ifreq_for(name);
        // SAFETY: ifru_addr has the room of a sockaddr_in.
        unsafe {
            let sin = &mut ifr.ifr_ifru.ifru_addr as *mut libc::sockaddr as *mut libc::sockaddr_in;
            (*sin).sin_family = libc::AF_INET as libc::sa_family_t;
            (*sin).sin_addr.s_addr = u32::from_ne_bytes(addr.octets());
        }
        ioctl(sock, cmd as _, &mut ifr, what)?;
    }
    set_up(sock, name)?;
    // SAFETY: a zeroed rtentry is valid.
    let mut route: libc::rtentry = unsafe { std::mem::zeroed() };
    let dst = &mut route.rt_dst as *mut libc::sockaddr as *mut libc::sockaddr_in;
    let gw = &mut route.rt_gateway as *mut libc::sockaddr as *mut libc::sockaddr_in;
    let mask = &mut route.rt_genmask as *mut libc::sockaddr as *mut libc::sockaddr_in;
    // SAFETY: each field has the room of a sockaddr_in.
    unsafe {
        (*dst).sin_family = libc::AF_INET as libc::sa_family_t;
        (*gw).sin_family = libc::AF_INET as libc::sa_family_t;
        (*gw).sin_addr.s_addr = u32::from_ne_bytes(tun.gateway.octets());
        (*mask).sin_family = libc::AF_INET as libc::sa_family_t;
    }
    route.rt_flags = libc::RTF_UP | libc::RTF_GATEWAY;
    let dev = CString::new(name)?;
    route.rt_dev = dev.as_ptr() as *mut libc::c_char;
    // SAFETY: a valid rtentry; `dev` lives for the call.
    if unsafe { libc::ioctl(sock.as_raw_fd(), libc::SIOCADDRT as _, &route as *const _) } < 0 {
        bail!("ioctl SIOCADDRT: {}", std::io::Error::last_os_error());
    }
    Ok(())
}

/// The option of `SOL_PACKET` that drops the frames that the host sends.
const PACKET_IGNORE_OUTGOING: libc::c_int = 23;

/// A packet socket bound to `name`, for all protocols.
fn packet_socket(name: &str) -> Result<OwnedFd> {
    let protocol = (libc::ETH_P_ALL as u16).to_be();
    // SAFETY: socket with constant arguments.
    let fd = unsafe {
        libc::socket(
            libc::AF_PACKET,
            libc::SOCK_RAW | libc::SOCK_CLOEXEC,
            libc::c_int::from(protocol),
        )
    };
    if fd < 0 {
        bail!("AF_PACKET socket: {}", std::io::Error::last_os_error());
    }
    // SAFETY: a new descriptor that nothing else owns.
    let sock = unsafe { OwnedFd::from_raw_fd(fd) };
    let c_name = CString::new(name)?;
    // SAFETY: a valid C string.
    let index = unsafe { libc::if_nametoindex(c_name.as_ptr()) };
    if index == 0 {
        bail!("if_nametoindex {name}: {}", std::io::Error::last_os_error());
    }
    // SAFETY: a zeroed sockaddr_ll is valid.
    let mut addr: libc::sockaddr_ll = unsafe { std::mem::zeroed() };
    addr.sll_family = libc::AF_PACKET as libc::c_ushort;
    addr.sll_protocol = protocol;
    addr.sll_ifindex = index as libc::c_int;
    // SAFETY: `addr` lives for the call.
    let r = unsafe {
        libc::bind(
            sock.as_raw_fd(),
            &addr as *const libc::sockaddr_ll as *const libc::sockaddr,
            std::mem::size_of::<libc::sockaddr_ll>() as libc::socklen_t,
        )
    };
    if r < 0 {
        bail!("bind AF_PACKET {name}: {}", std::io::Error::last_os_error());
    }
    let on: libc::c_int = 1;
    // SAFETY: an int option. An old kernel does not know it: the frames of
    // the gateway MAC are dropped in `take_frame` too.
    unsafe {
        libc::setsockopt(
            sock.as_raw_fd(),
            libc::SOL_PACKET,
            PACKET_IGNORE_OUTGOING,
            &on as *const libc::c_int as *const libc::c_void,
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        )
    };
    Ok(sock)
}

/// True when `fd` is a socket (the veth wire), not the TUN.
pub(crate) fn is_veth(fd: RawFd) -> bool {
    // SAFETY: a zeroed stat is valid; fstat fills it.
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: `st` lives for the call.
    unsafe { libc::fstat(fd, &mut st) == 0 && st.st_mode & libc::S_IFMT == libc::S_IFSOCK }
}

// ── frames ──────────────────────────────────────────────────────────────────

/// What the wire does with one frame of the program.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Frame {
    /// An IP packet for the stack.
    Ip(Vec<u8>),
    /// A frame to send back at once (the ARP reply of the gateway).
    Reply(Vec<u8>),
    /// Not for the stack.
    Drop,
}

/// Take one frame of the program end.
pub(crate) fn take_frame(frame: &[u8], mtu: usize) -> Frame {
    if frame.len() < ETH_HEADER || frame.len() > mtu + ETH_HEADER {
        return Frame::Drop;
    }
    let (dst, src) = (&frame[0..6], &frame[6..12]);
    // The own frames, and frames to another host.
    if src == GATEWAY_MAC || (dst != GATEWAY_MAC && dst != [0xff; 6]) {
        return Frame::Drop;
    }
    let payload = &frame[ETH_HEADER..];
    match u16::from_be_bytes([frame[12], frame[13]]) {
        ETH_P_ARP => arp_reply(src, payload).map_or(Frame::Drop, Frame::Reply),
        ETH_P_IP if payload.len() >= 20 && payload[0] >> 4 == 4 => {
            let total = usize::from(u16::from_be_bytes([payload[2], payload[3]]));
            match total >= 20 && total <= payload.len() {
                // An Ethernet frame can have padding after the packet.
                true => Frame::Ip(payload[..total].to_vec()),
                false => Frame::Drop,
            }
        }
        ETH_P_IPV6 if payload.len() >= 40 && payload[0] >> 4 == 6 => {
            let total = 40 + usize::from(u16::from_be_bytes([payload[4], payload[5]]));
            match total <= payload.len() {
                true => Frame::Ip(payload[..total].to_vec()),
                false => Frame::Drop,
            }
        }
        _ => Frame::Drop,
    }
}

/// The ARP reply to a request of `sender` for the gateway.
fn arp_reply(sender: &[u8], arp: &[u8]) -> Option<Vec<u8>> {
    // Ethernet and IPv4, a request, for the gateway.
    if arp.len() < 28 || arp[0..8] != [0, 1, 8, 0, 6, 4, 0, 1] || arp[24..28] != TUN_GW.octets() {
        return None;
    }
    let mut out = Vec::with_capacity(ETH_HEADER + 28);
    out.extend_from_slice(sender);
    out.extend_from_slice(&GATEWAY_MAC);
    out.extend_from_slice(&ETH_P_ARP.to_be_bytes());
    out.extend_from_slice(&[0, 1, 8, 0, 6, 4, 0, 2]);
    out.extend_from_slice(&GATEWAY_MAC);
    out.extend_from_slice(&TUN_GW.octets());
    out.extend_from_slice(&arp[8..14]); // the hardware address of the sender
    out.extend_from_slice(&arp[14..18]); // the address of the sender
    Some(out)
}

/// The frame of one IP packet of the stack to the program end.
pub(crate) fn frame_for(packet: &[u8]) -> Option<Vec<u8>> {
    let kind = match packet.first()? >> 4 {
        4 => ETH_P_IP,
        6 => ETH_P_IPV6,
        _ => return None,
    };
    let mut out = Vec::with_capacity(ETH_HEADER + packet.len());
    out.extend_from_slice(&GUEST_MAC);
    out.extend_from_slice(&GATEWAY_MAC);
    out.extend_from_slice(&kind.to_be_bytes());
    out.extend_from_slice(packet);
    Some(out)
}

#[cfg(test)]
mod tests;
