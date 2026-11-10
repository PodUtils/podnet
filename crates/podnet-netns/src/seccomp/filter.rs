//! The seccomp filter of the pod: the kernel structures of the user
//! notification, the list of the stopped system calls and the BPF program.

#[allow(unused_imports)]
use crate::prelude::*;

pub(crate) const SECCOMP_SET_MODE_FILTER: libc::c_uint = 1;
pub(crate) const SECCOMP_FILTER_FLAG_NEW_LISTENER: libc::c_uint = 8;
pub(crate) const SECCOMP_IOCTL_NOTIF_RECV: libc::c_ulong = 0xc050_2100;
pub(crate) const SECCOMP_IOCTL_NOTIF_SEND: libc::c_ulong = 0xc018_2101;
pub(crate) const SECCOMP_IOCTL_NOTIF_ADDFD: libc::c_ulong = 0x4018_2103;
pub(crate) const SECCOMP_USER_NOTIF_FLAG_CONTINUE: u32 = 1;
pub(crate) const SECCOMP_ADDFD_FLAG_SEND: u32 = 2;
/// `SECCOMP_ADDFD_FLAG_SETFD`: install the fd at the number in `newfd`,
/// in place of the fd that has that number.
pub(crate) const SECCOMP_ADDFD_FLAG_SETFD: u32 = 1;

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct SeccompData {
    pub(crate) nr: i32,
    pub(crate) arch: u32,
    pub(crate) instruction_pointer: u64,
    pub(crate) args: [u64; 6],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct SeccompNotif {
    pub(crate) id: u64,
    pub(crate) pid: u32,
    pub(crate) flags: u32,
    pub(crate) data: SeccompData,
}

#[repr(C)]
pub(crate) struct SeccompNotifResp {
    pub(crate) id: u64,
    pub(crate) val: i64,
    pub(crate) error: i32,
    pub(crate) flags: u32,
}

#[repr(C)]
pub(crate) struct SeccompNotifAddfd {
    pub(crate) id: u64,
    pub(crate) flags: u32,
    pub(crate) srcfd: u32,
    pub(crate) newfd: u32,
    pub(crate) newfd_flags: u32,
}

/// The syscalls the filter stops. Everything else is allowed untouched.
pub(crate) fn intercepted_syscalls() -> [libc::c_long; 21] {
    [
        libc::SYS_socket,
        libc::SYS_recvfrom,
        libc::SYS_recvmsg,
        libc::SYS_recvmmsg,
        libc::SYS_connect,
        libc::SYS_sendto,
        libc::SYS_sendmsg,
        libc::SYS_sendmmsg,
        libc::SYS_bind,
        libc::SYS_getsockname,
        libc::SYS_getpeername,
        libc::SYS_setsockopt,
        libc::SYS_getsockopt,
        libc::SYS_io_uring_setup,
        libc::SYS_close,
        libc::SYS_listen,
        SYS_ACCEPT,
        libc::SYS_accept4,
        libc::SYS_io_uring_enter,
        libc::SYS_io_uring_register,
        // T-223: an unconnected TCP socket gets ENOTCONN.
        libc::SYS_shutdown,
    ]
}

/// accept(2). i386 has no direct accept system call (only accept4, and
/// accept through socketcall), so the entry is accept4 there.
#[cfg(not(target_arch = "x86"))]
pub(crate) const SYS_ACCEPT: libc::c_long = libc::SYS_accept;
#[cfg(target_arch = "x86")]
pub(crate) const SYS_ACCEPT: libc::c_long = libc::SYS_accept4;

/// The audit arch value of this build target.
#[cfg(target_arch = "x86_64")]
pub(crate) const AUDIT_ARCH_NATIVE: u32 = 0xC000_003E;
#[cfg(target_arch = "aarch64")]
pub(crate) const AUDIT_ARCH_NATIVE: u32 = 0xC000_00B7;
#[cfg(target_arch = "arm")]
pub(crate) const AUDIT_ARCH_NATIVE: u32 = 0x4000_0028;
#[cfg(target_arch = "riscv64")]
pub(crate) const AUDIT_ARCH_NATIVE: u32 = 0xC000_00F3;
#[cfg(target_arch = "x86")]
pub(crate) const AUDIT_ARCH_NATIVE: u32 = 0x4000_0003;
#[cfg(target_arch = "loongarch64")]
pub(crate) const AUDIT_ARCH_NATIVE: u32 = 0xC000_0102;
#[cfg(all(target_arch = "powerpc64", target_endian = "little"))]
pub(crate) const AUDIT_ARCH_NATIVE: u32 = 0xC000_0015;
#[cfg(not(any(
    target_arch = "x86_64",
    target_arch = "aarch64",
    target_arch = "arm",
    target_arch = "riscv64",
    target_arch = "x86",
    target_arch = "loongarch64",
    all(target_arch = "powerpc64", target_endian = "little")
)))]
compile_error!("podnet-netns: add the AUDIT_ARCH value for this target");

