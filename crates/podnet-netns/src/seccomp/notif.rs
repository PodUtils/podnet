//! The notification helpers of the seccomp backend. pod-hold
//! (PodUtils/podcrates) has the answers to a notification
//! (`SECCOMP_IOCTL_NOTIF_SEND`, `SECCOMP_IOCTL_NOTIF_ADDFD`), the check of a
//! notification id and the socket address codec; this module keeps the
//! names of the backend.

#[allow(unused_imports)]
use crate::prelude::*;

pub(crate) use pod_hold::reply::{child_fd_flags, id_valid as notif_id_valid, Answer};

pub(crate) fn sockaddr_in(family: u16, ip: IpAddr, port: u16) -> Vec<u8> {
    pod_hold::sockaddr::encode(family, ip, port)
}

pub(crate) fn parse_sockaddr(bytes: &[u8]) -> Option<(u16, IpAddr, u16)> {
    pod_hold::sockaddr::parse(bytes).map(|a| (a.family, a.ip, a.port))
}

pub(crate) fn set_nonblocking(fd: RawFd) -> Result<()> {
    Ok(pod_hold::reply::set_nonblocking(fd)?)
}

pub(crate) fn notif_addfd_send(lfd: RawFd, id: u64, srcfd: RawFd, newfd_flags: u32) -> Result<i32> {
    Ok(pod_hold::reply::addfd_send(lfd, id, srcfd, newfd_flags)?)
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
    Ok(pod_hold::reply::addfd_setfd(
        lfd,
        id,
        srcfd,
        target,
        newfd_flags,
    )?)
}

/// Answer the notification `id`: the system call returns `val`, or fails
/// with the errno `error` (a positive errno, for example `libc::EPERM`).
pub(crate) fn notif_respond(lfd: impl Answer, id: u64, val: i64, error: i32) -> Result<()> {
    Ok(pod_hold::reply::respond(lfd, id, val, error)?)
}

/// Let the kernel run the system call of the notification `id`.
pub(crate) fn notif_continue(lfd: impl Answer, id: u64) -> Result<()> {
    Ok(pod_hold::reply::proceed(lfd, id)?)
}
