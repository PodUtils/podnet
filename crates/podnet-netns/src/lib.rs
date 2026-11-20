//! podnet-netns: run a program with all its network traffic sent through a
//! SOCKS5 or HTTP proxy. No `LD_PRELOAD` is necessary.
//!
//! The kernel enforces the redirection, so static binaries, Go binaries and
//! programs that do not use libc `connect(3)` get the same treatment.
//!
//! Three backends are available:
//!
//! * `netns`: the child gets a new user and network namespace. A TUN device is
//!   its only interface (a veth pair when the namespace cannot open
//!   `/dev/net/tun`). A `smoltcp` stack on the host side sends each flow to
//!   the upstream proxy. This backend needs unprivileged user namespaces.
//! * `netns-kernel` (opt in; `auto` does not take it): the same namespace,
//!   with `nat OUTPUT REDIRECT` rules of the kernel as the data plane. The
//!   parent serves the listeners that the child made in the namespace. No
//!   TUN and no `smoltcp`. See `kernel`.
//! * `seccomp`: a `SECCOMP_RET_USER_NOTIF` filter stops the socket system
//!   calls of the child. The supervisor answers them and gives the child
//!   connected descriptors with `SECCOMP_IOCTL_NOTIF_ADDFD`. This backend needs
//!   only seccomp.
//!
//! ```text
//!   podnet pod [OPTIONS] -- PROGRAM
//!     |
//!     +- child:  namespace or seccomp filter, then exec PROGRAM
//!     |
//!     +- parent: fake-IP DNS, routing rules, SOCKS5/HTTP/tailnet upstream
//! ```
//!
//! The library also gives the parts that other programs use: the upstream
//! client ([`upstream`]), the proxy server codec ([`socks`]), the DNS codec
//! ([`dns`]), listen addresses ([`listen`]) and the tailnet dialer
//! ([`tailscale`]).
//!
//! Use [`main_with_args`] to run the full command line from another program.

pub mod dns;
pub mod door;
pub mod fuzzing;
pub mod join;
pub mod listen;
pub mod opening;
pub mod raw;
pub mod rules;
pub mod socks;
pub mod tailscale;
pub mod upstream;

mod cli;
mod connect;
mod dgram;
mod dnstcp;
mod doctor;
mod fakedns;
mod fdtable;
mod flowlog;
mod frontdoor;
mod handoff;
mod icmp;
mod inbox;
mod kernel;
mod kernel_serve;
mod learn;
mod netns;
mod nfnl;
mod nfprobe;
mod nfrules;
mod notice;
mod outlive;
mod recvmmsg;
mod report;
mod seccomp;
mod socket;
mod sockopt;
mod tcp;
mod udp;

pub use cli::{set_program_name, set_version_line, usage, usage_for};
pub use listen::ProxyListen;
pub use upstream::{Net, Proxy, ProxyKind, Rule, RuleAction, RuleMatch, Upstream};

/// The common imports of the internal modules.
#[allow(unused_imports)]
mod prelude {
    pub(crate) use std::collections::{HashMap, VecDeque};
    pub(crate) use std::ffi::CString;
    pub(crate) use std::io::Write;
    pub(crate) use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    pub(crate) use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
    pub(crate) use std::sync::Arc;
    pub(crate) use std::time::Duration;

    pub(crate) use anyhow::{bail, Context, Result};

    pub(crate) use crate::upstream::{
        connect_upstream, connect_upstream_async, dial_proxy, select_chain, socks5_udp_associate,
        within_timeout, Proxy, ProxyKind, Rule, RuleMatch, Upstream,
    };

    pub(crate) use crate::cli::*;
    pub(crate) use crate::dgram::*;
    pub(crate) use crate::doctor::*;
    pub(crate) use crate::fakedns::*;
    pub(crate) use crate::fdtable::Key;
    pub(crate) use crate::frontdoor::*;
    pub(crate) use crate::netns::*;
    pub(crate) use crate::seccomp::*;
}

#[allow(unused_imports)]
use prelude::*;

