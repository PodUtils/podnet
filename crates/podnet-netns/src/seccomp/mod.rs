#[allow(unused_imports)]
use crate::prelude::*;

mod filter;

pub(crate) use filter::*;

// ── seccomp user-notification backend ───────────────────────────────────────
//
// Where a network namespace is denied, a seccomp filter that returns
// `SECCOMP_RET_USER_NOTIF` still lets a supervisor mediate syscalls without any
// `LD_PRELOAD`. This backend intercepts `socket(2)` and hands the child a
// socketpair end with `SECCOMP_IOCTL_NOTIF_ADDFD`; the child's own `connect(2)`
// is then answered by the supervisor, which dials the proxy and splices. Only
// `socket` and `connect` need emulating for TCP, because the socketpair carries
// the bytes natively. UDP port 53 is answered from the fake-IP pool.

/// Read a pointer-sized word (a pointer or a `size_t`) of the child at
/// `offset` of `bytes`.
pub(crate) fn read_word(bytes: &[u8], offset: usize) -> u64 {
    const WORD: usize = std::mem::size_of::<usize>();
    let mut buf = [0u8; WORD];
    buf.copy_from_slice(&bytes[offset..offset + WORD]);
    usize::from_ne_bytes(buf) as u64
}

/// Find `name` as `execvp` does: a name with a `/` is used as it is; another
/// name is looked up in the directories of `PATH`.
pub(crate) fn resolve_program(name: &str) -> Result<std::path::PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    if name.contains('/') {
        return Ok(std::path::PathBuf::from(name));
    }
    let path = std::env::var_os("PATH").unwrap_or_else(|| "/usr/local/bin:/usr/bin:/bin".into());
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if let Ok(meta) = std::fs::metadata(&candidate) {
            if meta.is_file() && meta.permissions().mode() & 0o111 != 0 {
                return Ok(candidate);
            }
        }
    }
    bail!("the program {name:?} is not in PATH")
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

/// A listening socket inside the sandbox. Connections arrive on a unix
/// listener (the only bind a sealed host permits) and are handed to the child's
/// `accept` as injected socketpair ends.
/// The queue of a listener of the child. The child's listener fd is one end
/// of a socketpair: for each queued connection the reader task writes one
/// byte to the other end, so poll, select and epoll on the listener wake.
/// Each accept takes one byte back through `drain` (a copy of the child's
/// end), so the listener is readable while a connection waits, and only
/// then.
pub(crate) struct Inbound {
    pub(crate) rx:
        Arc<tokio::sync::Mutex<tokio::sync::mpsc::UnboundedReceiver<tokio::net::UnixStream>>>,
    pub(crate) drain: Arc<std::os::unix::net::UnixStream>,
}

pub(crate) enum Sock {
    Stream {
        pair: tokio::net::UnixStream,
        bound: Option<(IpAddr, u16)>,
        /// The family is `AF_INET6` (T-223: the address of `listen` with no `bind`).
        v6: bool,
        inbound: Option<Inbound>,
        /// The pending error of the socket (`SO_ERROR`): the errno of the
        /// last failed connect, 0 when none. A read of `SO_ERROR` clears it.
        error: i32,
        options: crate::sockopt::Options,
    },
    /// A connected TCP socket (T-063): a relay, or the program (T-065), has it.
    Connected(crate::sockopt::Connected),
    /// A UDP socket (`src/udp.rs`).
    Datagram(crate::udp::Udp),
    /// An ICMP datagram socket (T-066): the pod answers an echo to its gateway.
    Icmp(crate::icmp::Icmp),
}

