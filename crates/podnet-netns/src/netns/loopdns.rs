//! The second path of the resolver of the netns backend (T-499).
//!
//! The first path is a private `resolv.conf` with the gateway of the TUN
//! ([`super::private_resolv_conf`]). Linux refuses the bind mount when
//! `/etc/resolv.conf` is a mount of a deleted file (`ENOENT`). The program
//! then reads the file of the host. A nameserver of another address is in
//! the TUN, and the pod answers it. A file with no nameserver sends glibc
//! to `127.0.0.1:53`, and a nameserver such as `127.0.0.53` is on the
//! loopback too: in the namespace, nothing answers there.
//!
//! The second path: the child binds UDP and TCP port 53 on 127.0.0.1 and on
//! each loopback nameserver of the host file (it owns the namespace, so the
//! port is free and allowed) and passes the sockets to the parent with the
//! TUN. The parent answers them from the
//! fake DNS of the pod. A socket stays in the namespace where it was made.
//!
//! The protocol on the socket of the parent, after the TUN: one byte, the
//! count of the addresses (`0`: none), then the UDP socket and the TCP
//! listener of each.

#[allow(unused_imports)]
use crate::prelude::*;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

/// The sockets of port 53 of the namespace: one UDP socket and one TCP
/// listener on each address.
pub(crate) struct Sockets {
    pairs: Vec<(std::net::UdpSocket, std::net::TcpListener)>,
}

/// The loopback addresses where the program asks: `127.0.0.1` (the default
/// of glibc with no `nameserver`), and each loopback nameserver of
/// `resolv_conf` (`127.0.0.53` of systemd-resolved, `::1`). glibc takes an
/// answer only from the address that it asked, so the pod binds each one
/// (a socket on `0.0.0.0` answers from `127.0.0.1`: glibc drops that).
fn addresses(resolv_conf: &str) -> Vec<std::net::IpAddr> {
    let mut out = vec![std::net::IpAddr::from([127, 0, 0, 1])];
    for line in resolv_conf.lines() {
        let mut words = line.split_whitespace();
        if words.next() != Some("nameserver") {
            continue;
        }
        let Some(Ok(ip)) = words.next().map(str::parse::<std::net::IpAddr>) else {
            continue;
        };
        if ip.is_loopback() && !out.contains(&ip) {
            out.push(ip);
        }
    }
    out
}

/// Bind UDP and TCP port 53 on each address of [`addresses`] in the
/// namespace of this process. An address that cannot be bound (no IPv6 on
/// the loopback) is left out; with none bound, the error of the first.
pub(crate) fn bind() -> std::io::Result<Sockets> {
    let conf = std::fs::read_to_string("/etc/resolv.conf").unwrap_or_default();
    let mut pairs = Vec::new();
    let mut first = None;
    for ip in addresses(&conf) {
        let at = std::net::SocketAddr::new(ip, super::DNS_PORT);
        let bound = std::net::UdpSocket::bind(at)
            .and_then(|udp| Ok((udp, std::net::TcpListener::bind(at)?)));
        match bound {
            Ok(pair) => pairs.push(pair),
            Err(e) => {
                first.get_or_insert(e);
            }
        }
    }
    match (pairs.is_empty(), first) {
        (true, Some(e)) => Err(e),
        _ => Ok(Sockets { pairs }),
    }
}

/// Child: send the count of the pairs (one byte; `0`: none), then the UDP
/// socket and the TCP listener of each. The sockets close in the child; the
/// parent has its copies.
pub(crate) fn send(sock: RawFd, sockets: Option<Sockets>) -> Result<()> {
    let pairs = sockets.map(|s| s.pairs).unwrap_or_default();
    let count = u8::try_from(pairs.len()).unwrap_or(u8::MAX);
    super::write_all_fd(sock, &[count])?;
    for (udp, tcp) in pairs.iter().take(usize::from(count)) {
        super::send_fd(sock, udp.as_raw_fd())?;
        super::send_fd(sock, tcp.as_raw_fd())?;
    }
    Ok(())
}

/// Parent: receive the sockets of [`send`]. A child that ended before the
/// byte sent none.
pub(crate) fn receive(parent: RawFd) -> Result<Option<Sockets>> {
    let mut count = [0u8; 1];
    if super::read_exact_fd(parent, &mut count)? != 1 || count[0] == 0 {
        return Ok(None);
    }
    let mut pairs = Vec::new();
    for _ in 0..count[0] {
        let udp = super::recv_fd(parent).context("receive a DNS socket of the namespace")?;
        // SAFETY: a descriptor that recv_fd gave to this process.
        let udp = unsafe { OwnedFd::from_raw_fd(udp) };
        let tcp = super::recv_fd(parent).context("receive a DNS listener of the namespace")?;
        // SAFETY: as above.
        let tcp = unsafe { OwnedFd::from_raw_fd(tcp) };
        pairs.push((udp.into(), tcp.into()));
    }
    Ok(Some(Sockets { pairs }))
}

/// Parent, in the runtime of the backend: answer each query on the
/// sockets from the fake DNS (through `asker`), until the runtime ends.
pub(crate) fn serve(sockets: Sockets, asker: crate::dnstcp::Asker) -> Result<()> {
    for (udp, tcp) in sockets.pairs {
        serve_pair(udp, tcp, asker.clone())?;
    }
    Ok(())
}

/// Answer the queries of one address: each datagram, and each connection.
fn serve_pair(
    udp: std::net::UdpSocket,
    tcp: std::net::TcpListener,
    asker: crate::dnstcp::Asker,
) -> Result<()> {
    udp.set_nonblocking(true)?;
    tcp.set_nonblocking(true)?;
    let udp = tokio::net::UdpSocket::from_std(udp)?;
    let tcp = tokio::net::TcpListener::from_std(tcp)?;
    let datagrams = asker.clone();
    tokio::spawn(async move {
        let udp = std::sync::Arc::new(udp);
        let mut buf = vec![0u8; 64 * 1024];
        while let Ok((n, from)) = udp.recv_from(&mut buf).await {
            let (udp, asker, query) = (udp.clone(), datagrams.clone(), buf[..n].to_vec());
            tokio::spawn(async move {
                if let Some(answer) = asker.ask(query).await {
                    let _ = udp.send_to(&answer, from).await;
                }
            });
        }
    });
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = tcp.accept().await {
            let mut pod = asker.connect();
            tokio::spawn(async move {
                let _ = tokio::io::copy_bidirectional(&mut stream, &mut pod).await;
            });
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T-499: the pod binds 127.0.0.1 and each loopback nameserver of the
    /// host file, once each, and no other address (a nameserver of another
    /// address is in the TUN, where the pod answers it).
    #[test]
    fn the_pod_answers_at_each_loopback_nameserver() {
        let ip = |s: &str| s.parse::<std::net::IpAddr>().unwrap();
        for (conf, want) in [
            ("options attempts:1\n", vec!["127.0.0.1"]),
            (
                "nameserver 127.0.0.53\noptions edns0 trust-ad\nsearch .\n",
                vec!["127.0.0.1", "127.0.0.53"],
            ),
            (
                "nameserver 8.8.8.8\nnameserver ::1\nnameserver 127.0.0.1\n",
                vec!["127.0.0.1", "::1"],
            ),
            (
                "nameserver bogus\n# nameserver 127.0.0.9\n",
                vec!["127.0.0.1"],
            ),
        ] {
            let want: Vec<_> = want.into_iter().map(ip).collect();
            assert_eq!(addresses(conf), want, "{conf:?}");
        }
    }
}
