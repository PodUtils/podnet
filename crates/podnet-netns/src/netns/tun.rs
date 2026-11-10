//! The TUN device of the netns backend: its setup inside the namespace,
//! and the device that the `smoltcp` stack reads and writes.

#[allow(unused_imports)]
use crate::prelude::*;

pub(crate) const TUNSETIFF: libc::c_ulong = 0x4004_54CA;
pub(crate) const IFF_TUN: libc::c_int = 0x0001;
pub(crate) const IFF_NO_PI: libc::c_int = 0x1000;

pub(crate) fn ifreq_for(name: &str) -> libc::ifreq {
    let mut ifr: libc::ifreq = unsafe { std::mem::zeroed() };
    for (i, b) in name.bytes().enumerate().take(libc::IF_NAMESIZE - 1) {
        ifr.ifr_name[i] = b as libc::c_char;
    }
    ifr
}

pub(crate) fn bringup_loopback() -> Result<()> {
    let sock = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0) };
    if sock < 0 {
        bail!("socket() for lo: {}", std::io::Error::last_os_error());
    }
    let mut ifr = ifreq_for("lo");
    let r = unsafe {
        ifr.ifr_ifru.ifru_flags = (libc::IFF_UP | libc::IFF_RUNNING) as i16;
        libc::ioctl(sock, libc::SIOCSIFFLAGS as _, &ifr as *const _)
    };
    unsafe { libc::close(sock) };
    if r < 0 {
        bail!("ioctl SIOCSIFFLAGS lo: {}", std::io::Error::last_os_error());
    }
    Ok(())
}

/// Create and configure the TUN device inside the namespace. Returns its fd.
pub(crate) fn create_tun(name: &str) -> Result<OwnedFd> {
    if name.len() >= libc::IF_NAMESIZE {
        bail!("TUN name {name:?} is too long");
    }
    let fd = unsafe { libc::open(c"/dev/net/tun".as_ptr(), libc::O_RDWR | libc::O_CLOEXEC) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error()).context("open /dev/net/tun");
    }
    let owned = unsafe { OwnedFd::from_raw_fd(fd) };

    let mut ifr = ifreq_for(name);
    let r = unsafe {
        ifr.ifr_ifru.ifru_flags = (IFF_TUN | IFF_NO_PI) as libc::c_short;
        libc::ioctl(
            owned.as_raw_fd(),
            TUNSETIFF as _,
            &mut ifr as *mut libc::ifreq,
        )
    };
    if r < 0 {
        return Err(std::io::Error::last_os_error()).context("ioctl TUNSETIFF");
    }

    // Configure with ioctls rather than `ip`: RTNETLINK can fail inside a user
    // namespace even when the ioctls succeed.
    let sock = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0) };
    if sock < 0 {
        bail!(
            "socket() for tun config: {}",
            std::io::Error::last_os_error()
        );
    }
    let guard = FdGuard(sock);
    let set_addr = |cmd: libc::c_ulong, addr: Ipv4Addr| -> Result<()> {
        let mut ifr = ifreq_for(name);
        let r = unsafe {
            let sin = &mut ifr.ifr_ifru.ifru_addr as *mut libc::sockaddr as *mut libc::sockaddr_in;
            (*sin).sin_family = libc::AF_INET as u16;
            (*sin).sin_addr.s_addr = u32::from_ne_bytes(addr.octets());
            libc::ioctl(guard.0, cmd as _, &ifr as *const _)
        };
        if r < 0 {
            bail!("ioctl {cmd:#x}: {}", std::io::Error::last_os_error());
        }
        Ok(())
    };

    let mut mtu_ifr = ifreq_for(name);
    let mtu_r = unsafe {
        mtu_ifr.ifr_ifru.ifru_mtu = TUN_MTU as libc::c_int;
        libc::ioctl(guard.0, libc::SIOCSIFMTU as _, &mtu_ifr as *const _)
    };
    if mtu_r < 0 {
        bail!("ioctl SIOCSIFMTU: {}", std::io::Error::last_os_error());
    }
    set_addr(libc::SIOCSIFADDR, TUN_ADDR)?;
    let netmask = u32::MAX << (32 - TUN_PREFIX as u32);
    set_addr(libc::SIOCSIFNETMASK, Ipv4Addr::from(netmask.to_be_bytes()))?;

    let mut up = ifreq_for(name);
    let up_r = unsafe {
        up.ifr_ifru.ifru_flags = (libc::IFF_UP | libc::IFF_RUNNING) as i16;
        libc::ioctl(guard.0, libc::SIOCSIFFLAGS as _, &up as *const _)
    };
    if up_r < 0 {
        bail!("ioctl SIOCSIFFLAGS up: {}", std::io::Error::last_os_error());
    }

    // Default route via the gateway the userspace stack impersonates.
    let mut route: libc::rtentry = unsafe { std::mem::zeroed() };
    let dst = &mut route.rt_dst as *mut libc::sockaddr as *mut libc::sockaddr_in;
    let gw = &mut route.rt_gateway as *mut libc::sockaddr as *mut libc::sockaddr_in;
    let mask = &mut route.rt_genmask as *mut libc::sockaddr as *mut libc::sockaddr_in;
    unsafe {
        (*dst).sin_family = libc::AF_INET as u16;
        (*gw).sin_family = libc::AF_INET as u16;
        (*gw).sin_addr.s_addr = u32::from_ne_bytes(TUN_GW.octets());
        (*mask).sin_family = libc::AF_INET as u16;
    }
    route.rt_flags = libc::RTF_UP | libc::RTF_GATEWAY;
    let name_c = CString::new(name)?;
    route.rt_dev = name_c.as_ptr() as *mut libc::c_char;
    if unsafe { libc::ioctl(guard.0, libc::SIOCADDRT as _, &route as *const _) } < 0 {
        bail!("ioctl SIOCADDRT: {}", std::io::Error::last_os_error());
    }
    Ok(owned)
}