pub(crate) fn run_seccomp(args: Args) -> Result<i32> {
    if args.proxies.is_empty() {
        bail!("the seccomp backend needs -x (there is no direct mode)");
    }
    // Issue 306: the inbound directory is private before the program runs.
    crate::listen::private_dir(&args.inbound_dir)?;
    // A seccomp filter is per-thread unless TSYNC is used. Install it on a
    // helper thread and fork there: the child inherits the filter, and the
    // listener fd stays in this process's fd table, so no SCM_RIGHTS handshake
    // is needed (the child's own sendmsg is itself intercepted).
    let program = args.program.clone();
    // execv does not search PATH, and the child must not allocate after the
    // fork: find the program here.
    let resolved = resolve_program(&program[0])?;
    let mut env = crate::child_env(args.keep_proxy_env);
    crate::with_single_request(&mut env);
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("podnet-netns-filter".into())
        .spawn(move || {
            // Built before fork so the child allocates nothing.
            let path = match CString::new(resolved.into_os_string().into_encoded_bytes()) {
                Ok(p) => p,
                Err(e) => {
                    let _ = tx.send(Err(anyhow::anyhow!("program path: {e}")));
                    return;
                }
            };
            let argv: Vec<CString> = match program
                .iter()
                .map(|a| CString::new(a.as_bytes()))
                .collect::<Result<Vec<_>, _>>()
            {
                Ok(v) => v,
                Err(e) => {
                    let _ = tx.send(Err(anyhow::anyhow!("argv: {e}")));
                    return;
                }
            };
            let ptrs: Vec<*const libc::c_char> = argv
                .iter()
                .map(|c| c.as_ptr())
                .chain(std::iter::once(std::ptr::null()))
                .collect();
            let envp: Vec<*const libc::c_char> = env
                .iter()
                .map(|c| c.as_ptr())
                .chain(std::iter::once(std::ptr::null()))
                .collect();

            let lfd = match install_notif_filter() {
                Ok(fd) => fd,
                Err(e) => {
                    let _ = tx.send(Err(e));
                    return;
                }
            };
            let pid = unsafe { libc::fork() };
            if pid < 0 {
                let _ = tx.send(Err(anyhow::anyhow!(
                    "fork: {}",
                    std::io::Error::last_os_error()
                )));
                return;
            }
            if pid == 0 {
                // Child: the filter is inherited. execv with no allocation.
                unsafe { libc::execve(path.as_ptr(), ptrs.as_ptr(), envp.as_ptr()) };
                unsafe { libc::_exit(127) };
            }
            let _ = tx.send(Ok((lfd, pid)));
        })?;
    let (lfd, pid) = rx.recv().context("filter thread died")??;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(supervise_seccomp(
        lfd,
        pid,
        args.proxies,
        args.rules,
        args.maps,
        args.verbose,
        args.inbound_dir,
    ))
}

