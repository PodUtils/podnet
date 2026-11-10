//! The `connect` call of the program under the seccomp backend.
//!
//! The dial of a TCP socket runs in its own task (T-067). The supervisor
//! does not answer the notification of the call: the kernel keeps the
//! caller blocked, and the supervisor serves the other calls of the
//! program. When the dial ends, the task sends a [`Dialed`] to the
//! notification loop. The loop then changes the entry of the socket and
//! answers each call that waits for the dial ([`finish`]).
//!
//! More than one call can wait for one dial: a second thread that connects
//! the same socket, or the same thread when the kernel starts its call
//! again after a signal. The kernel does the same for a blocking connect
//! on a socket in the state `SYN_SENT`. When no waiting call is pending at
//! the end of the dial (the caller exited, or a signal stopped the call),
//! the loop drops the connection. A later `connect` dials again.
//!
//! A plain upstream on a Unix socket goes to the program itself at the end
//! of the dial (T-065, [`crate::handoff`]). Another upstream gets a relay.
//!
//! T-223: a `connect` of a nonblocking socket gets `EINPROGRESS` at once,
//! as in the kernel, and the dial goes on. The socket of the program is one
//! end of an `AF_UNIX` socket pair: it is writable before the dial ends, so
//! `poll` does not wait. Thus a `getsockopt(SO_ERROR)` during the dial waits
//! for the end of the dial and gets its result. A second nonblocking
//! `connect` during the dial gets `EALREADY`. At the end of the dial, the
//! connection stays also when no call waits.

use crate::handoff::{dial_plain, Caller, Link};
use crate::prelude::*;
use crate::seccomp::{
    child_fd_flags, notif_continue, notif_id_valid, notif_respond, open_mem_of, parse_sockaddr,
    read_mem, spawn_relay, write_mem_to, Sock,
};

/// The end of the dial of one socket.
pub(crate) struct Dialed {
    key: Key,
    /// The first call: its descriptor number gives the ephemeral port.
    caller: Caller,
    peer: (IpAddr, u16),
    target: String,
    result: Result<Link>,
    /// The number of the flow in the record of `--flow-log` (T-157).
    flow: Option<u64>,
}

/// A `getsockopt(SO_ERROR)` that waits for a nonblocking dial (T-223).
pub(crate) struct ErrorCall {
    pub(crate) id: u64,
    pub(crate) pid: u32,
    pub(crate) fd: i32,
    /// The addresses of `optval` and `optlen` in the task, and the length
    /// that the task gave in `optlen`.
    pub(crate) optval: u64,
    pub(crate) optlen: u64,
    pub(crate) len: usize,
}

/// The calls that wait for the dial of one socket.
#[derive(Default)]
struct Waiting {
    connects: Vec<u64>,
    errors: Vec<ErrorCall>,
    /// The first `connect` was nonblocking: it got `EINPROGRESS`.
    nonblocking: bool,
}

/// The dials in progress.
pub(crate) struct Dials {
    /// For each socket with a dial in progress: the calls that wait for it.
    waiting: HashMap<Key, Waiting>,
    tx: tokio::sync::mpsc::UnboundedSender<Dialed>,
    rx: tokio::sync::mpsc::UnboundedReceiver<Dialed>,
}

impl Dials {
    pub(crate) fn new() -> Self {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        Dials {
            waiting: HashMap::new(),
            tx,
            rx,
        }
    }

    /// The next dial that ends.
    pub(crate) async fn next(&mut self) -> Option<Dialed> {
        self.rx.recv().await
    }

    /// True while a dial of the socket `key` is in progress.
    pub(crate) fn in_progress(&self, key: Key) -> bool {
        self.waiting.contains_key(&key)
    }

    /// True while a dial of the socket `key` that a nonblocking `connect`
    /// started is in progress.
    pub(crate) fn nonblocking(&self, key: Key) -> bool {
        self.waiting.get(&key).is_some_and(|w| w.nonblocking)
    }

    /// Add a `getsockopt(SO_ERROR)` to the calls that wait for the dial of
    /// `key`.
    pub(crate) fn wait_error(&mut self, key: Key, call: ErrorCall) {
        if let Some(waiting) = self.waiting.get_mut(&key) {
            waiting.errors.push(call);
        }
    }
}

/// True when the descriptor `fd` of the task `pid` has `O_NONBLOCK`.
fn is_nonblocking(pid: u32, fd: i32) -> bool {
    child_fd_flags(pid, fd).is_some_and(|f| f & libc::O_NONBLOCK != 0)
}

/// The settings of the pod that a dial uses.
pub(crate) struct Route<'a> {
    pub(crate) proxies: &'a [Proxy],
    pub(crate) rules: &'a [Rule],
    pub(crate) inbound_dir: &'a std::path::Path,
    pub(crate) verbose: bool,
}

