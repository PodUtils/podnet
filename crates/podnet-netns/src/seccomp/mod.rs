#[allow(unused_imports)]
use crate::prelude::*;

mod accept;
mod filter;
mod mem;
mod notif;
mod send;

pub(crate) use accept::*;
pub(crate) use filter::*;
pub(crate) use mem::*;
pub(crate) use notif::*;
pub(crate) use send::*;

// ── seccomp user-notification backend ───────────────────────────────────────
//
// Where a network namespace is denied, a seccomp filter that returns
// `SECCOMP_RET_USER_NOTIF` still lets a supervisor mediate syscalls without any
// `LD_PRELOAD`. This backend intercepts `socket(2)` and hands the child a
// socketpair end with `SECCOMP_IOCTL_NOTIF_ADDFD`; the child's own `connect(2)`
// is then answered by the supervisor, which dials the proxy and splices. Only
// `socket` and `connect` need emulating for TCP, because the socketpair carries
// the bytes natively. UDP port 53 is answered from the fake-IP pool.

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
    let verbose = args.verbose;
    crate::outlive::split(verbose, |reporter| supervise_pod(args, reporter))
}

/// Start the program under the filter and supervise it. With a front
/// ([`crate::outlive::Reporter::outlives`]), the supervisor reaps each
/// orphan of the pod and stays until no task uses the filter (T-107).
fn supervise_pod(args: Args, reporter: crate::outlive::Reporter) -> Result<i32> {
    if reporter.outlives() {
        crate::outlive::become_subreaper()?;
    }
    // Issue 306: the inbound directory is private before the program runs.
    crate::listen::private_dir(&args.inbound_dir)?;
    // T-110: the directory of the session is private too.
    if let Some(dir) = args.dns_state.as_deref().and_then(std::path::Path::parent) {
        crate::listen::private_dir(dir)?;
    }
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
    // T-181: the program has its own limit now; the supervisor takes the
    // hard limit, and the budget of tracked sockets follows it.
    crate::fdtable::raise_nofile();
    let derived = crate::fdtable::derived_ceiling(crate::fdtable::nofile().0);
    let budget = args.flow_budget.unwrap_or(derived);
    if budget > derived {
        crate::warn(&format!(
            "warning: --flow-budget {budget} is above {derived}, the budget that RLIMIT_NOFILE permits; a socket pair can fail"
        ));
    }
    crate::fdtable::set_ceiling(budget);

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(supervise_seccomp(
        lfd,
        (pid, reporter),
        args.proxies,
        args.rules,
        args.maps,
        args.verbose,
        (args.inbound_dir, args.dns_state),
    ))
}

pub(crate) async fn supervise_seccomp(
    lfd: RawFd,
    (pid, reporter): (i32, crate::outlive::Reporter),
    proxies: Vec<Proxy>,
    rules: Vec<Rule>,
    maps: Vec<(String, IpAddr)>,
    verbose: bool,
    (inbound_dir, dns_state): (std::path::PathBuf, Option<std::path::PathBuf>),
) -> Result<i32> {
    // NOTIF_RECV is non-blocking so the shutdown flag is observed; the fd is
    // also pollable, so the loop sleeps on readiness rather than spinning.
    unsafe { libc::fcntl(lfd, libc::F_SETFL, libc::O_NONBLOCK) };
    let async_fd = crate::readable_copy(lfd)?;

    let shutdown = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let first = Arc::new(std::sync::Mutex::new(None));
    tokio::spawn(watch_children(
        (pid, lfd),
        reporter,
        shutdown.clone(),
        first.clone(),
        verbose,
    ));

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
    let egress = crate::udp::Egress {
        proxies: &proxies,
        rules: &rules,
        verbose,
    };
    dns.v6 = true;
    dns.shared = dns_state;
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
                crate::udp::relay_written(&mut map, written, &mut dns, &egress).await;
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
            handle_sendto(lfd, id, a, mem, (&mut map, key), &mut dns, &egress).await
        } else if nr == libc::SYS_sendmsg as i32 {
            handle_sendmsg(lfd, id, a, mem, (&mut map, key), &mut dns, &egress).await
        } else if nr == libc::SYS_sendmmsg as i32 {
            handle_sendmmsg(
                lfd,
                id,
                a,
                (mem, mem_writable),
                (&mut map, key),
                &mut dns,
                &egress,
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
                (req.pid, verbose),
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
                eprintln!("{}: notif {nr}: {e:#}", crate::cli::program_name());
            }
            crate::fdtable::refuse(crate::fdtable::Refusal::Other, "a call", verbose);
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

    // The watcher reaps the first program; wait for its status.
    loop {
        if let Some(status) = *first.lock().unwrap_or_else(|e| e.into_inner()) {
            return Ok(crate::outlive::exit_code(status));
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Reap the first program `pid`, and put its status in `first`. Set
/// `shutdown` when the pod ends: at the exit of the first program when no
/// other task uses the filter `lfd` (or with no front), else when no child
/// is left. With a front, reap each orphan of the pod, and give the code
/// of the first program to the front at once (T-107).
async fn watch_children(
    (pid, lfd): (i32, RawFd),
    reporter: crate::outlive::Reporter,
    shutdown: Arc<std::sync::atomic::AtomicBool>,
    first: Arc<std::sync::Mutex<Option<i32>>>,
    verbose: bool,
) {
    use std::sync::atomic::Ordering;
    let outlives = reporter.outlives();
    let target = if outlives { -1 } else { pid };
    let mut reaped = false;
    loop {
        let mut status = 0;
        let r = unsafe { libc::waitpid(target, &mut status, libc::WNOHANG) };
        if r == pid {
            reaped = true;
            *first.lock().unwrap_or_else(|e| e.into_inner()) = Some(status);
            if !outlives || filter_unused(lfd) {
                shutdown.store(true, Ordering::Relaxed);
                return;
            }
            let code = crate::outlive::exit_code(status);
            if verbose {
                eprintln!(
                    "{}: the program exited {code}; the pod stays while its other processes live",
                    program_name()
                );
            }
            reporter.report(code, verbose);
            continue;
        }
        if r > 0 {
            continue; // an orphan of the pod
        }
        if r < 0 && reaped {
            shutdown.store(true, Ordering::Relaxed);
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// True when no task uses the filter of the notification descriptor `lfd`
/// (POLLHUP).
fn filter_unused(lfd: RawFd) -> bool {
    let mut pfd = libc::pollfd {
        fd: lfd,
        events: libc::POLLIN,
        revents: 0,
    };
    unsafe { libc::poll(&mut pfd, 1, 0) };
    pfd.revents & libc::POLLHUP != 0
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