/// Register a copy (`dup`) of `fd` with the tokio reactor, for readable
/// events. The copy is closed when the `AsyncFd` is dropped; `fd` stays open.
pub(crate) fn readable_copy(fd: RawFd) -> std::io::Result<tokio::io::unix::AsyncFd<OwnedFd>> {
    let copy = unsafe { libc::dup(fd) };
    if copy < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: `copy` is a new open descriptor that nothing else owns.
    let owned = unsafe { OwnedFd::from_raw_fd(copy) };
    // SAFETY: the OwnedFd holds `copy` open and returns it from as_raw_fd
    // until the AsyncFd drops it.
    unsafe {
        tokio::io::unix::AsyncFd::register_with_interest(owned, tokio::io::Interest::READABLE)
    }
    .map_err(std::io::Error::from)
}

/// The exit code of a usage error (sysexits `EX_USAGE`).
pub const USAGE: i32 = 64;

/// Run the command line of `podnet pod` with `argv` (without the program name).
/// Return the exit status.
pub fn main_with_args(argv: &[String]) -> i32 {
    let args = match parse_args(argv) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{}: {e:#}", crate::cli::program_name());
            return USAGE;
        }
    };
    if args.doctor {
        return doctor(args.doctor_json);
    }
    let report = match report::Report::begin(args.report.clone()) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{}: {e:#}", cli::program_name());
            return USAGE;
        }
    };
    let code = run_args(args, &report);
    report.finish(code);
    code
}

/// The run of [`main_with_args`] after the parse: each failure goes into
/// `report`.
fn run_args(args: Args, report: &report::Report) -> i32 {
    if let Some(path) = &args.flow_log {
        if let Err(e) = flowlog::open(path) {
            return report.fail(USAGE, &format!("{e:#}"));
        }
    }
    let learned = match &args.learned {
        Some(Learned::Learn(path)) => learn::learn(path),
        Some(Learned::Enforce(path)) => learn::enforce(path),
        None => Ok(()),
    };
    if let Err(e) = learned {
        return report.fail(USAGE, &format!("{e:#}"));
    }
    if args.serve.is_some() {
        if args.proxies.is_empty() {
            return report.fail(USAGE, "--serve needs at least one -x upstream");
        }
        let listen = args.serve.clone().unwrap();
        let runtime = match tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
        {
            Ok(r) => r,
            Err(e) => return report.fail(EXIT_NO_BACKEND, &format!("{e:#}")),
        };
        let outcome = runtime.block_on(serve_front_door(
            listen,
            args.proxies.clone(),
            args.rules.clone(),
            args.verbose,
            args.early_reply,
        ));
        match outcome {
            Ok(()) => return 0,
            Err(e) => return report.fail(EXIT_NO_BACKEND, &format!("{e:#}")),
        }
    }
    // Issue 95: no -x, no proxy variable and no rule is a usage mistake
    // (exit 64), not a host that has no backend (exit 3).
    if args.proxies.is_empty() && args.rules.is_empty() {
        return report.fail(
            USAGE,
            "no proxy: give -x PROXY, or set ALL_PROXY, HTTPS_PROXY or HTTP_PROXY",
        );
    }
    warn_unreachable(&args.proxies, args.backend == BackendKind::NetnsKernel);
    // Inbound listeners exist on the seccomp backend only (issue 69).
    if args.inbound_dir_given
        && matches!(args.backend, BackendKind::Netns | BackendKind::NetnsKernel)
    {
        return report.fail(USAGE, "--inbound-dir works with --backend seccomp only");
    }
    let warn_inbound = |backend: &str| {
        if args.inbound_dir_given {
            warn(&format!(
                "warning: the {backend} backend has no inbound listeners; --inbound-dir is not used (give --backend seccomp)"
            ));
        }
    };
    let name_backend = args.name_backend;
    let netns = |args: Args, reason: &str| {
        name_the_backend(name_backend, BackendKind::Netns, reason);
        report.backend(BackendKind::Netns, reason);
        run(args)
    };
    // T-153: opt in only. `auto` never takes it: the flow table of the netns
    // backend stays the default.
    let netns_kernel = |args: Args, reason: &str| {
        name_the_backend(name_backend, BackendKind::NetnsKernel, reason);
        let probe = nfprobe::probe_netfilter();
        if !probe.ok {
            bail!(
                "the netns-kernel backend cannot run here ({}); use --backend netns",
                probe.detail
            );
        }
        report.backend(BackendKind::NetnsKernel, reason);
        kernel::run(args)
    };
    let seccomp = |args: Args, reason: &str| {
        name_the_backend(name_backend, BackendKind::Seccomp, reason);
        // T-231: with no read of the memory of the program, the pod sees no
        // address and refuses each connect: refuse the run at its start.
        seccomp_can_hold(&doctor::probe_mem_read())?;
        report.backend(BackendKind::Seccomp, reason);
        run_seccomp(args)
    };
    let result = match args.backend {
        // T-382: a refused TCP connect holds in each namespace (Landlock).
        BackendKind::Netns => match tcp_connect_refused() {
            Some(why) => Err(anyhow::anyhow!(
                "the netns backend cannot carry a TCP connect here ({why}; the refusal holds in each \
                 network namespace); use --backend seccomp"
            )),
            None => netns(args, "--backend netns"),
        },
        BackendKind::NetnsKernel => netns_kernel(args, "--backend netns-kernel"),
        BackendKind::Seccomp => seccomp(args, "--backend seccomp"),
        BackendKind::Auto if args.prefer_udp && doctor::probe_seccomp_notif().ok => {
            seccomp(args, "auto with --prefer-udp takes seccomp first")
        }
        // T-113: the netns backend needs unshare and a TUN; without them
        // the seccomp backend is the only thing that can work.
        BackendKind::Auto => match netns_probe() {
            Ok(()) => {
                warn_inbound("netns");
                netns(
                    args,
                    "auto takes netns first; the user and network namespace and the TUN (or a veth pair) work here",
                )
            }
            Err(why) => seccomp(
                args,
                &format!("auto takes netns first, but netns cannot run here ({why})"),
            ),
        },
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            report.no_backend();
            report.fail(EXIT_NO_BACKEND, &format!("{e:#}"))
        }
    }
}