/// The `connect` call. A datagram socket gets its answer now. A stream
/// socket gets its answer when its dial ends.
#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_connect(
    lfd: RawFd,
    id: u64,
    a: [u64; 6],
    mem: RawFd,
    (map, key, pid): (&mut HashMap<Key, Sock>, Key, u32),
    dns: &mut FakeDns,
    dials: &mut Dials,
    route: &Route<'_>,
) -> Result<()> {
    let fd = a[0] as i32;
    let is_datagram = matches!(map.get(&key), Some(Sock::Datagram(_)));
    match map.get(&key) {
        None => return notif_continue(lfd, id),
        Some(Sock::Connected(_)) => return notif_respond(lfd, id, -1, libc::EISCONN),
        Some(_) => {}
    }
    if let Some(waiting) = dials.waiting.get_mut(&key) {
        // A dial of this socket is in progress: a blocking call waits for
        // it, and a nonblocking call gets EALREADY, as in the kernel.
        if is_nonblocking(pid, fd) {
            return notif_respond(lfd, id, -1, libc::EALREADY);
        }
        waiting.connects.push(id);
        return Ok(());
    }
    let addr = read_mem(mem, a[1], (a[2] as usize).min(128))?;
    // T-146: `AF_UNSPEC` dissolves the association of a datagram socket, as
    // in the kernel.
    if is_datagram && addr.len() >= 2 && u16::from_ne_bytes([addr[0], addr[1]]) == 0 {
        if let Some(Sock::Datagram(udp)) = map.get(&key) {
            udp.inbox.set_dest(None);
        }
        return notif_respond(lfd, id, 0, 0);
    }
    let Some((_family, ip, port)) = parse_sockaddr(&addr) else {
        return notif_respond(lfd, id, -1, libc::EAFNOSUPPORT);
    };
    if is_datagram {
        if let Some(Sock::Datagram(udp)) = map.get(&key) {
            udp.inbox.set_dest(Some((ip, port)));
        }
        return notif_respond(lfd, id, 0, 0);
    }
    let target = match (dns.name_for_ip_synced(ip), ip) {
        (Some(name), _) => format!("{name}:{port}"),
        (None, IpAddr::V4(v4)) => format!("{v4}:{port}"),
        (None, IpAddr::V6(v6)) => format!("[{v6}]:{port}"),
    };
    // T-156: a rule that refuses the target answers at once.
    let picked = match ip.is_loopback() {
        true => select_chain(route.rules, route.proxies, &target),
        false => crate::learn::select(route.rules, route.proxies, &target, "tcp"),
    };
    let chain = picked.chain().map(<[Proxy]>::to_vec);
    let flow = match ip.is_loopback() {
        true => crate::flowlog::dial("tcp", &target, "loopback", Some(&[])),
        false => crate::flowlog::decided("tcp", &target, &picked),
    };
    if chain.is_none() && !ip.is_loopback() {
        crate::rules::note_refusal("connect", &target, picked.rule_name(), route.verbose);
        return notif_respond(lfd, id, -1, libc::ECONNREFUSED);
    }
    let nonblocking = is_nonblocking(pid, fd);
    let waiting = Waiting {
        connects: if nonblocking { Vec::new() } else { vec![id] },
        errors: Vec::new(),
        nonblocking,
    };
    dials.waiting.insert(key, waiting);
    let tx = dials.tx.clone();
    let mut done = Dialed {
        key,
        caller: Caller { id, pid, fd },
        peer: (ip, port),
        target: target.clone(),
        result: Err(anyhow::anyhow!("no dial")),
        flow,
    };
    if ip.is_loopback() {
        // Loopback does not go to the proxy (issue 80): a listener of the
        // program itself (its inbound socket), else TCP on this host.
        if route.verbose {
            eprintln!(
                "{}: connect {target} direct (loopback)",
                crate::cli::program_name()
            );
        }
        let inbound_dir = route.inbound_dir.to_path_buf();
        tokio::spawn(async move {
            let dial = connect_loopback(&inbound_dir, ip, port);
            done.result = within_timeout("loopback connect", dial)
                .await
                .map(Link::Relay);
            let _ = tx.send(done);
        });
    } else {
        let chain = chain.unwrap_or_default();
        if route.verbose {
            eprintln!(
                "{}: connect {target} via {}",
                crate::cli::program_name(),
                chain[0].endpoint()
            );
        }
        tokio::spawn(async move {
            // With --flow-log, the pod relays each flow, and counts its
            // bytes (T-157): no hand-off.
            let plain = crate::handoff::plain_hop(&chain).filter(|_| !crate::flowlog::on());
            done.result = match plain {
                Some(hop) => within_timeout("upstream connect", dial_plain(hop, &target)).await,
                None => {
                    let dial = connect_upstream_async(&chain, &target);
                    within_timeout("upstream connect", dial)
                        .await
                        .map(Link::Relay)
                }
            };
            let _ = tx.send(done);
        });
    }
    match nonblocking {
        true => notif_respond(lfd, id, -1, libc::EINPROGRESS),
        false => Ok(()),
    }
}

