//! The descriptor passing of the netns backend: one descriptor over a
//! unix socket with `SCM_RIGHTS` (the child sends the TUN to the parent).

#[allow(unused_imports)]
use crate::prelude::*;

// ── fd passing (child → parent) ─────────────────────────────────────────────

/// A control-message buffer with the alignment of `cmsghdr`.
///
/// A plain `[u8; N]` has alignment 1, so `CMSG_FIRSTHDR` can return a
/// misaligned `*mut cmsghdr`. A debug build aborts on that dereference.
#[repr(C, align(8))]
struct CmsgBuf([u8; 64]);

/// [`send_fd`] that does not allocate, for a child after `fork()` in a
/// multi-threaded process: 0, or -1 on an error.
pub(crate) unsafe fn send_fd_raw(sock: RawFd, fd: RawFd) -> libc::c_int {
    let mut byte = [0u8; 1];
    let mut iov = libc::iovec {
        iov_base: byte.as_mut_ptr() as *mut _,
        iov_len: 1,
    };
    let mut cmsg = CmsgBuf([0u8; 64]);
    let fd_len = std::mem::size_of::<RawFd>() as u32;
    let mut msg: libc::msghdr = std::mem::zeroed();
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = cmsg.0.as_mut_ptr() as *mut libc::c_void;
    msg.msg_controllen = libc::CMSG_SPACE(fd_len) as _;
    let hdr = libc::CMSG_FIRSTHDR(&msg);
    if hdr.is_null() {
        return -1;
    }
    (*hdr).cmsg_level = libc::SOL_SOCKET;
    (*hdr).cmsg_type = libc::SCM_RIGHTS;
    (*hdr).cmsg_len = libc::CMSG_LEN(fd_len) as _;
    std::ptr::copy_nonoverlapping(
        (&fd as *const RawFd).cast::<u8>(),
        libc::CMSG_DATA(hdr),
        fd_len as usize,
    );
    if libc::sendmsg(sock, &msg, 0) < 0 {
        -1
    } else {
        0
    }
}

/// Send one fd over a unix socketpair with SCM_RIGHTS.
pub(crate) fn send_fd(sock: RawFd, fd: RawFd) -> Result<()> {
    let mut byte = [0u8; 1];
    let mut iov = libc::iovec {
        iov_base: byte.as_mut_ptr() as *mut _,
        iov_len: 1,
    };
    let mut cmsg = CmsgBuf([0u8; 64]);
    let fd_len = std::mem::size_of::<RawFd>() as u32;
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = cmsg.0.as_mut_ptr() as *mut libc::c_void;
    msg.msg_controllen = unsafe { libc::CMSG_SPACE(fd_len) } as _;
    unsafe {
        let hdr = libc::CMSG_FIRSTHDR(&msg);
        if hdr.is_null() {
            bail!("the control buffer is too small for one fd");
        }
        (*hdr).cmsg_level = libc::SOL_SOCKET;
        (*hdr).cmsg_type = libc::SCM_RIGHTS;
        (*hdr).cmsg_len = libc::CMSG_LEN(fd_len) as _;
        std::ptr::copy_nonoverlapping(
            (&fd as *const RawFd).cast::<u8>(),
            libc::CMSG_DATA(hdr),
            fd_len as usize,
        );
        if libc::sendmsg(sock, &msg, 0) < 0 {
            bail!("sendmsg(SCM_RIGHTS): {}", std::io::Error::last_os_error());
        }
    }
    Ok(())
}

/// Receive one fd over a unix socketpair.
pub(crate) fn recv_fd(sock: RawFd) -> Result<RawFd> {
    let mut byte = [0u8; 1];
    let mut iov = libc::iovec {
        iov_base: byte.as_mut_ptr() as *mut _,
        iov_len: 1,
    };
    let mut cmsg = CmsgBuf([0u8; 64]);
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = cmsg.0.as_mut_ptr() as *mut libc::c_void;
    msg.msg_controllen = cmsg.0.len() as _;
    let n = unsafe { libc::recvmsg(sock, &mut msg, libc::MSG_CMSG_CLOEXEC) };
    if n < 0 {
        bail!("recvmsg(SCM_RIGHTS): {}", std::io::Error::last_os_error());
    }
    if n == 0 {
        bail!("the peer closed the socket before it sent a descriptor");
    }
    let hdr = unsafe { libc::CMSG_FIRSTHDR(&msg) };
    if hdr.is_null() {
        bail!("no fd in the received message");
    }
    let (level, kind) = unsafe { ((*hdr).cmsg_level, (*hdr).cmsg_type) };
    if level != libc::SOL_SOCKET || kind != libc::SCM_RIGHTS {
        bail!("the received control message is not SCM_RIGHTS");
    }
    let mut fd: RawFd = -1;
    // Copy bytes, not elements.
    unsafe {
        std::ptr::copy_nonoverlapping(
            libc::CMSG_DATA(hdr) as *const u8,
            (&mut fd as *mut RawFd).cast::<u8>(),
            std::mem::size_of::<RawFd>(),
        );
    }
    Ok(fd)
}
