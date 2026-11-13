//! The parent side of the `netns-kernel` backend (T-153): the tasks that
//! serve the descriptors that the child made in the namespace.
//!
//! * TCP: each accepted connection has its original destination in
//!   `SO_ORIGINAL_DST`. The task dials the upstream with the rules and
//!   `connect_upstream_async`, as the other backends do, and copies.
//! * DNS: each query gets the answer of the fake DNS of the pod.
//! * UDP: each client address is a flow ([`UdpFlows`]). The original
//!   destination comes from conntrack ([`crate::nfrules`]). A reply goes out
//!   of the redirect port: conntrack gives it the source of the original
//!   destination.

use crate::nfrules::udp_original_destination;
#[allow(unused_imports)]
use crate::prelude::*;
use std::net::SocketAddrV4;
use std::sync::Mutex;
use tokio::net::{TcpListener, TcpStream, UdpSocket};

/// What the tasks share.
pub(crate) struct Shared {
    proxies: Vec<Proxy>,
    rules: Vec<Rule>,
    dns: Mutex<FakeDns>,
    verbose: bool,
}

/// The running plane: its tasks end with the runtime.
pub(crate) struct Plane {
    _shared: Arc<Shared>,
}

/// The name of a fake address, else the address, with the port.
fn target_of(dns: &FakeDns, dst: Ipv4Addr, port: u16) -> String {
    match dns.name_for(dst) {
        Some(name) => format!("{name}:{port}"),
        None => format!("{dst}:{port}"),
    }
}

/// The original destination of an accepted TCP connection.
pub(crate) fn tcp_original_destination(fd: RawFd) -> std::io::Result<SocketAddrV4> {
    // SAFETY: a sockaddr_in of zeros is valid.
    let mut addr: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t;
    // SAFETY: getsockopt into a buffer that lives.
    let rc = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_IP,
            crate::nfnl::SO_ORIGINAL_DST,
            (&mut addr as *mut libc::sockaddr_in).cast(),
            &mut len,
        )
    };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(SocketAddrV4::new(
        Ipv4Addr::from(u32::from_be(addr.sin_addr.s_addr)),
        u16::from_be(addr.sin_port),
    ))
}

impl Plane {
    /// Serve `fds` (the TCP listener, the UDP socket, the DNS socket and the
    /// netfilter socket of the namespace). Call it inside a tokio runtime.
    pub(crate) fn start(
        fds: [RawFd; 4],
        proxies: Vec<Proxy>,
        rules: Vec<Rule>,
        maps: Vec<(String, IpAddr)>,
        verbose: bool,
    ) -> Result<Plane> {
        let mut dns = FakeDns::new();
        for (name, ip) in &maps {
            dns.pin(name, *ip);
        }
        let shared = Arc::new(Shared {
            proxies,
            rules,
            dns: Mutex::new(dns),
            verbose,
        });
        // SAFETY: the descriptors came over SCM_RIGHTS and nothing else owns them.
        let [tcp, udp, dns_fd, netlink] = fds.map(|fd| unsafe { OwnedFd::from_raw_fd(fd) });
        let tcp = std::net::TcpListener::from(tcp);
        tcp.set_nonblocking(true)?;
        let tcp = TcpListener::from_std(tcp)?;
        let udp = std::net::UdpSocket::from(udp);
        udp.set_nonblocking(true)?;
        let udp = UdpSocket::from_std(udp)?;
        let udp_port = udp.local_addr()?.port();
        let dns_socket = std::net::UdpSocket::from(dns_fd);
        dns_socket.set_nonblocking(true)?;
        let dns_socket = UdpSocket::from_std(dns_socket)?;
        tokio::spawn(serve_tcp(shared.clone(), tcp));
        tokio::spawn(serve_dns(shared.clone(), dns_socket));
        tokio::spawn(serve_udp(shared.clone(), udp, udp_port, netlink));
        Ok(Plane { _shared: shared })
    }
}

async fn serve_tcp(shared: Arc<Shared>, listener: TcpListener) {
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                tokio::spawn(tcp_flow(shared.clone(), stream));
            }
            Err(e) => {
                if shared.verbose {
                    eprintln!("{}: accept: {e}", program_name());
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    }
}