pub(crate) async fn supervise_seccomp(
    lfd: RawFd,
    pid: i32,
    proxies: Vec<Proxy>,
    rules: Vec<Rule>,
    maps: Vec<(String, IpAddr)>,
    verbose: bool,
    inbound_dir: std::path::PathBuf,
) -> Result<i32> {
    // NOTIF_RECV is non-blocking so the shutdown flag is observed; the fd is
    // also pollable, so the loop sleeps on readiness rather than spinning.
    unsafe { libc::fcntl(lfd, libc::F_SETFL, libc::O_NONBLOCK) };
    let async_fd = crate::readable_copy(lfd)?;

    let shutdown = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    {
        let stop = shutdown.clone();
        tokio::spawn(async move {
            loop {
                let mut status = 0;
                let r = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
                if r == pid {
                    stop.store(true, std::sync::atomic::Ordering::Relaxed);
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        });
    }

    let mut map = crate::fdtable::Table::new(pid);
    let mut dns = FakeDns::new();
    let mut dials = crate::connect::Dials::new();
    let (writes_tx, mut writes) = tokio::sync::mpsc::channel(crate::udp::WRITES);
    let route = crate::connect::Route {
        proxies: &proxies,
        rules: &rules,
        inbound_dir: &inbound_dir,
        verbose,
    };
    dns.v6 = true;
    for (name, ip) in &maps {
        dns.pin(name, *ip);
    }

    while !shutdown.load(std::sync::atomic::Ordering::Relaxed) {
        // NOTIF_RECV blocks even on an O_NONBLOCK descriptor, so call it only
        // when poll(2) reports a pending notification.
        // T-067: a dial that ended gets its answer here.
        let guard = tokio::select! {
            guard = async_fd.readable() => guard,
            Some(done) = dials.next() => {
                crate::connect::finish(lfd, done, &mut map, &mut dials, verbose);
                continue;
            }
            // T-143: a datagram that the program wrote with `write(2)`.
            Some(written) = writes.recv() => {
                crate::udp::relay_written(&mut map, written, &mut dns, &proxies, verbose).await;
                continue;
            }
            _ = tokio::time::sleep(Duration::from_millis(50)) => continue,
        };
        let mut guard = guard?;
        let mut pfd = libc::pollfd {
            fd: lfd,
            events: libc::POLLIN,
            revents: 0,
        };
        unsafe { libc::poll(&mut pfd, 1, 0) };
        if pfd.revents & libc::POLLIN == 0 {
            guard.clear_ready();
            if pfd.revents & (libc::POLLHUP | libc::POLLERR) != 0 {
                // No task uses the filter any more.
                break;
            }
            continue;
        }
        let mut req: SeccompNotif = unsafe { std::mem::zeroed() };
        let r = unsafe { libc::ioctl(lfd, SECCOMP_IOCTL_NOTIF_RECV as _, &mut req) };
        if r < 0 {
            match std::io::Error::last_os_error().raw_os_error() {
                // ENOENT: the child left the system call before it was read.
                Some(libc::EAGAIN) | Some(libc::EINTR) | Some(libc::ENOENT) => continue,
                _ => break,
            }
        }
        let id = req.id;
        let nr = req.data.nr;
        let a = req.data.args;
        // T-064: the entry of a socket follows the socket, not the number.
        let key = match nr == libc::SYS_socket as i32 {
            true => 0,
            false => map.key_of(req.pid, a[0] as i32),
        };

        // Open the memory of the task that made this call, for each call. A
        // descriptor opened before the child's execve keeps the old address
        // space and reads 0 bytes. A forked grandchild has a different pid.
        let (mem, mem_writable) = open_mem(req.pid);
        // The pid can be reused after the task exits. NOTIF_ID_VALID makes
        // sure the task that made this call is still alive after the open.
        if mem >= 0 && !notif_id_valid(lfd, id) {
            unsafe { libc::close(mem) };
            continue;
        }

        let call = (nr, a, req.pid, (mem, mem_writable));
        let tcp = crate::tcp::handle(lfd, id, call, (&mut map, key), &mut dials);
        let icmp = crate::icmp::handle(lfd, id, nr, a, (mem, mem_writable), (&mut map, key));
        let outcome = if let Some(outcome) = tcp {
            outcome
        } else if let Some(outcome) = icmp.await {
            outcome
        } else if nr == libc::SYS_socket as i32 {
            crate::socket::handle_socket(lfd, id, a, &mut map, &writes_tx, verbose)
        } else if nr == libc::SYS_connect as i32 {
            let entry = (&mut *map, key, req.pid);
            crate::connect::handle_connect(lfd, id, a, mem, entry, &mut dns, &mut dials, &route)
        } else if nr == libc::SYS_sendto as i32 {
            handle_sendto(lfd, id, a, mem, (&mut map, key), &mut dns, &proxies).await
        } else if nr == libc::SYS_sendmsg as i32 {
            handle_sendmsg(lfd, id, a, mem, (&mut map, key), &mut dns, &proxies).await
        } else if nr == libc::SYS_sendmmsg as i32 {
            handle_sendmmsg(
                lfd,
                id,
                a,
                (mem, mem_writable),
                (&mut map, key),
                &mut dns,
                &proxies,
            )
            .await
        } else if nr == libc::SYS_setsockopt as i32 {
            crate::sockopt::handle_setsockopt(lfd, id, a, mem, (&mut map, key), verbose)
        } else if nr == libc::SYS_getsockopt as i32 {
            crate::sockopt::handle_getsockopt(lfd, id, a, mem, mem_writable, (&mut map, key))
        } else if nr == libc::SYS_close as i32 {
            // Drop the entry when the last copy of the socket closes.
            crate::fdtable::handle_close(lfd, id, a[0] as i32, key, req.pid, &mut map)
        } else if nr == libc::SYS_io_uring_setup as i32 {
            // io_uring submits network operations without the intercepted
            // syscalls, which would bypass this filter entirely. Refuse it.
            notif_respond(lfd, id, -1, libc::EPERM)
        } else if nr == libc::SYS_bind as i32 {
            crate::sockopt::handle_bind(lfd, id, a, mem, (&mut map, key))
        } else if nr == libc::SYS_listen as i32 {
            handle_listen(lfd, id, a, (&mut map, key), &inbound_dir, req.pid).await
        } else if nr == SYS_ACCEPT as i32 || nr == libc::SYS_accept4 as i32 {
            handle_accept(
                lfd,
                id,
                a,
                (&mut map, key),
                nr == libc::SYS_accept4 as i32,
                req.pid,
            )
            .await
        } else if crate::inbox::is_recv(nr) {
            crate::inbox::handle_recv(lfd, id, a, (mem, mem_writable), &map, key, nr, req.pid)
        } else if nr == libc::SYS_getsockname as i32 || nr == libc::SYS_getpeername as i32 {
            let peer = nr == libc::SYS_getpeername as i32;
            crate::sockopt::handle_sockname(lfd, id, a, mem, mem_writable, (&mut map, key), peer)
        } else {
            notif_continue(lfd, id)
        };
        if let Err(e) = outcome {
            if verbose {
                eprintln!("podnet-netns: notif {nr}: {e:#}");
            }
            // A handler that fails before it answers must not leave the child
            // blocked. If the handler answered already, the kernel refuses
            // this second answer with ENOENT, which is correct.
            let errno = if nr == libc::SYS_connect as i32 {
                libc::ECONNREFUSED
            } else {
                libc::EIO
            };
            let _ = notif_respond(lfd, id, -1, errno);
        }
        if mem >= 0 {
            unsafe { libc::close(mem) };
        }
    }

    // The child can exit while a relay still carries its last bytes (a server
    // that writes a reply and exits). Let the relays finish, for a short time.
    // The UDP relays of the program stop now (T-142).
    map.retain(|_, sock| !matches!(sock, Sock::Datagram(_)));
    drain_relays(RELAY_DRAIN).await;

    let mut status = 0;
    unsafe { libc::waitpid(pid, &mut status, 0) };
    Ok(if libc::WIFEXITED(status) {
        libc::WEXITSTATUS(status)
    } else {
        128 + libc::WTERMSIG(status)
    })
}

/// `SECCOMP_IOCTL_NOTIF_ID_VALID`: `_IOW('!', 2, __u64)`.
pub(crate) const SECCOMP_IOCTL_NOTIF_ID_VALID: libc::c_ulong = 0x4008_2102;

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

/// Return true while the notification `id` is still pending.
pub(crate) fn notif_id_valid(lfd: RawFd, id: u64) -> bool {
    let mut id = id;
    unsafe { libc::ioctl(lfd, SECCOMP_IOCTL_NOTIF_ID_VALID as _, &mut id) == 0 }
}

/// The number of byte relays that are in progress.
static ACTIVE_RELAYS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// The longest time to wait for relays after the child exits.
pub(crate) const RELAY_DRAIN: Duration = Duration::from_secs(3);

/// Spawn a relay task and count it until it ends.
pub(crate) fn spawn_relay<F>(relay: F)
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    use std::sync::atomic::Ordering;
    ACTIVE_RELAYS.fetch_add(1, Ordering::SeqCst);
    tokio::spawn(async move {
        relay.await;
        ACTIVE_RELAYS.fetch_sub(1, Ordering::SeqCst);
    });
}

/// Wait until no relay is in progress, or until `limit` elapses.
pub(crate) async fn drain_relays(limit: Duration) {
    let deadline = tokio::time::Instant::now() + limit;
    while ACTIVE_RELAYS.load(std::sync::atomic::Ordering::SeqCst) > 0
        && tokio::time::Instant::now() < deadline
    {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Above this many tracked descriptors the child is refused more, so a program
/// that leaks sockets cannot grow the supervisor without bound.
pub(crate) const MAX_TRACKED_FDS: usize = 4096;

pub(crate) async fn handle_sendto(
    lfd: RawFd,
    id: u64,
    a: [u64; 6],
    mem: RawFd,
    (map, fd): (&mut HashMap<Key, Sock>, Key),
    dns: &mut FakeDns,
    proxies: &[Proxy],
) -> Result<()> {
    let buf = a[1];
    let len = a[2] as usize;
    let name = a[4];
    let namelen = a[5] as usize;
    let recorded = match map.get(&fd) {
        Some(Sock::Datagram(udp)) => udp.inbox.dest(),
        _ => return notif_continue(lfd, id),
    };
    let target = if name != 0 && namelen >= 4 {
        parse_sockaddr(&read_mem(mem, name, namelen.min(128))?)
            .map(|(_, ip, port)| (ip, port))
            .or(recorded)
    } else {
        recorded
    };
    let Some((ip, port)) = target else {
        return notif_respond(lfd, id, -1, libc::EDESTADDRREQ);
    };
    // The length comes from the program: check it before the read.
    if len > MAX_DATAGRAM {
        return notif_respond(lfd, id, -1, libc::EMSGSIZE);
    }
    let Ok(payload) = read_mem(mem, buf, len) else {
        return notif_respond(lfd, id, -1, libc::EFAULT);
    };
    match crate::udp::send_payload(map, fd, dns, proxies, &payload, (ip, port)).await {
        Ok(sent) => notif_respond(lfd, id, sent as i64, 0),
        Err(errno) => notif_respond(lfd, id, -1, errno),
    }
}

/// `sendmmsg` is what the glibc resolver uses: a vector of `mmsghdr`, for
/// example the A and the AAAA query in one call.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_sendmmsg(
    lfd: RawFd,
    id: u64,
    a: [u64; 6],
    (mem, mem_writable): (RawFd, bool),
    (map, fd): (&mut HashMap<Key, Sock>, Key),
    dns: &mut FakeDns,
    proxies: &[Proxy],
) -> Result<()> {
    // struct mmsghdr is a struct msghdr (56 bytes), a u32 msg_len and padding.
    // The layouts of the child are those of this target: same ABI.
    const MMSGHDR_SIZE: u64 = std::mem::size_of::<libc::mmsghdr>() as u64;
    const MSG_LEN_OFFSET: u64 = std::mem::offset_of!(libc::mmsghdr, msg_len) as u64;
    let msgvec = a[1];
    let vlen = (a[2] as usize).min(64);
    if !map.contains_key(&fd) {
        return notif_continue(lfd, id);
    }
    if vlen == 0 {
        return notif_respond(lfd, id, 0, 0);
    }
    // Without write access to the child, msg_len cannot be set. Send one
    // message and report 1. glibc's resolver does not send the others: it
    // reads the msg_len of 0 as a failure. The pod therefore sets
    // RES_OPTIONS=single-request (issue 113), and glibc sends one query in
    // each call.
    let count = if mem_writable { vlen } else { 1 };
    let mut sent = 0usize;
    for index in 0..count {
        let entry = msgvec + index as u64 * MMSGHDR_SIZE;
        match send_msghdr(fd, entry, mem, map, dns, proxies).await? {
            None => return notif_continue(lfd, id),
            Some(Ok(bytes)) => {
                if mem_writable {
                    write_mem(mem, entry + MSG_LEN_OFFSET, &(bytes as u32).to_ne_bytes())?;
                }
                sent += 1;
            }
            Some(Err(errno)) => {
                if sent == 0 {
                    return notif_respond(lfd, id, -1, errno);
                }
                break;
            }
        }
    }
    notif_respond(lfd, id, sent as i64, 0)
}

pub(crate) async fn handle_sendmsg(
    lfd: RawFd,
    id: u64,
    a: [u64; 6],
    mem: RawFd,
    (map, fd): (&mut HashMap<Key, Sock>, Key),
    dns: &mut FakeDns,
    proxies: &[Proxy],
) -> Result<()> {
    match send_msghdr(fd, a[1], mem, map, dns, proxies).await? {
        None => notif_continue(lfd, id),
        Some(Ok(bytes)) => notif_respond(lfd, id, bytes as i64, 0),
        Some(Err(errno)) => notif_respond(lfd, id, -1, errno),
    }
}

/// Send one `struct msghdr` for the child. Return the byte count, or the
/// errno to give the child. `Ok(None)` means: not a socket of ours.
pub(crate) async fn send_msghdr(
    fd: Key,
    msg_ptr: u64,
    mem: RawFd,
    map: &mut HashMap<Key, Sock>,
    dns: &mut FakeDns,
    proxies: &[Proxy],
) -> Result<Option<std::result::Result<usize, i32>>> {
    let recorded = match map.get(&fd) {
        Some(Sock::Datagram(udp)) => udp.inbox.dest(),
        _ => return Ok(None),
    };
    // struct msghdr on x86_64: name, namelen, iov, iovlen, control, controllen, flags.
    // struct msghdr of this target (the child has the same ABI).
    let hdr = read_mem(mem, msg_ptr, std::mem::size_of::<libc::msghdr>())?;
    let name_ptr = read_word(&hdr, std::mem::offset_of!(libc::msghdr, msg_name));
    let name_len = u32::from_ne_bytes(
        hdr[std::mem::offset_of!(libc::msghdr, msg_namelen)..][..4]
            .try_into()
            .unwrap(),
    ) as usize;
    let iov_ptr = read_word(&hdr, std::mem::offset_of!(libc::msghdr, msg_iov));
    let iov_len = read_word(&hdr, std::mem::offset_of!(libc::msghdr, msg_iovlen)) as usize;
    let target = if name_ptr != 0 && name_len >= 4 {
        parse_sockaddr(&read_mem(mem, name_ptr, name_len.min(128))?)
            .map(|(_, ip, port)| (ip, port))
            .or(recorded)
    } else {
        recorded
    };
    let Some((ip, port)) = target else {
        return Ok(Some(Err(libc::EDESTADDRREQ)));
    };
    // The lengths come from the program: check the count of the iovecs and
    // the sum of their lengths before a read (issue 303). Before, only the
    // first 8 iovecs were read, each up to 64 KiB.
    if iov_len > crate::inbox::UIO_MAXIOV {
        return Ok(Some(Err(libc::EMSGSIZE)));
    }
    let size = std::mem::size_of::<libc::iovec>();
    let Ok(iovs) = read_mem(mem, iov_ptr, iov_len * size) else {
        return Ok(Some(Err(libc::EFAULT)));
    };
    let parts: Vec<(u64, usize)> = iovs
        .chunks_exact(size)
        .map(|iov| {
            (
                read_word(iov, std::mem::offset_of!(libc::iovec, iov_base)),
                read_word(iov, std::mem::offset_of!(libc::iovec, iov_len)) as usize,
            )
        })
        .collect();
    let total = parts
        .iter()
        .fold(0usize, |sum, (_, len)| sum.saturating_add(*len));
    if total > MAX_DATAGRAM {
        return Ok(Some(Err(libc::EMSGSIZE)));
    }
    let mut payload = Vec::with_capacity(total);
    for (base, len) in parts {
        if len == 0 {
            continue;
        }
        let Ok(part) = read_mem(mem, base, len) else {
            return Ok(Some(Err(libc::EFAULT)));
        };
        payload.extend_from_slice(&part);
    }
    let sent = crate::udp::send_payload(map, fd, dns, proxies, &payload, (ip, port)).await;
    Ok(Some(sent))
}

/// `listen` creates the real listener the sandbox can have: a unix socket named
/// after the port. A reader task turns accepted connections into a queue that
/// `accept` draws from, and makes the child's listener fd readable while a
/// connection waits (see [`Inbound`]).
pub(crate) async fn handle_listen(
    lfd: RawFd,
    id: u64,
    a: [u64; 6],
    (map, key): (&mut HashMap<Key, Sock>, Key),
    inbound_dir: &std::path::Path,
    pid: u32,
) -> Result<()> {
    let fd = a[0] as i32;
    let Some(Sock::Stream {
        bound: Some((_, port)),
        ..
    }) = map.get(&key)
    else {
        return notif_continue(lfd, id);
    };
    let port = *port;
    crate::listen::private_dir(inbound_dir)?;
    let path = inbound_dir.join(format!("{port}.sock"));
    let _ = std::fs::remove_file(&path);
    let listener = crate::listen::bind_private(&path)
        .with_context(|| format!("bind inbound listener {}", path.display()))?;

    // The new listener fd of the child: one end of a socketpair, with the
    // O_NONBLOCK and close-on-exec of the old fd.
    let flags = child_fd_flags(pid, fd).unwrap_or(0);
    let (signal, child_end) = std::os::unix::net::UnixStream::pair()?;
    if flags & libc::O_NONBLOCK != 0 {
        set_nonblocking(child_end.as_raw_fd())?;
    }
    let drain = child_end.try_clone()?;
    let newfd_flags = if flags & libc::O_CLOEXEC != 0 {
        libc::O_CLOEXEC as u32
    } else {
        0
    };
    // The new end is a new socket: the entry moves to its key.
    let new_key = crate::fdtable::inode_of(child_end.as_raw_fd())?;
    notif_addfd_setfd(lfd, id, child_end.as_raw_fd(), fd, newfd_flags)?;
    drop(child_end);
    if let Some(entry) = map.remove(&key) {
        map.insert(new_key, entry);
    }
    set_nonblocking(signal.as_raw_fd())?;
    let mut signal = tokio::net::UnixStream::from_std(signal)?;

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Ok((conn, _)) = listener.accept().await {
            // The byte first: when accept finds the connection, its byte is
            // there to take.
            if tokio::io::AsyncWriteExt::write_all(&mut signal, &[1])
                .await
                .is_err()
            {
                break;
            }
            if tx.send(conn).is_err() {
                break;
            }
        }
    });
    if let Some(Sock::Stream { inbound, .. }) = map.get_mut(&new_key) {
        *inbound = Some(Inbound {
            rx: Arc::new(tokio::sync::Mutex::new(rx)),
            drain: Arc::new(drain),
        });
    }
    notif_respond(lfd, id, 0, 0)?;
    // The listener is a real unix socket; say where it is.
    eprintln!(
        "podnet-netns: listening on unix:{} (port {port})",
        path.display()
    );
    Ok(())
}

