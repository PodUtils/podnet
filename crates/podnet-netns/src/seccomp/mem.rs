//! The memory access of the seccomp backend: read and write the memory
//! of the task of a notification (`/proc/<pid>/mem`, else
//! `process_vm_writev`).

#[allow(unused_imports)]
use crate::prelude::*;

/// Read a pointer-sized word (a pointer or a `size_t`) of the child at
/// `offset` of `bytes`.
pub(crate) fn read_word(bytes: &[u8], offset: usize) -> u64 {
    const WORD: usize = std::mem::size_of::<usize>();
    let mut buf = [0u8; WORD];
    buf.copy_from_slice(&bytes[offset..offset + WORD]);
    usize::from_ne_bytes(buf) as u64
}

/// Read `len` bytes from the child at `addr`.
pub(crate) fn read_mem(mem: RawFd, addr: u64, len: usize) -> Result<Vec<u8>> {
    let mut buf = vec![0u8; len];
    let mut off = 0;
    while off < len {
        let n = unsafe {
            libc::pread(
                mem,
                buf[off..].as_mut_ptr() as *mut _,
                len - off,
                (addr as i64) + off as i64,
            )
        };
        if n <= 0 {
            bail!("pread(/proc/pid/mem): {}", std::io::Error::last_os_error());
        }
        off += n as usize;
    }
    Ok(buf)
}

/// Write `data` into the child at `addr`.
/// The pid of the notification of now: [`open_mem`] sets it, and
/// [`write_mem`] uses it for `process_vm_writev`. The supervisor handles one
/// notification at a time.
static MEM_PID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Write `data` at `addr` in the memory of the task of the notification:
/// through `mem` (`/proc/<pid>/mem` opened for writing), else with
/// `process_vm_writev` (issue 98: a host can refuse the write open of the
/// file and allow the system call).
pub(crate) fn write_mem(mem: RawFd, addr: u64, data: &[u8]) -> Result<()> {
    write_mem_to(
        mem,
        MEM_PID.load(std::sync::atomic::Ordering::Relaxed),
        addr,
        data,
    )
}

/// [`write_mem`] for the task `pid`: for a write from a task that does not
/// handle the notification of now (a blocking read).
pub(crate) fn write_mem_to(mem: RawFd, pid: u32, addr: u64, data: &[u8]) -> Result<()> {
    let mut off = 0;
    while off < data.len() {
        let n = if mem >= 0 {
            unsafe {
                libc::pwrite(
                    mem,
                    data[off..].as_ptr() as *const _,
                    data.len() - off,
                    (addr as i64) + off as i64,
                )
            }
        } else {
            -1
        };
        let n = if n > 0 {
            n
        } else {
            let local = libc::iovec {
                iov_base: data[off..].as_ptr() as *mut _,
                iov_len: data.len() - off,
            };
            let remote = libc::iovec {
                iov_base: (addr as usize + off) as *mut _,
                iov_len: data.len() - off,
            };
            // SAFETY: the local buffer is valid for iov_len bytes; the kernel
            // checks the remote range in the other process.
            unsafe { libc::process_vm_writev(pid as i32, &local, 1, &remote, 1, 0) }
        };
        if n <= 0 {
            bail!(
                "writing the memory of the program: {}",
                std::io::Error::last_os_error()
            );
        }
        off += n as usize;
    }
    Ok(())
}

/// Whether `process_vm_writev` to `pid` passes the permission check: a
/// write of one byte to address 1 (never mapped) gives EFAULT when the
/// check passes.
pub(crate) fn process_vm_writable(pid: u32) -> bool {
    let byte = [0u8; 1];
    let local = libc::iovec {
        iov_base: byte.as_ptr() as *mut _,
        iov_len: 1,
    };
    let remote = libc::iovec {
        iov_base: std::ptr::dangling_mut::<u8>() as *mut _,
        iov_len: 1,
    };
    // SAFETY: a one-byte local buffer; the remote address is never mapped.
    let n = unsafe { libc::process_vm_writev(pid as i32, &local, 1, &remote, 1, 0) };
    n >= 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EFAULT)
}

/// The test hook of the memory path (issue 98): `no-fd` skips the write open
/// of `/proc/<pid>/mem`; `none` also skips `process_vm_writev`.
fn mem_test_hook() -> &'static str {
    static HOOK: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    HOOK.get_or_init(|| std::env::var("PODNET_NETNS_TEST_MEM").unwrap_or_default())
}

/// Open `/proc/<pid>/mem`. Try read-write first, because some calls write a
/// result into the child. Return the descriptor (or -1) and whether it is
/// writable.
pub(crate) fn open_mem(pid: u32) -> (RawFd, bool) {
    MEM_PID.store(pid, std::sync::atomic::Ordering::Relaxed);
    let (fd, writable) = open_mem_of(pid);
    if !writable {
        warn_no_mem_write();
    }
    (fd, writable)
}

/// [`open_mem`] with no change of the pid of now and no warning.
pub(crate) fn open_mem_of(pid: u32) -> (RawFd, bool) {
    let Ok(path) = CString::new(format!("/proc/{pid}/mem")) else {
        return (-1, false);
    };
    let hook = mem_test_hook();
    if hook.is_empty() {
        let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDWR | libc::O_CLOEXEC) };
        if fd >= 0 {
            return (fd, true);
        }
    }
    let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
    // A read-only descriptor: writes go through process_vm_writev, when the
    // host allows it (issue 98).
    let writable = hook != "none" && process_vm_writable(pid);
    (fd, writable)
}

/// Once: the memory of the program cannot be written.
fn warn_no_mem_write() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        crate::warn(
            "warning: this host refuses writes to the memory of the program; \
             getsockname and getpeername fail with EOPNOTSUPP: QUIC clients (curl --http3) fail, and a program \
             that reads its own address can crash (for example a Go program); use --backend netns where it works",
        );
    });
}
