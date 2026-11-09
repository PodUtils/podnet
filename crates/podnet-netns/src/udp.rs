//! The UDP sockets of the program under the seccomp backend: the entry of
//! a socket, and its SOCKS5 UDP relay.
//!
//! The relay of a socket has reader tasks. Each task holds a copy of the
//! socketpair, of the inbox and of the relay socket (or of the control
//! connection). The entry holds the stop signal of these tasks: the drop of
//! the entry (a `close`, a sweep, the end of the pod) stops each task, and
//! the descriptors close (T-142).

use crate::inbox::Inbox;
use crate::prelude::*;

/// A live SOCKS5 UDP association.
pub(crate) enum UdpRelay {
    /// `UDP ASSOCIATE`: the relay socket, its address, and the control
    /// connection that must stay open for the association to live.
    Socket {
        udp: Arc<tokio::net::UdpSocket>,
        relay_addr: SocketAddr,
        _control: Upstream,
    },
    /// The datagrams go as frames on the control connection (issue 84):
    /// no kernel UDP socket. A writer task owns the write half.
    Stream {
        tx: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
    },
}

/// A UDP socket of the program.
pub(crate) struct Udp {
    /// The end of the supervisor of the socketpair.
    pub(crate) pair: Arc<tokio::net::UnixDatagram>,
    pub(crate) dest: Option<(IpAddr, u16)>,
    pub(crate) relay: Option<UdpRelay>,
    /// The receive side: the sources of the datagrams (issue 305).
    pub(crate) inbox: Arc<Inbox>,
    /// The tasks of the socket stop when this sender drops.
    stop: tokio::sync::watch::Sender<()>,
}

impl Udp {
    pub(crate) fn new(pair: Arc<tokio::net::UnixDatagram>, inbox: Arc<Inbox>) -> Self {
        Udp {
            pair,
            dest: None,
            relay: None,
            inbox,
            stop: tokio::sync::watch::channel(()).0,
        }
    }

    /// A future that ends when the entry of the socket drops.
    pub(crate) fn stopped(&self) -> impl std::future::Future<Output = ()> + Send + 'static {
        let mut stop = self.stop.subscribe();
        async move { while stop.changed().await.is_ok() {} }
    }
}

