//! The `netns-kernel` backend (T-153): the pod of the `netns` backend with
//! the netfilter of its own namespace as the data plane.
//!
//! The child makes a user and network namespace, and brings `lo` up. It adds
//! one route that makes each IPv4 address local (`local default dev lo`), so
//! a `connect` to any address gets past the route lookup. It installs the
//! `nat` rules of [`crate::nfrules`] and makes three listeners on `lo` (TCP,
//! UDP and DNS) and a netfilter netlink socket. It sends the four
//! descriptors to the parent over the `SCM_RIGHTS` pair and starts the
//! program. The kernel redirects each flow of the program to a listener;
//! there is no TUN, no `smoltcp` stack and no flow table. The parent serves
//! the descriptors ([`crate::kernel_serve`]).
//!
//! The limits: IPv4 only; a destination on 127.0.0.0/8 stays in the
//! namespace; a ping of another address is answered by the namespace (an
//! ICMP echo cannot be redirected).

use crate::nfrules::{self, Ports};
#[allow(unused_imports)]
use crate::prelude::*;

/// The network namespace is gone when the program is: run the pod until no
/// process lives in it. Same contract as the `netns` backend.
pub(crate) fn run(args: Args) -> Result<i32> {
    let verbose = args.verbose;
    crate::outlive::split(verbose, |reporter| run_pod(args, reporter))
}

/// The report of the reaper: the status of the first program (4 bytes) and
/// 1 when other processes of the pod live.
const REAPER_REPORT: usize = 5;

fn run_pod(args: Args, reporter: crate::outlive::Reporter) -> Result<i32> {
    let (parent, child) = socketpair()?;
    // SAFETY: fork; the child only calls `child_setup` and `_exit`.
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        bail!("fork: {}", std::io::Error::last_os_error());
    }
    if pid == 0 {
        // SAFETY: `parent` is open in the child; it is not used there.
        unsafe { libc::close(parent) };
        reporter.close_in_child();
        if let Err(e) = child_setup(child, &args) {
            eprintln!("{}: {e:#}", program_name());
            // SAFETY: the child ends here.
            unsafe { libc::_exit(EXIT_NO_BACKEND) };
        }
        // SAFETY: the child ends here.
        unsafe { libc::_exit(0) };
    }
    // SAFETY: `child` belongs to the child process now.
    unsafe { libc::close(child) };
    let (uid, gid) = unsafe { (libc::getuid(), libc::getgid()) };
    pod_hold::netns::answer(parent, pid as u32, uid, gid).context("write uid/gid maps")?;
    // The four descriptors of the plane, in this order.
    let mut fds = Vec::new();
    for what in [
        "TCP listener",
        "UDP socket",
        "DNS socket",
        "netfilter socket",
    ] {
        fds.push(recv_fd(parent).with_context(|| format!("receive the {what}"))?);
    }
    // SAFETY: the reaper writes its report later; read it with no wait.
    unsafe { libc::fcntl(parent, libc::F_SETFL, libc::O_NONBLOCK) };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let verbose = args.verbose;
    let result = runtime.block_on(async move {
        let plane = crate::kernel_serve::Plane::start(
            [fds[0], fds[1], fds[2], fds[3]],
            args.proxies.clone(),
            args.rules.clone(),
            args.maps.clone(),
            verbose,
        )?;
        let _plane = plane;
        let mut report = Vec::with_capacity(REAPER_REPORT);
        let mut first: Option<i32> = None;
        loop {
            tokio::time::sleep(Duration::from_millis(20)).await;
            if first.is_none() && read_report(parent, &mut report) {
                let status = i32::from_le_bytes([report[0], report[1], report[2], report[3]]);
                let code = crate::outlive::exit_code(status);
                if report[4] == 0 {
                    return Ok::<i32, anyhow::Error>(code);
                }
                if !reporter.outlives() {
                    // SAFETY: kill of our own child.
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
            // SAFETY: waitpid of our own child.
            if unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) } == pid {
                return Ok(first.unwrap_or_else(|| crate::outlive::exit_code(status)));
            }
        }
    });
    // SAFETY: `parent` is open and not used again.
    unsafe { libc::close(parent) };
    let mut status = 0;
    // SAFETY: reap the child if the loop did not.
    unsafe { libc::waitpid(pid, &mut status, 0) };
    result
}

fn read_report(fd: RawFd, report: &mut Vec<u8>) -> bool {
    let mut buf = [0u8; REAPER_REPORT];
    let want = REAPER_REPORT - report.len();
    // SAFETY: read into a buffer that lives.
    let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), want) };
    if n > 0 {
        report.extend_from_slice(&buf[..n as usize]);
    }
    report.len() == REAPER_REPORT
}

