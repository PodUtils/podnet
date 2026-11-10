#[allow(unused_imports)]
use crate::prelude::*;

mod backend;
mod fdpass;
mod packet;
mod tun;

pub(crate) use backend::*;
pub(crate) use fdpass::*;
pub(crate) use packet::*;
pub(crate) use tun::*;

/// A std `Ipv4Addr` as smoltcp's wire type.
pub(crate) fn smol_v4(addr: Ipv4Addr) -> Ipv4Address {
    // smoltcp 0.12: the wire type is `core::net::Ipv4Addr`.
    addr
}

// ── tunables ────────────────────────────────────────────────────────────────

/// Address on the TUN inside the namespace (the guest side of the /31).
pub(crate) const TUN_ADDR: Ipv4Addr = Ipv4Addr::new(10, 66, 255, 255);
/// The gateway the stack impersonates, and the DNS server handed to the guest.
pub(crate) const TUN_GW: Ipv4Addr = Ipv4Addr::new(10, 66, 255, 254);
pub(crate) const TUN_PREFIX: u8 = 31;
pub(crate) const TUN_MTU: usize = 1500;
pub(crate) const TCP_BUF: usize = 64 * 1024;
pub(crate) const DNS_PORT: u16 = 53;
/// The longest wait of the netns stack loop between two passes.
pub(crate) const POLL_INTERVAL: Duration = Duration::from_millis(5);
/// RFC 2544 benchmarking range, used by proxychains and friends for fake IPs.
pub(crate) const FAKE_IP_BASE: u32 = 0xC612_0000; // 198.18.0.0
pub(crate) const FAKE_IP_COUNT: u32 = 1 << 17; // 198.18.0.0/15
/// The first 96 bits of a fake IPv6 address: fd63:6672:7300::/96 (a ULA).
pub(crate) const FAKE_IP6_PREFIX: [u8; 16] = [
    0xfd, 0x63, 0x66, 0x72, 0x73, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

// ── errors ──────────────────────────────────────────────────────────────────

/// Exit code for "this host cannot run any backend", distinct from a failure
/// of the program itself.
pub(crate) const EXIT_NO_BACKEND: i32 = 3;
// ── namespace + TUN setup (child side) ──────────────────────────────────────

pub(crate) fn write_proc(path: &str, data: &str) -> Result<()> {
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .with_context(|| format!("open {path}"))?;
    f.write_all(data.as_bytes())
        .with_context(|| format!("write {path}"))
}

pub(crate) fn write_id_maps(pid: u32) -> Result<()> {
    let uid = unsafe { libc::getuid() };
    let gid = unsafe { libc::getgid() };
    write_proc(&format!("/proc/{pid}/setgroups"), "deny")?;
    write_proc(&format!("/proc/{pid}/uid_map"), &format!("{uid} {uid} 1\n"))?;
    write_proc(&format!("/proc/{pid}/gid_map"), &format!("{gid} {gid} 1\n"))?;
    Ok(())
}

// ── process orchestration ───────────────────────────────────────────────────

pub(crate) fn run(args: Args) -> Result<i32> {
    let verbose = args.verbose;
    crate::outlive::split(verbose, |reporter| run_router(args, reporter))
}

/// The report of the reaper of the namespace: the `waitpid` status of the
/// first program (4 bytes) and 1 when other processes of the pod live.
const REAPER_REPORT: usize = 5;

/// Run the router of the pod until no process lives in the namespace, or,
/// with no front ([`crate::outlive::Reporter::none`]), until the first
/// program exits.
fn run_router(args: Args, reporter: crate::outlive::Reporter) -> Result<i32> {
    let (parent, child) = socketpair()?;
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        bail!("fork: {}", std::io::Error::last_os_error());
    }
    if pid == 0 {
        // Child: new namespace, TUN, then the reaper and the program.
        unsafe { libc::close(parent) };
        reporter.close_in_child();
        let result = child_setup(child, &args);
        // child_setup only returns on error; it execs on success.
        if let Err(e) = result {
            eprintln!("{}: {e:#}", program_name());
            unsafe { libc::_exit(EXIT_NO_BACKEND) };
        }
        unsafe { libc::_exit(0) };
    }
    unsafe { libc::close(child) };

    // Parent: the child reports whether it made a user namespace; if so, write
    // its id maps before it continues, then take the TUN fd.
    let mut kind = [0u8; 1];
    if read_exact_fd(parent, &mut kind)? == 0 {
        bail!("child died before reporting its namespace");
    }
    if kind[0] == b'u' {
        write_id_maps(pid as u32).context("write uid/gid maps")?;
        write_all_fd(parent, b"a")?;
    }
    let tun_fd = recv_fd(parent).context("receive TUN fd")?;
    // The reaper writes its report on this socket later; read it with no wait.
    unsafe { libc::fcntl(parent, libc::F_SETFL, libc::O_NONBLOCK) };

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let proxies = args.proxies.clone();
    let rules = args.rules.clone();
    let maps = args.maps.clone();
    let verbose = args.verbose;
    let result = runtime.block_on(async move {
        let mut backend = Backend::new(tun_fd, proxies, rules, maps, verbose)?;
        let fd = backend.device.fd;
        let async_fd = crate::readable_copy(fd)?;
        let mut report = Vec::with_capacity(REAPER_REPORT);
        let mut first: Option<i32> = None;
        loop {
            backend.device.read_ready();
            backend.tick().await?;
            // Wake on TUN input or a short timer, whichever comes first. The
            // readiness must be cleared after the TUN is drained: if it stays
            // set, this loop never parks, and the I/O driver never marks an
            // upstream socket readable.
            tokio::select! {
                guard = async_fd.readable() => {
                    if let Ok(mut guard) = guard {
                        backend.device.read_ready();
                        guard.clear_ready();
                    }
                }
                _ = tokio::time::sleep(POLL_INTERVAL) => {}
            }
            if first.is_none() && read_report(parent, &mut report) {
                let status = i32::from_le_bytes([report[0], report[1], report[2], report[3]]);
                let code = crate::outlive::exit_code(status);
                if report[4] == 0 {
                    // No other process lives in the namespace.
                    return Ok::<i32, anyhow::Error>(code);
                }
                if !reporter.outlives() {
                    // No front: the pod ends with its first program.
                    unsafe { libc::kill(pid, libc::SIGKILL) };
                    return Ok(code);
                }
                if verbose {
                    eprintln!(
                        "{}: the program exited {code}; the pod stays while its other processes live",
                        program_name()
                    );
                }
                reporter.report(code, verbose);
                first = Some(code);
            }
            let mut status = 0;
            let r = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
            if r == pid {
                return Ok(first.unwrap_or_else(|| crate::outlive::exit_code(status)));
            }
        }
    });
    unsafe { libc::close(parent) };
    // Reap if the loop exited without seeing it.
    let mut status = 0;
    unsafe { libc::waitpid(pid, &mut status, 0) };
    result
}