/// Make the UDP association of the socket `fd` when it has none.
pub(crate) async fn ensure_udp_relay(
    map: &mut HashMap<Key, Sock>,
    fd: Key,
    proxy: &Proxy,
) -> Result<()> {
    let (pair, inbox, stopped) = match map.get(&fd) {
        Some(Sock::Datagram(udp)) if udp.relay.is_none() => {
            (udp.pair.clone(), udp.inbox.clone(), udp.stopped())
        }
        _ => return Ok(()),
    };
    if proxy.kind != ProxyKind::Socks5 {
        bail!("UDP relay needs a SOCKS5 proxy");
    }
    // Issue 84: a proxy on a unix socket (warp proxy, a pod front door)
    // carries the datagrams on the control connection, so no kernel UDP
    // socket is necessary. A server that does not know the command gets
    // UDP ASSOCIATE.
    if proxy.is_unix() {
        let control = within_timeout("udp in the stream dial", dial_proxy(proxy)).await?;
        // On an error (0x07, or a server that closes on a command that it
        // does not know): UDP ASSOCIATE below.
        if let Ok(control) = within_timeout(
            "udp in the stream",
            crate::upstream::socks5_udp_in_stream(control, proxy),
        )
        .await
        {
            let (mut read, mut write) = tokio::io::split(control);
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
            // The writer ends when the entry drops its sender.
            tokio::spawn(async move {
                while let Some(datagram) = rx.recv().await {
                    if crate::upstream::write_udp_frame(&mut write, &datagram)
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            });
            tokio::spawn(async move {
                let frames = async {
                    while let Ok(Some(datagram)) = crate::upstream::read_udp_frame(&mut read).await
                    {
                        if let Some((source, off)) = crate::inbox::udp_source(&datagram) {
                            inbox.deliver(&pair, source, &datagram[off..]).await;
                        }
                    }
                };
                tokio::select! {
                    _ = frames => {}
                    _ = stopped => {}
                }
            });
            if let Some(Sock::Datagram(udp)) = map.get_mut(&fd) {
                udp.relay = Some(UdpRelay::Stream { tx });
            }
            return Ok(());
        }
    }
    let control = within_timeout("udp associate dial", dial_proxy(proxy)).await?;
    let (control, relay_addr) =
        within_timeout("udp associate", socks5_udp_associate(control, proxy)).await?;
    let socket = Arc::new(tokio::net::UdpSocket::bind("0.0.0.0:0").await?);
    let reader = socket.clone();
    tokio::spawn(async move {
        let datagrams = async {
            let mut buf = vec![0u8; 65536];
            while let Ok((n, _)) = reader.recv_from(&mut buf).await {
                if let Some((source, off)) = crate::inbox::udp_source(&buf[..n]) {
                    inbox.deliver(&pair, source, &buf[off..n]).await;
                }
            }
        };
        tokio::select! {
            _ = datagrams => {}
            _ = stopped => {}
        }
    });
    if let Some(Sock::Datagram(udp)) = map.get_mut(&fd) {
        udp.relay = Some(UdpRelay::Socket {
            udp: socket,
            relay_addr,
            _control: control,
        });
    }
    Ok(())
}

/// Send one datagram of the program through the UDP relay: wrap it in a
/// SOCKS5 UDP request header. A fake IP of the pod goes to the proxy as its
/// name (ATYP 3, issue 113): before, the proxy got the fake IP, which no
/// host has.
pub(crate) async fn relay_datagram(
    map: &mut HashMap<Key, Sock>,
    fd: Key,
    proxy: &Proxy,
    payload: &[u8],
    (ip, name, port): (IpAddr, Option<String>, u16),
) -> Result<()> {
    ensure_udp_relay(map, fd, proxy).await?;
    let Some(Sock::Datagram(udp)) = map.get_mut(&fd) else {
        return Ok(());
    };
    let wrapped = match name {
        Some(name) => {
            let wire = crate::inbox::WireSource::Name(name.clone(), port);
            udp.inbox.remember(wire, ip, port);
            crate::upstream::wrap_udp_name(payload, &name, port)
        }
        None => {
            let wire = crate::inbox::WireSource::Ip(SocketAddr::new(ip, port));
            udp.inbox.remember(wire, ip, port);
            wrap_udp(payload, ip, port)
        }
    };
    match &udp.relay {
        Some(UdpRelay::Socket {
            udp, relay_addr, ..
        }) => {
            udp.send_to(&wrapped, *relay_addr).await?;
        }
        Some(UdpRelay::Stream { tx }) if tx.send(wrapped).is_err() => {
            // The association ended: the next datagram opens a new one.
            udp.relay = None;
            bail!("the UDP association in the stream ended");
        }
        _ => {}
    }
    Ok(())
}

/// Send `payload` of the socket `fd` to `ip:port` for the program: a query
/// to port 53 gets its answer from the fake-IP pool, another datagram goes
/// through the relay. Return the byte count, or the errno for the program.
pub(crate) async fn send_payload(
    map: &mut HashMap<Key, Sock>,
    fd: Key,
    dns: &mut FakeDns,
    proxies: &[Proxy],
    payload: &[u8],
    (ip, port): (IpAddr, u16),
) -> Result<usize, i32> {
    let Some(Sock::Datagram(udp)) = map.get(&fd) else {
        return Err(libc::EBADF);
    };
    if port == DNS_PORT {
        let Ok(parsed) = crate::dns::ParsedQuery::parse(payload) else {
            return Err(libc::EINVAL);
        };
        let reply = dns.answer(&parsed);
        let source = crate::inbox::WireSource::Ip(SocketAddr::new(ip, port));
        let (pair, inbox) = (udp.pair.clone(), udp.inbox.clone());
        inbox.deliver(&pair, source, &reply).await;
        return Ok(payload.len());
    }
    let name = dns.name_for_ip(ip).map(str::to_string);
    crate::dgram::fits_relay(payload.len(), ip, name.as_deref())?;
    match relay_datagram(map, fd, &proxies[0], payload, (ip, name, port)).await {
        Ok(()) => Ok(payload.len()),
        Err(_) => Err(libc::ENETUNREACH),
    }
}