/// Close a connection so that the program sees a reset.
fn reset(stream: TcpStream) {
    let linger = libc::linger {
        l_onoff: 1,
        l_linger: 0,
    };
    // SAFETY: setsockopt with a struct that lives, on a descriptor that `stream` owns.
    unsafe {
        libc::setsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_LINGER,
            (&linger as *const libc::linger).cast(),
            std::mem::size_of::<libc::linger>() as libc::socklen_t,
        );
    }
}

async fn tcp_flow(shared: Arc<Shared>, stream: TcpStream) {
    let dst = match tcp_original_destination(stream.as_raw_fd()) {
        Ok(dst) => dst,
        Err(e) => {
            if shared.verbose {
                eprintln!("{}: SO_ORIGINAL_DST: {e}", program_name());
            }
            return reset(stream);
        }
    };
    let target = match shared.dns.lock() {
        Ok(dns) => target_of(&dns, *dst.ip(), dst.port()),
        Err(_) => return reset(stream),
    };
    let picked = crate::learn::select(&shared.rules, &shared.proxies, &target, "tcp");
    let flow = crate::flowlog::decided("tcp", &target, &picked);
    let Some(chain) = picked.chain().map(<[Proxy]>::to_vec) else {
        crate::rules::note_refusal("connect", &target, picked.rule_name(), shared.verbose);
        return reset(stream);
    };
    if shared.verbose {
        let hops: Vec<String> = chain.iter().map(Proxy::endpoint).collect();
        eprintln!("{}: {target} via {}", program_name(), hops.join(" -> "));
    }
    let dialed = within_timeout("upstream connect", connect_upstream_async(&chain, &target)).await;
    match dialed {
        Ok(upstream) => crate::flowlog::relay(stream, upstream, flow, &target).await,
        Err(e) => {
            if shared.verbose {
                eprintln!(
                    "{}: upstream for {target} failed: {e:#}; the connect of the program is reset",
                    program_name()
                );
            }
            reset(stream);
        }
    }
}

async fn serve_dns(shared: Arc<Shared>, socket: UdpSocket) {
    let mut buf = vec![0u8; 4096];
    loop {
        let Ok((n, from)) = socket.recv_from(&mut buf).await else {
            continue;
        };
        let Ok(query) = crate::dns::ParsedQuery::parse(&buf[..n]) else {
            continue;
        };
        let answer = match shared.dns.lock() {
            Ok(mut dns) => dns.answer(&query),
            Err(_) => continue,
        };
        let _ = socket.send_to(&answer, from).await;
    }
}

async fn serve_udp(shared: Arc<Shared>, socket: UdpSocket, port: u16, netlink: OwnedFd) {
    let mut flows = UdpFlows::new();
    let mut buf = vec![0u8; 65536];
    let mut reaper = tokio::time::interval(Duration::from_secs(5));
    loop {
        tokio::select! {
            received = socket.recv_from(&mut buf) => {
                let Ok((n, SocketAddr::V4(client))) = received else { continue };
                let dst = match udp_original_destination(netlink.as_raw_fd(), port, client) {
                    Ok(dst) => dst,
                    Err(e) => {
                        if shared.verbose {
                            eprintln!("{}: udp: {e}", program_name());
                        }
                        continue;
                    }
                };
                let key: UdpKey = (*client.ip(), client.port(), *dst.ip(), dst.port());
                let Ok(dns) = shared.dns.lock() else { continue };
                let route = UdpRoute {
                    proxies: &shared.proxies,
                    rules: &shared.rules,
                    dns: &dns,
                    host_gateway: false,
                    verbose: shared.verbose,
                };
                // A full queue loses the datagram, as a network does.
                let _ = flows.give(key, buf[..n].to_vec(), std::time::Instant::now(), &route);
            }
            Some((key, payload)) = flows.next_reply() => {
                let _ = socket.send_to(&payload, (key.0, key.1)).await;
            }
            _ = reaper.tick() => {
                drop(flows.table.reap(std::time::Instant::now()));
            }
        }
    }
}
