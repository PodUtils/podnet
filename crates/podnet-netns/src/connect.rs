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
//!
//! The state of the dials (the calls that wait, and their answers at the
//! end of a dial) is `pod_hold::connect` of PodUtils/podcrates. This module
//! keeps the dial, the rules, the entry of the socket, the hand-off and the
//! relay.

use crate::handoff::{dial_plain, Caller, Link};
use crate::prelude::*;
use crate::seccomp::{
    child_fd_flags, mem_access, notif_continue, notif_id_valid, notif_respond, parse_sockaddr,
    read_mem, spawn_relay, Sock,
};
use pod_hold::connect::{write_error_to_task, Joined, Started};

pub(crate) use pod_hold::connect::ErrorCall;

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

/// The dials in progress: their state, and the channel of their ends.
pub(crate) struct Dials {
    state: pod_hold::connect::Dials<Key>,
    tx: tokio::sync::mpsc::UnboundedSender<Dialed>,
    rx: tokio::sync::mpsc::UnboundedReceiver<Dialed>,
}

impl Dials {
    pub(crate) fn new() -> Self {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        Dials {
            state: pod_hold::connect::Dials::new(),
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
        self.state.in_progress(key)
    }

    /// True while a dial of the socket `key` that a nonblocking `connect`
    /// started is in progress.
    pub(crate) fn nonblocking(&self, key: Key) -> bool {
        self.state.nonblocking(key)
    }

    /// Add a `getsockopt(SO_ERROR)` to the calls that wait for the dial of
    /// `key`.
    pub(crate) fn wait_error(&mut self, key: Key, call: ErrorCall) {
        self.state.wait_error(key, call);
    }
}

/// True when the descriptor `fd` of the task `pid` has `O_NONBLOCK`.
fn is_nonblocking(pid: u32, fd: i32) -> bool {
    child_fd_flags(pid, fd).is_some_and(|f| f & libc::O_NONBLOCK != 0)
}

/// The settings of the pod that a dial uses.
/// `--loopback-rules` (T-231): the rules decide a loopback connect too. A
/// loopback target that a rule (or `--default-action reject`) refuses is
/// refused; an allowed one goes direct, as before.
static LOOPBACK_RULES: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Set `--loopback-rules` for the process.
pub(crate) fn set_loopback_rules(on: bool) {
    LOOPBACK_RULES.store(on, std::sync::atomic::Ordering::Relaxed);
}

fn loopback_rules() -> bool {
    LOOPBACK_RULES.load(std::sync::atomic::Ordering::Relaxed)
}

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
    if dials.in_progress(key) {
        // A dial of this socket is in progress: a blocking call waits for
        // it, and a nonblocking call gets EALREADY, as in the kernel.
        return match dials.state.join(key, id, is_nonblocking(pid, fd)) {
            Some(Joined::Already) => notif_respond(lfd, id, -1, libc::EALREADY),
            _ => Ok(()),
        };
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
    // T-231: with --loopback-rules, the rules decide a loopback target too.
    // T-255: an IPv4-mapped address (::ffff:127.0.0.1, from a dual-stack
    // socket) is loopback as its IPv4 address is.
    let loopback = is_loopback_target(ip);
    let ruled = !loopback || loopback_rules();
    let picked = match ruled {
        false => select_chain(route.rules, route.proxies, &target),
        true => crate::learn::select(route.rules, route.proxies, &target, "tcp"),
    };
    let chain = picked.chain().map(<[Proxy]>::to_vec);
    let flow = match (loopback, chain.is_some() || !ruled) {
        (true, true) => crate::flowlog::dial("tcp", &target, "loopback", Some(&[])),
        _ => crate::flowlog::decided("tcp", &target, &picked),
    };
    if chain.is_none() && ruled {
        crate::rules::note_refusal("connect", &target, picked.rule_name(), route.verbose);
        return notif_respond(lfd, id, -1, libc::ECONNREFUSED);
    }
    let started = dials.state.start(key, id, is_nonblocking(pid, fd));
    let tx = dials.tx.clone();
    let mut done = Dialed {
        key,
        caller: Caller { id, pid, fd },
        peer: (ip, port),
        target: target.clone(),
        result: Err(anyhow::anyhow!("no dial")),
        flow,
    };
    if let Some(ip) = loopback_dial(ip) {
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
    match started {
        Started::InProgress => notif_respond(lfd, id, -1, libc::EINPROGRESS),
        Started::Waits => Ok(()),
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
    let Some(waiting) = dials.state.finish(key, |id| notif_id_valid(lfd, id)) else {
        // No call waits for the blocking dial: drop the connection.
        if verbose {
            eprintln!(
                "{}: connect {target}: the call left; drop the dial",
                crate::cli::program_name()
            );
        }
        return;
    };
    // Answer each waiting call. A `connect` takes the error, else the first
    // `SO_ERROR` takes it, as in the kernel. True when a call took it.
    let answer = |errno: i32| {
        let write = |call: &ErrorCall, value| write_error_to_task(lfd, mem_access(), call, value);
        waiting.answer(lfd, errno, write)
    };
    // The one call that can take the upstream socket (T-065): the blocking
    // `connect`, or the one `SO_ERROR` of a nonblocking dial.
    let (taker, ids) = waiting.taker(caller);
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

/// The address of a direct loopback dial for a connect to `ip`, else
/// `None`: the IPv4 form of an IPv4-mapped address (T-317), as the rules
/// take it (T-255).
pub(crate) fn loopback_dial(ip: IpAddr) -> Option<IpAddr> {
    is_loopback_target(ip).then(|| ip.to_canonical())
}

/// True when a connect to `ip` is a connect to this host: `127.0.0.0/8`,
/// `::1`, or an IPv4-mapped form of a loopback address (`::ffff:127.0.0.1`).
pub(crate) fn is_loopback_target(ip: IpAddr) -> bool {
    ip.to_canonical().is_loopback()
}

#[cfg(test)]
mod loopback_tests {
    use super::{is_loopback_target, loopback_dial};

    /// T-255 (PodUtils/podrunner#64): a dual-stack socket gives the
    /// IPv4-mapped form of 127.0.0.1; it is loopback.
    #[test]
    fn a_mapped_loopback_address_is_loopback() {
        for (ip, want) in [
            ("127.0.0.1", true),
            ("127.1.2.3", true),
            ("::1", true),
            ("::ffff:127.0.0.1", true),
            ("::ffff:127.9.9.9", true),
            ("::ffff:10.0.0.1", false),
            ("10.0.0.1", false),
            ("2001:db8::1", false),
        ] {
            assert_eq!(is_loopback_target(ip.parse().unwrap()), want, "{ip}");
        }
    }

    /// T-317 (PodUtils/podrunner#69): the dial of a mapped loopback address
    /// goes direct to its IPv4 form, not to the proxy chain.
    #[test]
    fn a_mapped_loopback_address_is_dialed_direct() {
        let v4 = |s: &str| Some(s.parse().unwrap());
        assert_eq!(
            loopback_dial("::ffff:127.0.0.1".parse().unwrap()),
            v4("127.0.0.1")
        );
        assert_eq!(loopback_dial("127.0.0.1".parse().unwrap()), v4("127.0.0.1"));
        assert_eq!(loopback_dial("::1".parse().unwrap()), v4("::1"));
        assert_eq!(loopback_dial("::ffff:10.0.0.1".parse().unwrap()), None);
    }
}
