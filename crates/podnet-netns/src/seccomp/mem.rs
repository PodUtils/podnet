//! The memory access of the seccomp backend. pod-hold (PodUtils/podcrates)
//! reads and writes the memory of the task of a notification
//! (`/proc/<pid>/mem`, else `process_vm_writev`); this module keeps the
//! pid of the notification of now and the test hook of the pod.

#[allow(unused_imports)]
use crate::prelude::*;

pub(crate) use pod_hold::mem::process_vm_writable;

/// Read a pointer-sized word (a pointer or a `size_t`) of the child at
/// `offset` of `bytes`. The caller checks the length of `bytes` first.
pub(crate) fn read_word(bytes: &[u8], offset: usize) -> u64 {
    pod_hold::mem::read_word(bytes, offset).expect("a word inside the bytes")
}

/// Read `len` bytes from the child at `addr`.
pub(crate) fn read_mem(mem: RawFd, addr: u64, len: usize) -> Result<Vec<u8>> {
    Ok(pod_hold::mem::read(mem, addr, len)?)
}

/// The pid of the notification of now: [`note_task`] sets it, and
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
    Ok(pod_hold::mem::write_to(mem, pid, addr, data)?)
}

/// The access of the pod to the memory of the program. The test hook of
/// the memory path (issue 98): `no-fd` skips the write open of
/// `/proc/<pid>/mem`; `none` also skips `process_vm_writev`.
pub(crate) fn mem_access() -> pod_hold::mem::Access {
    static HOOK: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let hook = HOOK.get_or_init(|| std::env::var("PODNET_NETNS_TEST_MEM").unwrap_or_default());
    match hook.as_str() {
        "" => pod_hold::mem::Access::default(),
        "none" => pod_hold::mem::Access::READ_ONLY,
        _ => pod_hold::mem::Access {
            write_open: false,
            vm_write: true,
        },
    }
}

/// The task of the notification of now, and whether its memory can be
/// written: the pid for [`write_mem`], and the warning once.
pub(crate) fn note_task(pid: u32, writable: bool) {
    MEM_PID.store(pid, std::sync::atomic::Ordering::Relaxed);
    if !writable {
        warn_no_mem_write();
    }
}

/// Open the memory of the task `pid`. Return the descriptor (or -1) and
/// whether it is writable. The caller closes the descriptor.
pub(crate) fn open_mem_of(pid: u32) -> (RawFd, bool) {
    let memory = pod_hold::mem::Memory::open(pid, mem_access());
    let writable = memory.writable();
    (memory.into_raw_fd(), writable)
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