pub(crate) struct FdGuard(RawFd);
impl Drop for FdGuard {
    fn drop(&mut self) {
        unsafe { libc::close(self.0) };
    }
}

// ── TUN as a smoltcp device ─────────────────────────────────────────────────

pub(crate) struct Tun {
    pub(crate) fd: RawFd,
    pub(crate) mtu: usize,
    pub(crate) rx: VecDeque<Vec<u8>>,
    pub(crate) tx: VecDeque<Vec<u8>>,
}

impl Tun {
    pub(crate) fn new(fd: RawFd, mtu: usize) -> Self {
        // The read loop drains until EAGAIN, so the fd must not block.
        unsafe { libc::fcntl(fd, libc::F_SETFL, libc::O_NONBLOCK) };
        Self {
            fd,
            mtu,
            rx: VecDeque::new(),
            tx: VecDeque::new(),
        }
    }

    /// Read whatever the kernel has queued into `rx`.
    pub(crate) fn read_ready(&mut self) {
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

    pub(crate) fn flush(&mut self) {
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

impl Device for Tun {
    type RxToken<'a> = TunRx;
    type TxToken<'a> = TunTx<'a>;
    fn receive(&mut self, _t: SmolInstant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        let pkt = self.rx.pop_front()?;
        Some((TunRx(pkt), TunTx { out: &mut self.tx }))
    }
    fn transmit(&mut self, _t: SmolInstant) -> Option<Self::TxToken<'_>> {
        Some(TunTx { out: &mut self.tx })
    }
    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.medium = Medium::Ip;
        caps.max_transmission_unit = self.mtu;
        caps.checksum = ChecksumCapabilities::default();
        caps
    }
}

pub(crate) struct TunRx(Vec<u8>);
impl RxToken for TunRx {
    fn consume<R, F: FnOnce(&[u8]) -> R>(self, f: F) -> R {
        f(&self.0)
    }
}
pub(crate) struct TunTx<'a> {
    pub(crate) out: &'a mut VecDeque<Vec<u8>>,
}
impl TxToken for TunTx<'_> {
    fn consume<R, F: FnOnce(&mut [u8]) -> R>(self, len: usize, f: F) -> R {
        let mut buf = vec![0u8; len];
        let r = f(&mut buf);
        self.out.push_back(buf);
        r
    }
}