/// Take the byte of one queued connection from the child's listener.
fn drain_one(drain: &std::os::unix::net::UnixStream) {
    let mut byte = [0u8; 1];
    unsafe {
        libc::recv(
            drain.as_raw_fd(),
            byte.as_mut_ptr() as *mut libc::c_void,
            1,
            libc::MSG_DONTWAIT,
        )
    };
}

/// Give the connection `conn` to the child as the answer of `accept`.
fn install_accepted(
    lfd: RawFd,
    id: u64,
    conn: tokio::net::UnixStream,
    nonblocking: bool,
    newfd_flags: u32,
) -> Result<()> {
    let (ours, theirs) = std::os::unix::net::UnixStream::pair()?;
    set_nonblocking(ours.as_raw_fd())?;
    if nonblocking {
        set_nonblocking(theirs.as_raw_fd())?;
    }
    // ADDFD with SEND already answers the syscall with the installed fd.
    notif_addfd_send(lfd, id, theirs.as_raw_fd(), newfd_flags)?;
    drop(theirs);
    let mut ours = tokio::net::UnixStream::from_std(ours)?;
    let mut conn = conn;
    spawn_relay(async move {
        let _ = tokio::io::copy_bidirectional(&mut ours, &mut conn).await;
    });
    Ok(())
}