/// Refuse the seccomp backend when the supervisor cannot read the memory of
/// a program (`probe`, `doctor::probe_mem_read`): no call can be held, and
/// the program would run with each connect refused (T-231).
pub(crate) fn seccomp_can_hold(probe: &doctor::Probe) -> Result<()> {
    if probe.ok {
        return Ok(());
    }
    bail!(
        "the seccomp backend cannot hold this program: the pod cannot read its memory ({}); \
         use --backend netns, or run the pod where /proc/<pid>/mem of a child can be opened",
        probe.detail
    )
}

/// The proxy variables that a program in the pod does not get: the pod is
/// its egress, and a program that used them would send to a host proxy that
/// the pod cannot reach (issue 61).
pub const PROXY_VARIABLES: &[&str] = &[
    "ALL_PROXY",
    "all_proxy",
    "HTTPS_PROXY",
    "https_proxy",
    "HTTP_PROXY",
    "http_proxy",
    "NO_PROXY",
    "no_proxy",
];

/// The environment of the program, as `KEY=VALUE` C strings for execve:
/// the environment of this process without [`PROXY_VARIABLES`] (unless
/// `keep_proxy`). Built before the fork; this process's own environment is
/// not changed (podnet warp run calls podnet-netns on a thread). Prints one line
/// that names the removed variables.
pub(crate) fn child_env(keep_proxy: bool) -> Vec<std::ffi::CString> {
    use std::os::unix::ffi::OsStrExt;
    let mut removed = Vec::new();
    let env: Vec<std::ffi::CString> = std::env::vars_os()
        .filter(|(key, _)| {
            let drop = !keep_proxy && key.to_str().is_some_and(|k| PROXY_VARIABLES.contains(&k));
            if drop {
                removed.push(key.to_string_lossy().into_owned());
            }
            !drop
        })
        .filter_map(|(key, value)| {
            let mut entry = key.as_bytes().to_vec();
            entry.push(b'=');
            entry.extend_from_slice(value.as_bytes());
            std::ffi::CString::new(entry).ok()
        })
        .collect();
    if !removed.is_empty() {
        warn(&format!(
            "the program does not get {} (the pod is its egress; --keep-proxy-env keeps them)",
            removed.join(", ")
        ));
    }
    env
}