/// Add the route `local default dev lo` (RTM_NEWROUTE): the namespace
/// treats each IPv4 address as its own, with the source address 127.0.0.1.
fn local_default_route() -> Result<()> {
    const RTM_NEWROUTE: u16 = 24;
    const NLM_F_REQUEST: u16 = 1;
    const NLM_F_ACK: u16 = 4;
    const NLM_F_EXCL: u16 = 0x200;
    const NLM_F_CREATE: u16 = 0x400;
    const RT_TABLE_MAIN: u8 = 254;
    const RTPROT_BOOT: u8 = 3;
    const RT_SCOPE_HOST: u8 = 254;
    const RTN_LOCAL: u8 = 2;
    const RTA_OIF: u16 = 4;
    const RTA_PREFSRC: u16 = 7;
    let lo = 1u32; // the index of `lo` in a new namespace
    let mut m = Vec::new();
    m.extend_from_slice(&0u32.to_ne_bytes()); // the length, set below
    m.extend_from_slice(&RTM_NEWROUTE.to_ne_bytes());
    m.extend_from_slice(&(NLM_F_REQUEST | NLM_F_ACK | NLM_F_EXCL | NLM_F_CREATE).to_ne_bytes());
    m.extend_from_slice(&1u32.to_ne_bytes()); // seq
    m.extend_from_slice(&0u32.to_ne_bytes()); // pid
                                              // rtmsg: family, dst_len, src_len, tos, table, protocol, scope, type, flags
    m.extend_from_slice(&[
        libc::AF_INET as u8,
        0,
        0,
        0,
        RT_TABLE_MAIN,
        RTPROT_BOOT,
        RT_SCOPE_HOST,
        RTN_LOCAL,
    ]);
    m.extend_from_slice(&0u32.to_ne_bytes());
    for (ty, data) in [
        (RTA_OIF, lo.to_ne_bytes()),
        (RTA_PREFSRC, Ipv4Addr::LOCALHOST.octets()),
    ] {
        m.extend_from_slice(&8u16.to_ne_bytes());
        m.extend_from_slice(&ty.to_ne_bytes());
        m.extend_from_slice(&data);
    }
    let len = m.len() as u32;
    m[..4].copy_from_slice(&len.to_ne_bytes());
    // SAFETY: socket, send, recv and close of buffers and a descriptor that live.
    unsafe {
        let fd = libc::socket(
            libc::AF_NETLINK,
            libc::SOCK_RAW | libc::SOCK_CLOEXEC,
            libc::NETLINK_ROUTE,
        );
        if fd < 0 {
            bail!("socket(NETLINK_ROUTE): {}", std::io::Error::last_os_error());
        }
        let sent = libc::send(fd, m.as_ptr().cast(), m.len(), 0);
        let mut answer = [0u8; 256];
        let got = libc::recv(fd, answer.as_mut_ptr().cast(), answer.len(), 0);
        let error = std::io::Error::last_os_error();
        libc::close(fd);
        if sent < 0 || got < 20 {
            bail!("the route of the namespace: {error}");
        }
        let code = i32::from_ne_bytes(answer[16..20].try_into().unwrap_or([0; 4]));
        if code != 0 {
            bail!(
                "the route local default dev lo: {}",
                std::io::Error::from_raw_os_error(-code)
            );
        }
    }
    Ok(())
}

/// Let the groups of the namespace make ping sockets: the range is the
/// mapped group (a user namespace maps one group), best effort.
fn ping_group_range(gid: u32) {
    let _ = std::fs::write(
        "/proc/sys/net/ipv4/ping_group_range",
        format!("{gid} {gid}\n"),
    );
}

/// The child: the namespace, the route, the rules and the listeners; then
/// the program. Return only on an error.
fn child_setup(sock: RawFd, args: &Args) -> Result<()> {
    let userns = pod_hold::netns::enter()?;
    pod_hold::netns::announce(sock, userns)?;
    bringup_loopback()?;
    local_default_route()?;
    let tcp = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).context("TCP listener")?;
    let udp = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).context("UDP socket")?;
    let dns = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).context("DNS socket")?;
    let port = |r: std::io::Result<SocketAddr>| r.map(|a| a.port()).context("local address");
    let ports = Ports {
        tcp: port(tcp.local_addr())?,
        udp: port(udp.local_addr())?,
        dns: port(dns.local_addr())?,
    };
    let netlink = crate::nfnl::open_socket(2).context("socket(NETLINK_NETFILTER)")?;
    nfrules::install(netlink.as_raw_fd(), &nfrules::batch(ports)).map_err(anyhow::Error::msg)?;
    ping_group_range(unsafe { libc::getgid() });
    for fd in [
        tcp.as_raw_fd(),
        udp.as_raw_fd(),
        dns.as_raw_fd(),
        netlink.as_raw_fd(),
    ] {
        send_fd(sock, fd)?;
    }
    // The parent owns the descriptors now: the program must not get them.
    drop((tcp, udp, dns, netlink));
    if let Err(e) = private_resolv_conf() {
        crate::warn(&format!(
            "warning: the program keeps the resolver of the host ({e:#})"
        ));
    }
    exec_program(sock, args)
}