/// Read the report of the reaper into `report`, with no wait. True when it
/// is complete.
fn read_report(fd: RawFd, report: &mut Vec<u8>) -> bool {
    let mut buf = [0u8; REAPER_REPORT];
    let want = REAPER_REPORT - report.len();
    let n = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut _, want) };
    if n > 0 {
        report.extend_from_slice(&buf[..n as usize]);
    }
    report.len() == REAPER_REPORT
}

/// The first process of the namespace: start the program, reap each orphan
/// of the pod, and report the status of the program on `sock`. Exit when
/// no child is left. A daemon that the program starts keeps the namespace
/// and its router (T-108).
fn reaper(sock: RawFd, program: libc::pid_t) -> ! {
    // Ctrl-C goes to the program; the reaper stays for the other processes.
    // The reaper lets go of the standard streams of the caller, so a pipe of
    // the caller closes when the program and its children close it.
    unsafe {
        libc::signal(libc::SIGINT, libc::SIG_IGN);
        libc::signal(libc::SIGQUIT, libc::SIG_IGN);
        let null = libc::open(c"/dev/null".as_ptr(), libc::O_RDWR);
        if null >= 0 {
            for fd in 0..3 {
                libc::dup2(null, fd);
            }
            libc::close(null);
        }
    }
    let mut first = 0;
    loop {
        let mut status = 0;
        let r = unsafe { libc::waitpid(-1, &mut status, 0) };
        if r < 0 {
            if std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            break;
        }
        if r == program {
            first = crate::outlive::exit_code(status);
            let mut report = status.to_le_bytes().to_vec();
            report.push(u8::from(crate::outlive::has_children()));
            let _ = write_all_fd(sock, &report);
        }
    }
    unsafe { libc::_exit(first) }
}