/// The variable that silences the warnings of podnet-netns (issue 115): set
/// and not empty. `podnet -q` sets it. Errors are still printed.
pub const QUIET_VARIABLE: &str = "PODNET_NETNS_QUIET";

/// True when [`QUIET_VARIABLE`] is set.
pub fn quiet() -> bool {
    std::env::var_os(QUIET_VARIABLE).is_some_and(|v| !v.is_empty())
}

/// Print a warning on stderr, unless [`quiet`].
pub(crate) fn warn(text: &str) {
    if !quiet() {
        eprintln!("{}: {text}", crate::cli::program_name());
    }
}

/// The line that names the backend that runs and its limits (T-071).
fn backend_line(kind: BackendKind) -> &'static str {
    match kind {
        BackendKind::Seccomp => "seccomp backend: socket options and addresses are emulated; use --backend netns when a program fails",
        BackendKind::NetnsKernel => "netns-kernel backend: IPv4 TCP and UDP only; the program has the loopback and no other interface, a destination on 127.0.0.0/8 stays in the pod, and a ping of another address is answered by the pod; use --backend netns when a program needs more",
        _ => "netns backend: a loopback listener of the program is not seen from outside, and a UDP datagram larger than 1500 bytes does not go through; use --backend seccomp when a program needs them",
    }
}

/// Print [`backend_line`] and the reason of the choice (T-113) once on
/// stderr when `on`, unless [`quiet`].
fn name_the_backend(on: bool, kind: BackendKind, reason: &str) {
    if on {
        warn(&format!("{}. Choice: {reason}", backend_line(kind)));
    }
}

/// The test hook of the choice of `auto` (T-113): set and not empty, the
/// netns backend cannot run.
const TEST_NO_NETNS_VARIABLE: &str = "PODNET_NETNS_TEST_NO_NETNS";

/// Whether the netns backend can run here: `unshare` of a network namespace
/// (as root) or of a user and a network namespace, with `lo` and a TUN in
/// it. Gives the reason when it cannot.
fn netns_probe() -> std::result::Result<(), String> {
    if std::env::var_os(TEST_NO_NETNS_VARIABLE).is_some_and(|v| !v.is_empty()) {
        return Err(format!("{TEST_NO_NETNS_VARIABLE} is set"));
    }
    if let Some(why) = tcp_connect_refused() {
        return Err(why);
    }
    if probe_unshare(libc::CLONE_NEWNET).ok {
        return Ok(());
    }
    let probe = probe_unshare(libc::CLONE_NEWUSER | libc::CLONE_NEWNET);
    if probe.ok {
        return Ok(());
    }
    Err(format!("{}: {}", probe.name, probe.detail))
}

/// Whether a TCP connect of this process is refused with `EACCES` or
/// `EPERM` (T-382). A Landlock domain that handles `CONNECT_TCP` (the lab
/// `bind-denied`) refuses each TCP connect of the program in each network
/// namespace, so the program cannot reach the stack of the netns backend.
/// The probe connects to `127.0.0.1:1` with no wait: a refused port or a
/// listener both mean that the connect is allowed. Gives the reason.
fn tcp_connect_refused() -> Option<String> {
    let refused = |e: &std::io::Error| matches!(e.raw_os_error(), Some(libc::EACCES | libc::EPERM));
    let fd = unsafe {
        libc::socket(
            libc::AF_INET,
            libc::SOCK_STREAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
            0,
        )
    };
    if fd < 0 {
        let e = std::io::Error::last_os_error();
        return refused(&e).then(|| format!("a TCP socket is refused: {e}"));
    }
    let addr = libc::sockaddr_in {
        sin_family: libc::AF_INET as libc::sa_family_t,
        sin_port: 1u16.to_be(),
        sin_addr: libc::in_addr {
            s_addr: u32::from(std::net::Ipv4Addr::LOCALHOST).to_be(),
        },
        sin_zero: [0; 8],
    };
    let rc = unsafe {
        libc::connect(
            fd,
            (&addr as *const libc::sockaddr_in).cast(),
            std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
        )
    };
    let e = std::io::Error::last_os_error();
    unsafe { libc::close(fd) };
    (rc != 0 && refused(&e)).then(|| format!("a TCP connect to 127.0.0.1 is refused: {e}"))
}

