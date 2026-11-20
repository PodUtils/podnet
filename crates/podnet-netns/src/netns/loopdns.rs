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
//! The second path: the child binds UDP and TCP port 53 of the namespace
//! (it owns the namespace, so the port is free and allowed) and passes both
//! sockets to the parent with the TUN. The parent answers them from the
//! fake DNS of the pod. A socket stays in the namespace where it was made.
//!
//! The protocol on the socket of the parent, after the TUN: one byte, `0`
//! (no sockets) or `1`, then the UDP socket and the TCP listener.

#[allow(unused_imports)]
use crate::prelude::*;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

/// The sockets of port 53 of the namespace.
pub(crate) struct Sockets {
    udp: std::net::UdpSocket,
    tcp: std::net::TcpListener,
}

/// Bind UDP and TCP `0.0.0.0:53` in the namespace of this process. Port 53
/// of each loopback address (127.0.0.1, 127.0.0.53) is then local.
pub(crate) fn bind() -> std::io::Result<Sockets> {
    let at = std::net::SocketAddr::from(([0, 0, 0, 0], super::DNS_PORT));
    let udp = std::net::UdpSocket::bind(at)?;
    let tcp = std::net::TcpListener::bind(at)?;
    Ok(Sockets { udp, tcp })
}

/// Child: send the sockets, or the byte `0` when there are none. The
/// sockets close in the child; the parent has its copies.
pub(crate) fn send(sock: RawFd, sockets: Option<Sockets>) -> Result<()> {
    let Some(sockets) = sockets else {
        return super::write_all_fd(sock, &[0]);
    };
    super::write_all_fd(sock, &[1])?;
    super::send_fd(sock, sockets.udp.as_raw_fd())?;
    super::send_fd(sock, sockets.tcp.as_raw_fd())?;
    Ok(())
}

/// Parent: receive the sockets of [`send`]. A child that ended before the
/// byte sent none.
pub(crate) fn receive(parent: RawFd) -> Result<Option<Sockets>> {
    let mut flag = [0u8; 1];
    if super::read_exact_fd(parent, &mut flag)? != 1 || flag[0] != 1 {
        return Ok(None);
    }
    let udp = super::recv_fd(parent).context("receive the DNS socket of the namespace")?;
    // SAFETY: a descriptor that recv_fd gave to this process.
    let udp = unsafe { OwnedFd::from_raw_fd(udp) };
    let tcp = super::recv_fd(parent).context("receive the DNS listener of the namespace")?;
    // SAFETY: as above.
    let tcp = unsafe { OwnedFd::from_raw_fd(tcp) };
    Ok(Some(Sockets {
        udp: udp.into(),
        tcp: tcp.into(),
    }))
}

/// Parent, in the runtime of the backend: answer each query on the
/// sockets from the fake DNS (through `asker`), until the runtime ends.
pub(crate) fn serve(sockets: Sockets, asker: crate::dnstcp::Asker) -> Result<()> {
    sockets.udp.set_nonblocking(true)?;
    sockets.tcp.set_nonblocking(true)?;
    let udp = tokio::net::UdpSocket::from_std(sockets.udp)?;
    let tcp = tokio::net::TcpListener::from_std(sockets.tcp)?;
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