/// The end of a dial: change the entry of the socket and answer each call
/// that waits for the dial and is still pending.
pub(crate) fn finish(
    lfd: RawFd,
    done: Dialed,
    map: &mut crate::fdtable::Table,
    dials: &mut Dials,
    verbose: bool,
) {
    let Dialed {
        key,
        caller,
        peer,
        target,
        result,
        flow,
    } = done;
    let mut waiting = dials.waiting.remove(&key).unwrap_or_default();
    waiting.connects.retain(|&id| notif_id_valid(lfd, id));
    waiting.errors.retain(|call| notif_id_valid(lfd, call.id));
    if !waiting.nonblocking && waiting.connects.is_empty() {
        // No call waits for the blocking dial: drop the connection.
        if verbose {
            eprintln!(
                "{}: connect {target}: the call left; drop the dial",
                crate::cli::program_name()
            );
        }
        return;
    }
    // Answer each waiting call. A `connect` takes the error, else the first
    // `SO_ERROR` takes it, as in the kernel. True when a call took it.
    let answer = |errno: i32| {
        for &id in &waiting.connects {
            let _ = notif_respond(lfd, id, if errno == 0 { 0 } else { -1 }, errno);
        }
        let mut pending = if waiting.connects.is_empty() {
            errno
        } else {
            0
        };
        for call in &waiting.errors {
            answer_error(lfd, call, std::mem::take(&mut pending));
        }
        !waiting.connects.is_empty() || !waiting.errors.is_empty()
    };
    // The one call that can take the upstream socket (T-065): the blocking
    // `connect`, or the one `SO_ERROR` of a nonblocking dial.
    let (taker, ids) = match (&waiting.connects[..], &waiting.errors[..]) {
        ([], [call]) => {
            let who = Caller {
                id: call.id,
                pid: call.pid,
                fd: call.fd,
            };
            (who, vec![call.id])
        }
        _ => (caller, waiting.connects.clone()),
    };
    match result {
        Ok(link) => match map.remove(&key) {
            Some(Sock::Stream {
                mut pair,
                bound,
                options,
                ..
            }) => {
                // T-063: the entry stays, for the options and the addresses.
                let entry = crate::sockopt::Connected::new(caller.fd, peer, bound, options);
                let handed = match &link {
                    Link::Plain(up) => {
                        let ends = (up, &pair);
                        crate::handoff::hand_off(lfd, taker, &ids, ends, (map, key))
                    }
                    Link::Relay(_) => None,
                };
                if let Some(new_key) = handed {
                    // T-065: the program holds the upstream; no relay.
                    if verbose {
                        let what = "the program holds the upstream socket (no relay)";
                        eprintln!("{}: connect {target}: {what}", crate::cli::program_name());
                    }
                    map.insert(new_key, Sock::Connected(entry));
                    answer(0);
                    return;
                }
                let Ok(mut upstream) = link.into_relay() else {
                    answer(libc::ECONNABORTED);
                    return;
                };
                map.insert(key, Sock::Connected(entry));
                let target = target.clone();
                spawn_relay(async move {
                    crate::flowlog::relay(&mut pair, &mut upstream, flow, &target).await;
                });
                answer(0);
            }
            other => {
                // The program closed the socket during the dial.
                if let Some(sock) = other {
                    map.insert(key, sock);
                }
                answer(libc::ECONNABORTED);
            }
        },
        Err(e) => {
            if verbose {
                eprintln!(
                    "{}: upstream for {target} failed: {e:#}",
                    crate::cli::program_name()
                );
            }
            // Keep the error for SO_ERROR when no call took it (issue 302,
            // T-223): the kernel clears it when `connect` returns it.
            if !answer(libc::ECONNREFUSED) {
                if let Some(Sock::Stream { error, .. }) = map.get_mut(&key) {
                    *error = libc::ECONNREFUSED;
                }
            }
        }
    }
}

/// Answer a `getsockopt(SO_ERROR)` that waited for a dial: the option is
/// `errno`.
fn answer_error(lfd: RawFd, call: &ErrorCall, errno: i32) {
    let (mem, _) = open_mem_of(call.pid);
    let value = errno.to_ne_bytes();
    let out = call.len.min(value.len());
    // The task is the same after the open of its memory.
    let written = notif_id_valid(lfd, call.id)
        && write_mem_to(mem, call.pid, call.optval, &value[..out]).is_ok()
        && write_mem_to(mem, call.pid, call.optlen, &(out as u32).to_ne_bytes()).is_ok();
    if mem >= 0 {
        // SAFETY: `mem` is a descriptor that this function opened.
        unsafe { libc::close(mem) };
    }
    let _ = match written {
        true => notif_respond(lfd, call.id, 0, 0),
        false => notif_respond(lfd, call.id, -1, libc::EFAULT),
    };
}

/// A loopback connect of the program: its own inbound listener on `port`
/// when there is one, else TCP to `ip:port` on this host.
async fn connect_loopback(
    inbound_dir: &std::path::Path,
    ip: IpAddr,
    port: u16,
) -> Result<crate::upstream::Upstream> {
    let own = inbound_dir.join(format!("{port}.sock"));
    if let Ok(stream) = tokio::net::UnixStream::connect(&own).await {
        return Ok(Box::new(stream));
    }
    let stream =
        crate::upstream::connect_direct(&std::net::SocketAddr::new(ip, port).to_string()).await?;
    Ok(Box::new(stream))
}