/// Add `single-request` to `RES_OPTIONS` of a program environment (issue
/// 113). glibc then sends the A and the AAAA query one after the other.
/// Without it, glibc sends both with one `sendmmsg`; when the seccomp
/// supervisor cannot write `msg_len` into the program, glibc sees the
/// first message as not sent, and `getaddrinfo` with `AF_UNSPEC` fails.
/// A value that has `single-request` or `single-request-reopen` stays.
pub(crate) fn with_single_request(env: &mut Vec<std::ffi::CString>) {
    with_res_option(env, "single-request", &["single-request-reopen"]);
}

/// Add `use-vc` to `RES_OPTIONS` of a program environment when this
/// process cannot make a UDP socket (T-383). The program gets the same
/// filter, so its resolver cannot send a datagram; with `use-vc`, glibc
/// sends each query over TCP, and the pod answers it from its fake DNS
/// ([`dnstcp`]). Prints one line when it adds the option. musl does not
/// read `use-vc`: a musl program cannot resolve a name there.
pub(crate) fn with_dns_over_tcp(env: &mut Vec<std::ffi::CString>) {
    let Some(why) = dnstcp::udp_socket_refused() else {
        return;
    };
    if with_res_option(env, "use-vc", &[]) {
        warn(&format!(
            "the program gets RES_OPTIONS use-vc ({why}): glibc asks the fake DNS of the pod over TCP; a musl program cannot resolve a name"
        ));
    }
}

/// Add `option` to `RES_OPTIONS` of `env`. A value that has `option` or
/// one of `same` stays. True when the option was added.
fn with_res_option(env: &mut Vec<std::ffi::CString>, option: &str, same: &[&str]) -> bool {
    const KEY: &[u8] = b"RES_OPTIONS=";
    match env.iter_mut().find(|e| e.as_bytes().starts_with(KEY)) {
        Some(entry) => {
            let value = String::from_utf8_lossy(&entry.as_bytes()[KEY.len()..]).into_owned();
            if value
                .split_whitespace()
                .any(|o| o == option || same.contains(&o))
            {
                return false;
            }
            let value = format!("{} {option}", value.trim());
            match std::ffi::CString::new(format!("RES_OPTIONS={}", value.trim())) {
                Ok(new) => {
                    *entry = new;
                    true
                }
                Err(_) => false,
            }
        }
        None => match std::ffi::CString::new(format!("RES_OPTIONS={option}")) {
            Ok(new) => {
                env.push(new);
                true
            }
            Err(_) => false,
        },
    }
}

/// Warn when an upstream does not answer a connect in 2 s. The program
/// still runs (it may open no connection), but each connection that it
/// opens fails; before, a dead upstream gave no line at all.
fn warn_unreachable(proxies: &[upstream::Proxy], reset: bool) {
    for proxy in proxies {
        if let Some(line) = unreachable_line(proxy, reset) {
            warn(&line);
            notice::upstream_dead(proxy.connect_context());
        }
    }
}

/// The warning for an upstream that does not answer a connect in 2 s, else
/// `None`. Issue 621: `direct` is no upstream to dial, so it gets no line.
/// Issue 610: the line gives what the program sees: `ECONNREFUSED`, or a
/// reset on the netns-kernel backend (`reset`), which accepts the connect
/// first (T-452).
fn unreachable_line(proxy: &upstream::Proxy, reset: bool) -> Option<String> {
    use std::net::ToSocketAddrs;
    let limit = std::time::Duration::from_secs(2);
    if proxy.kind == upstream::ProxyKind::Direct {
        return None;
    }
    let (shown, result) = match &proxy.unix_path {
        Some(path) => (
            format!("unix:{path}"),
            std::os::unix::net::UnixStream::connect(path).map(drop),
        ),
        None => {
            let shown = format!("{}:{}", proxy.host, proxy.port);
            let result = (proxy.host.as_str(), proxy.port)
                .to_socket_addrs()
                .and_then(|mut addrs| {
                    addrs.next().ok_or_else(|| {
                        std::io::Error::new(std::io::ErrorKind::NotFound, "no address")
                    })
                })
                .and_then(|addr| std::net::TcpStream::connect_timeout(&addr, limit).map(drop));
            (shown, result)
        }
    };
    let err = result.err()?;
    let seen = match reset {
        true => "is reset",
        false => "fails with ECONNREFUSED",
    };
    Some(format!(
        "warning: the upstream {shown} does not answer ({err}); each connection of the program through it {seen}"
    ))
}