/// `accept` takes a queued connection at once. With none queued, a
/// non-blocking listener gets EAGAIN, and a blocking accept waits in its own
/// task: the other system calls of the child go on (before, one accept that
/// waited stopped each other call of the child).
pub(crate) async fn handle_accept(
    lfd: RawFd,
    id: u64,
    a: [u64; 6],
    (map, key): (&mut HashMap<Key, Sock>, Key),
    accept4: bool,
    pid: u32,
) -> Result<()> {
    let fd = a[0] as i32;
    // The caller's flags are on accept4 only; ADDFD takes just O_CLOEXEC.
    let flags = if accept4 { a[3] as i32 } else { 0 };
    let mut newfd_flags = 0u32;
    if flags & libc::SOCK_CLOEXEC != 0 {
        newfd_flags |= libc::O_CLOEXEC as u32;
    }
    let nonblocking_conn = flags & libc::SOCK_NONBLOCK != 0;
    let (rx, drain) = {
        let Some(Sock::Stream {
            inbound: Some(inc), ..
        }) = map.get(&key)
        else {
            return notif_continue(lfd, id);
        };
        (inc.rx.clone(), inc.drain.clone())
    };
    let queued = rx.try_lock().ok().and_then(|mut rx| rx.try_recv().ok());
    if let Some(conn) = queued {
        drain_one(&drain);
        return install_accepted(lfd, id, conn, nonblocking_conn, newfd_flags);
    }
    let listener_nonblocking = child_fd_flags(pid, fd).is_some_and(|f| f & libc::O_NONBLOCK != 0);
    if listener_nonblocking {
        return notif_respond(lfd, id, -1, libc::EAGAIN);
    }
    tokio::spawn(async move {
        let conn = rx.lock().await.recv().await;
        match conn {
            Some(conn) => {
                drain_one(&drain);
                // ENOENT: the child left the accept (a signal); the
                // connection is closed.
                let _ = install_accepted(lfd, id, conn, nonblocking_conn, newfd_flags);
            }
            None => {
                let _ = notif_respond(lfd, id, -1, libc::EINVAL);
            }
        }
    });
    Ok(())
}
