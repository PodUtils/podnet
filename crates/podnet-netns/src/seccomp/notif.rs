//! The notification helpers of the seccomp backend: the answers to a
//! notification (`SECCOMP_IOCTL_NOTIF_SEND`, `SECCOMP_IOCTL_NOTIF_ADDFD`),
//! the check of a notification id, and the socket address codec.

#[allow(unused_imports)]
use crate::prelude::*;

pub(crate) fn sockaddr_in(family: u16, ip: IpAddr, port: u16) -> Vec<u8> {
    match ip {
        IpAddr::V4(v4) => {
            let mut b = vec![0u8; 16];
            b[0..2].copy_from_slice(&family.to_ne_bytes());
            b[2..4].copy_from_slice(&port.to_be_bytes());
            b[4..8].copy_from_slice(&v4.octets());
            b
        }
        IpAddr::V6(v6) => {
            let mut b = vec![0u8; 28];
            b[0..2].copy_from_slice(&family.to_ne_bytes());
            b[2..4].copy_from_slice(&port.to_be_bytes());
            b[8..24].copy_from_slice(&v6.octets());
            b
        }
    }
}

pub(crate) fn parse_sockaddr(bytes: &[u8]) -> Option<(u16, IpAddr, u16)> {
    if bytes.len() < 4 {
        return None;
    }
    let family = u16::from_ne_bytes([bytes[0], bytes[1]]);
    let port = u16::from_be_bytes([bytes[2], bytes[3]]);
    match family as i32 {
        libc::AF_INET if bytes.len() >= 8 => Some((
            family,
            IpAddr::V4(Ipv4Addr::new(bytes[4], bytes[5], bytes[6], bytes[7])),
            port,
        )),
        libc::AF_INET6 if bytes.len() >= 24 => {
            let mut o = [0u8; 16];
            o.copy_from_slice(&bytes[8..24]);
            Some((family, IpAddr::V6(o.into()), port))
        }
        _ => None,
    }
}

pub(crate) fn set_nonblocking(fd: RawFd) -> Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        bail!("fcntl(F_GETFL): {}", std::io::Error::last_os_error());
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        bail!("fcntl(F_SETFL): {}", std::io::Error::last_os_error());
    }
    Ok(())
}

pub(crate) fn notif_addfd_send(lfd: RawFd, id: u64, srcfd: RawFd, newfd_flags: u32) -> Result<i32> {
    let req = SeccompNotifAddfd {
        id,
        flags: SECCOMP_ADDFD_FLAG_SEND,
        srcfd: srcfd as u32,
        newfd: 0,
        newfd_flags,
    };
    let r = unsafe { libc::ioctl(lfd, SECCOMP_IOCTL_NOTIF_ADDFD as _, &req) };
    if r < 0 {
        return Err(std::io::Error::last_os_error()).context("NOTIF_ADDFD");
    }
    Ok(r)
}

/// Install `srcfd` in the child at the number `target`, in place of the fd
/// that has that number (the syscall is not answered).
pub(crate) fn notif_addfd_setfd(
    lfd: RawFd,
    id: u64,
    srcfd: RawFd,
    target: i32,
    newfd_flags: u32,
) -> Result<i32> {
    let req = SeccompNotifAddfd {
        id,
        flags: SECCOMP_ADDFD_FLAG_SETFD,
        srcfd: srcfd as u32,
        newfd: target as u32,
        newfd_flags,
    };
    let r = unsafe { libc::ioctl(lfd, SECCOMP_IOCTL_NOTIF_ADDFD as _, &req) };
    if r < 0 {
        return Err(std::io::Error::last_os_error()).context("NOTIF_ADDFD (SETFD)");
    }
    Ok(r)
}

/// The file status flags of the fd `fd` of the task `pid`
/// (`/proc/<pid>/fdinfo/<fd>`, the line `flags:` in octal).
pub(crate) fn child_fd_flags(pid: u32, fd: i32) -> Option<i32> {
    let text = std::fs::read_to_string(format!("/proc/{pid}/fdinfo/{fd}")).ok()?;
    let value = text.lines().find_map(|l| l.strip_prefix("flags:"))?;
    i32::from_str_radix(value.trim(), 8).ok()
}

/// Answer the notification `id`: the system call returns `val`, or fails
/// with the errno `error` (a positive errno, for example `libc::EPERM`).
pub(crate) fn notif_respond(lfd: RawFd, id: u64, val: i64, error: i32) -> Result<()> {
    let resp = SeccompNotifResp {
        id,
        val,
        // The kernel takes a nonzero error as the return value of the system
        // call, so it must be the negative errno. Before (issue 98), a
        // positive errno was a positive return: getpeername returned 95
        // (EOPNOTSUPP), connect returned 111 (ECONNREFUSED), as successes.
        error: -error.abs(),
        flags: 0,
    };
    let r = unsafe { libc::ioctl(lfd, SECCOMP_IOCTL_NOTIF_SEND as _, &resp) };
    if r < 0 {
        let e = std::io::Error::last_os_error();
        // ENOENT means the target died or the syscall was interrupted.
        if e.raw_os_error() == Some(libc::ENOENT) {
            return Ok(());
        }
        return Err(e).context("NOTIF_SEND");
    }
    Ok(())
}

pub(crate) fn notif_continue(lfd: RawFd, id: u64) -> Result<()> {
    let resp = SeccompNotifResp {
        id,
        val: 0,
        error: 0,
        flags: SECCOMP_USER_NOTIF_FLAG_CONTINUE,
    };
    let r = unsafe { libc::ioctl(lfd, SECCOMP_IOCTL_NOTIF_SEND as _, &resp) };
    if r < 0 {
        let e = std::io::Error::last_os_error();
        if e.raw_os_error() == Some(libc::ENOENT) {
            return Ok(());
        }
        return Err(e).context("NOTIF_SEND(CONTINUE)");
    }
    Ok(())
}

/// `SECCOMP_IOCTL_NOTIF_ID_VALID`: `_IOW('!', 2, __u64)`.
pub(crate) const SECCOMP_IOCTL_NOTIF_ID_VALID: libc::c_ulong = 0x4008_2102;

/// Return true while the notification `id` is still pending.
pub(crate) fn notif_id_valid(lfd: RawFd, id: u64) -> bool {
    let mut id = id;
    unsafe { libc::ioctl(lfd, SECCOMP_IOCTL_NOTIF_ID_VALID as _, &mut id) == 0 }
}