/// The x32 system-call bit on x86_64. Zero on other targets.
#[cfg(target_arch = "x86_64")]
pub(crate) const X32_SYSCALL_BIT: u32 = 0x4000_0000;
#[cfg(not(target_arch = "x86_64"))]
pub(crate) const X32_SYSCALL_BIT: u32 = 0;

pub(crate) fn install_notif_filter() -> Result<RawFd> {
    const BPF_LD_ABS_W: u16 = 0x20;
    const BPF_JEQ_K: u16 = 0x15;
    const BPF_JGE_K: u16 = 0x35;
    const BPF_RET_K: u16 = 0x06;
    const RET_USER_NOTIF: u32 = 0x7fc0_0000;
    const RET_ALLOW: u32 = 0x7fff_0000;
    const RET_ERRNO: u32 = 0x0005_0000;
    // seccomp_data: nr at offset 0, arch at offset 4.
    const OFFSET_ARCH: u32 = 4;
    // seccomp_data: args[] (u64 each) at offset 16.
    const OFFSET_ARG4: u32 = 16 + 4 * 8;

    // An unprivileged task can install a filter only with no_new_privs set.
    // The flag is inherited by the child and kept across execve.
    if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0 {
        return Err(std::io::Error::last_os_error()).context("prctl(PR_SET_NO_NEW_PRIVS)");
    }

    let stmt = |code, k| libc::sock_filter {
        code,
        jt: 0,
        jf: 0,
        k,
    };
    // A system call from a different ABI (i386 on x86_64, for example) has
    // different numbers and would pass the list below. Refuse it.
    let mut filter = vec![
        stmt(BPF_LD_ABS_W, OFFSET_ARCH),
        libc::sock_filter {
            code: BPF_JEQ_K,
            jt: 1,
            jf: 0,
            k: AUDIT_ARCH_NATIVE,
        },
        stmt(BPF_RET_K, RET_ERRNO | libc::ENOSYS as u32),
        stmt(BPF_LD_ABS_W, 0),
    ];
    if X32_SYSCALL_BIT != 0 {
        // x32 system calls use the native arch value with this bit set.
        filter.push(libc::sock_filter {
            code: BPF_JGE_K,
            jt: 0,
            jf: 1,
            k: X32_SYSCALL_BIT,
        });
        filter.push(stmt(BPF_RET_K, RET_ERRNO | libc::ENOSYS as u32));
    }
    // socketcall(2) multiplexes the socket calls on i386 and powerpc64. The
    // list below names the direct calls only, so socketcall fails: a program
    // then cannot pass around the proxy (the C libraries use the direct
    // calls on the kernels that have them).
    #[cfg(any(target_arch = "x86", target_arch = "powerpc64"))]
    {
        filter.push(libc::sock_filter {
            code: BPF_JEQ_K,
            jt: 0,
            jf: 1,
            k: libc::SYS_socketcall as u32,
        });
        filter.push(stmt(BPF_RET_K, RET_ERRNO | libc::ENOSYS as u32));
    }
    for syscall in intercepted_syscalls() {
        if syscall == libc::SYS_recvfrom {
            // A recvfrom with no source buffer (recv(2), the reads of a TCP
            // program) needs no source address: the kernel runs it, as a
            // read(2). The inbox keeps the order of the sources across such
            // a read (issue 305). The pointer is args[4]; the targets are
            // little-endian, so its low word is first.
            filter.push(libc::sock_filter {
                code: BPF_JEQ_K,
                jt: 0,
                jf: 6,
                k: syscall as u32,
            });
            filter.push(stmt(BPF_LD_ABS_W, OFFSET_ARG4));
            filter.push(libc::sock_filter {
                code: BPF_JEQ_K,
                jt: 0,
                jf: 3,
                k: 0,
            });
            filter.push(stmt(BPF_LD_ABS_W, OFFSET_ARG4 + 4));
            filter.push(libc::sock_filter {
                code: BPF_JEQ_K,
                jt: 0,
                jf: 1,
                k: 0,
            });
            filter.push(stmt(BPF_RET_K, RET_ALLOW));
            filter.push(stmt(BPF_RET_K, RET_USER_NOTIF));
            continue;
        }
        filter.push(libc::sock_filter {
            code: BPF_JEQ_K,
            jt: 0,
            jf: 1,
            k: syscall as u32,
        });
        filter.push(stmt(BPF_RET_K, RET_USER_NOTIF));
    }
    filter.push(stmt(BPF_RET_K, RET_ALLOW));

    let prog = libc::sock_fprog {
        len: filter.len() as u16,
        filter: filter.as_ptr() as *mut _,
    };
    let r = unsafe {
        libc::syscall(
            libc::SYS_seccomp,
            SECCOMP_SET_MODE_FILTER,
            SECCOMP_FILTER_FLAG_NEW_LISTENER,
            &prog as *const libc::sock_fprog,
        )
    };
    if r < 0 {
        return Err(std::io::Error::last_os_error()).context("seccomp(NEW_LISTENER)");
    }
    Ok(r as RawFd)
}
