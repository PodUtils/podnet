//! The descriptor passing of the netns backend: one descriptor over a
//! unix socket with `SCM_RIGHTS` (the child sends the TUN to the parent).
//! pod-hold (PodUtils/podcrates) has the code.

#[allow(unused_imports)]
use crate::prelude::*;

pub(crate) use pod_hold::netns::send_fd_raw;

/// Send one fd over a unix socketpair with SCM_RIGHTS.
pub(crate) fn send_fd(sock: RawFd, fd: RawFd) -> Result<()> {
    Ok(pod_hold::netns::send_fd(sock, fd)?)
}

/// Receive one fd over a unix socketpair. A message with no descriptor or
/// with more than one is refused.
pub(crate) fn recv_fd(sock: RawFd) -> Result<RawFd> {
    use std::os::fd::IntoRawFd;
    Ok(pod_hold::netns::recv_fd(sock)?.into_raw_fd())
}