pub(crate) fn child_setup(sock: RawFd, args: &Args) -> Result<()> {
    // A network namespace alone needs CAP_SYS_ADMIN; the user-namespace pair
    // works unprivileged and the parent writes the maps.
    let userns = if unsafe { libc::unshare(libc::CLONE_NEWNET) } == 0 {
        false
    } else {
        let e = std::io::Error::last_os_error();
        if unsafe { libc::unshare(libc::CLONE_NEWUSER | libc::CLONE_NEWNET) } != 0 {
            bail!(
                "unshare(CLONE_NEWNET) failed ({e}) and unshare(CLONE_NEWUSER|CLONE_NEWNET) failed ({})",
                std::io::Error::last_os_error()
            );
        }
        true
    };
    // Tell the parent, and wait for the id maps before using the namespace.
    write_all_fd(sock, if userns { b"u" } else { b"n" })?;
    if userns {
        let mut ack = [0u8; 1];
        if read_exact_fd(sock, &mut ack)? == 0 {
            bail!("parent died before writing the id maps");
        }
    }
    bringup_loopback()?;
    let tun = create_tun("eth0")?;
    send_fd(sock, tun.as_raw_fd())?;
    std::mem::forget(tun); // the parent owns its own handle; keep ours open too
                           // The resolver of the host (often 127.0.0.53) is on the loopback of the
                           // host, which this namespace does not have: give the program a
                           // resolv.conf with the gateway of the TUN, where the pod answers DNS
                           // (issue 61). A failure leaves the host file, with a warning.
    if let Err(e) = private_resolv_conf() {
        crate::warn(&format!(
            "warning: the program keeps the resolver of the host ({e:#})"
        ));
    }

    let program = &args.program;
    let path = CString::new(program[0].as_bytes())?;
    let mut argv: Vec<CString> = Vec::with_capacity(program.len() + 1);
    for a in program {
        argv.push(CString::new(a.as_bytes())?);
    }
    let ptrs: Vec<*const libc::c_char> = argv
        .iter()
        .map(|c| c.as_ptr())
        .chain(std::iter::once(std::ptr::null()))
        .collect();
    let env = crate::child_env(args.keep_proxy_env);
    let envp: Vec<*const libc::c_char> = env
        .iter()
        .map(|c| c.as_ptr())
        .chain(std::iter::once(std::ptr::null()))
        .collect();
    crate::outlive::become_subreaper()?;
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        bail!("fork: {}", std::io::Error::last_os_error());
    }
    if pid > 0 {
        reaper(sock, pid);
    }
    unsafe {
        libc::close(sock);
        libc::execvpe(path.as_ptr(), ptrs.as_ptr(), envp.as_ptr());
    }
    bail!(
        "execvpe {}: {}",
        program[0],
        std::io::Error::last_os_error()
    )
}

/// In a new mount namespace, put a resolv.conf with the TUN gateway over
/// /etc/resolv.conf (a bind mount of a file that is then unlinked).
fn private_resolv_conf() -> Result<()> {
    if unsafe { libc::unshare(libc::CLONE_NEWNS) } != 0 {
        bail!("unshare(CLONE_NEWNS): {}", std::io::Error::last_os_error());
    }
    let root = CString::new("/")?;
    // Make the mounts private, so the bind mount does not reach the host.
    if unsafe {
        libc::mount(
            std::ptr::null(),
            root.as_ptr(),
            std::ptr::null(),
            libc::MS_REC | libc::MS_PRIVATE,
            std::ptr::null(),
        )
    } != 0
    {
        bail!(
            "mount --make-rprivate /: {}",
            std::io::Error::last_os_error()
        );
    }
    let file =
        std::env::temp_dir().join(format!("podnet-netns-resolv-{}.conf", std::process::id()));
    std::fs::write(&file, format!("nameserver {TUN_GW}\noptions ndots:0\n"))
        .context("writing the resolv.conf of the pod")?;
    let source = CString::new(file.as_os_str().as_encoded_bytes())?;
    let target = CString::new("/etc/resolv.conf")?;
    let rc = unsafe {
        libc::mount(
            source.as_ptr(),
            target.as_ptr(),
            std::ptr::null(),
            libc::MS_BIND,
            std::ptr::null(),
        )
    };
    let err = std::io::Error::last_os_error();
    let _ = std::fs::remove_file(&file);
    if rc != 0 {
        bail!("bind mount over /etc/resolv.conf: {err}");
    }
    Ok(())
}

pub(crate) fn write_all_fd(fd: RawFd, mut buf: &[u8]) -> Result<()> {
    while !buf.is_empty() {
        let n = unsafe { libc::write(fd, buf.as_ptr() as *const _, buf.len()) };
        if n <= 0 {
            bail!("write: {}", std::io::Error::last_os_error());
        }
        buf = &buf[n as usize..];
    }
    Ok(())
}

pub(crate) fn read_exact_fd(fd: RawFd, buf: &mut [u8]) -> Result<usize> {
    let mut off = 0;
    while off < buf.len() {
        let n = unsafe { libc::read(fd, buf[off..].as_mut_ptr() as *mut _, buf.len() - off) };
        if n < 0 {
            bail!("read: {}", std::io::Error::last_os_error());
        }
        if n == 0 {
            return Ok(off);
        }
        off += n as usize;
    }
    Ok(off)
}

pub(crate) fn socketpair() -> Result<(RawFd, RawFd)> {
    let mut fds = [0 as RawFd; 2];
    if unsafe {
        libc::socketpair(
            libc::AF_UNIX,
            libc::SOCK_STREAM | libc::SOCK_CLOEXEC,
            0,
            fds.as_mut_ptr(),
        )
    } != 0
    {
        bail!("socketpair: {}", std::io::Error::last_os_error());
    }
    Ok((fds[0], fds[1]))
}