#[cfg(test)]
mod tests {
    use super::{unreachable_line, with_res_option, with_single_request};
    use crate::prelude::*;

    // Issue 621: -x direct dials no upstream, so it gets no warning. Issue
    // 610: a dead upstream gets a line that names what the program sees:
    // ECONNREFUSED, or a reset on the netns-kernel backend (T-452).
    #[test]
    fn only_a_dead_upstream_gets_the_warning() {
        let direct = crate::upstream::Proxy::parse("direct").unwrap();
        assert_eq!(unreachable_line(&direct, false), None);
        let dir = std::env::temp_dir().join(format!("podnet-netns-dead-{}", std::process::id()));
        let spec = format!("socks5h+unix:{}", dir.join("no-such.sock").display());
        let dead = crate::upstream::Proxy::parse(&spec).unwrap();
        for (reset, seen) in [(false, "fails with ECONNREFUSED"), (true, "is reset")] {
            let line = unreachable_line(&dead, reset).expect("a line for a dead upstream");
            assert!(line.contains("does not answer"), "{line}");
            assert!(line.ends_with(seen), "{reset}: {line}");
        }
    }

    fn res_options(env: &[std::ffi::CString]) -> Vec<String> {
        env.iter()
            .filter_map(|e| {
                e.to_str()
                    .ok()?
                    .strip_prefix("RES_OPTIONS=")
                    .map(str::to_string)
            })
            .collect()
    }

    // Issue 113: the pod adds single-request, and keeps a value that has it.
    #[test]
    fn res_options_gets_single_request() {
        let c = |s: &str| std::ffi::CString::new(s).unwrap();
        let mut env = vec![c("PATH=/bin")];
        with_single_request(&mut env);
        assert_eq!(res_options(&env), ["single-request"]);
        let mut env = vec![c("RES_OPTIONS=ndots:2")];
        with_single_request(&mut env);
        assert_eq!(res_options(&env), ["ndots:2 single-request"]);
        let mut env = vec![c("RES_OPTIONS=single-request-reopen")];
        with_single_request(&mut env);
        assert_eq!(res_options(&env), ["single-request-reopen"]);
        with_single_request(&mut env);
        assert_eq!(res_options(&env).len(), 1);
    }

    // T-383: use-vc goes after the options of the program, once.
    #[test]
    fn res_options_gets_use_vc_once() {
        let c = |s: &str| std::ffi::CString::new(s).unwrap();
        let mut env = vec![c("RES_OPTIONS=ndots:2")];
        with_single_request(&mut env);
        assert!(with_res_option(&mut env, "use-vc", &[]));
        assert_eq!(res_options(&env), ["ndots:2 single-request use-vc"]);
        assert!(!with_res_option(&mut env, "use-vc", &[]));
        assert_eq!(res_options(&env), ["ndots:2 single-request use-vc"]);
    }

    #[test]
    fn fake_dns_is_stable_and_reversible() {
        let mut dns = FakeDns::new();
        let a = dns.address_for("example.com").unwrap();
        let b = dns.address_for("example.com").unwrap();
        assert_eq!(a, b, "the same name must map to the same fake ip");
        assert_eq!(dns.name_for(a), Some("example.com"));
        let other = dns.address_for("example.org").unwrap();
        assert_ne!(a, other);
        assert!(a.octets()[0] == 198 && a.octets()[1] == 18);
    }
}
