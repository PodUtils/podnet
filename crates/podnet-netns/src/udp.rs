//! The UDP sockets of the program under the seccomp backend: the entry of
//! a socket, and its SOCKS5 UDP relay.
//!
//! The relay of a socket has reader tasks. Each task holds a copy of the
//! socketpair, of the inbox and of the relay socket (or of the control
//! connection). The entry holds the stop signal of these tasks: the drop of
//! the entry (a `close`, a sweep, the end of the pod) stops each task, and
//! the descriptors close (T-142).
//!
//! The filter does not stop `write(2)` (a notification for each write of
//! each program costs too much). A datagram that the program writes on the
//! socketpair goes to a reader task of the socket, which gives it, with the
//! address of `connect`, to the supervisor loop (T-143). The loop sends it
//! as `sendto` does. A datagram written on a socket with no `connect` has no
//! destination: the supervisor drops it, and the `write` of the program
//! gives no `EDESTADDRREQ`.

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

/// The most datagrams that wait between the readers of the socketpairs and
/// the supervisor loop. A full queue stops the readers; the socketpair then
/// fills, and the `write` of the program waits.
pub(crate) const WRITES: usize = 256;

/// A datagram that the program wrote on the socket `key`, and the address
/// of `connect` at that time.
pub(crate) struct Written {
    key: Key,
    to: Option<(IpAddr, u16)>,
    payload: Vec<u8>,
}

/// The queue of the written datagrams to the supervisor loop.
pub(crate) type Writes = tokio::sync::mpsc::Sender<Written>;

/// A UDP socket of the program.
pub(crate) struct Udp {
    /// The end of the supervisor of the socketpair.
    pub(crate) pair: Arc<tokio::net::UnixDatagram>,
    pub(crate) relay: Option<UdpRelay>,
    /// The address of `bind` (T-144). No datagram arrives at it: the relay
    /// gives only the answers to the datagrams of the socket.
    bound: Option<(IpAddr, u16)>,
    /// The options that the program set (T-148): an echo for `getsockopt`.
    /// SOCKS5 carries no IP option, so a value has no effect upstream.
    pub(crate) options: crate::sockopt::Options,
    /// The receive side: the sources of the datagrams (issue 305).
    pub(crate) inbox: Arc<Inbox>,
    /// The tasks of the socket stop when this sender drops.
    stop: tokio::sync::watch::Sender<()>,
}

impl Udp {
    /// The entry of the socket `key`. Start the reader of the datagrams
    /// that the program writes on the socketpair.
    pub(crate) fn new(
        pair: Arc<tokio::net::UnixDatagram>,
        inbox: Arc<Inbox>,
        (key, writes): (Key, &Writes),
    ) -> Self {
        let udp = Udp {
            pair,
            relay: None,
            bound: None,
            options: Default::default(),
            inbox,
            stop: tokio::sync::watch::channel(()).0,
        };
        let (pair, inbox, stopped) = (udp.pair.clone(), udp.inbox.clone(), udp.stopped());
        let writes = writes.clone();
        tokio::spawn(async move {
            let reads = async {
                let mut buf = vec![0u8; MAX_DATAGRAM + 1];
                while let Ok(n) = pair.recv(&mut buf).await {
                    let written = Written {
                        key,
                        to: inbox.dest(),
                        payload: buf[..n].to_vec(),
                    };
                    if writes.send(written).await.is_err() {
                        break;
                    }
                }
            };
            tokio::select! {
                _ = reads => {}
                _ = stopped => {}
            }
        });
        udp
    }

    /// `bind` of the descriptor `fd` to `to`. Port 0 gets a stable port of
    /// the ephemeral range. A second `bind` gets `EINVAL`, as in the kernel.
    /// An address that is not `AF_INET` or `AF_INET6` is not kept.
    pub(crate) fn bind(&mut self, fd: i32, to: Option<(IpAddr, u16)>) -> Result<(), i32> {
        if self.bound.is_some() {
            return Err(libc::EINVAL);
        }
        if let Some((ip, port)) = to {
            let port = match port {
                0 => crate::sockopt::ephemeral_port(fd),
                port => port,
            };
            self.bound = Some((ip, port));
        }
        Ok(())
    }

    /// The address of `getsockname` (`peer` false) of the descriptor `fd`,
    /// or of `getpeername`. `None`: the socket has no peer.
    pub(crate) fn name(&self, fd: i32, peer: bool) -> Option<(IpAddr, u16)> {
        let dest = self.inbox.dest();
        if peer {
            return dest;
        }
        if let Some(bound) = self.bound {
            return Some(bound);
        }
        let ip = match self.inbox.v6() {
            true => IpAddr::V6(std::net::Ipv6Addr::UNSPECIFIED),
            false => IpAddr::V4(Ipv4Addr::UNSPECIFIED),
        };
        // The kernel binds a port at the first connect or send.
        let port = match dest.is_some() || self.relay.is_some() {
            true => crate::sockopt::ephemeral_port(fd),
            false => 0,
        };
        Some((ip, port))
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

/// Send a datagram that the program wrote with `write(2)` (T-143). A
/// datagram with no destination, or one that fails, is dropped: the `write`
/// of the program has its answer already. `-v` prints one line for each.
pub(crate) async fn relay_written(
    map: &mut HashMap<Key, Sock>,
    written: Written,
    dns: &mut FakeDns,
    proxies: &[Proxy],
    verbose: bool,
) {
    let Written { key, to, payload } = written;
    if !matches!(map.get(&key), Some(Sock::Datagram(_))) {
        return;
    }
    let Some(to) = to else {
        if verbose {
            eprintln!(
                "podnet-netns: a datagram written on a UDP socket with no connect is dropped"
            );
        }
        return;
    };
    if let Err(errno) = send_payload(map, key, dns, proxies, &payload, to).await {
        if verbose {
            let error = std::io::Error::from_raw_os_error(errno);
            eprintln!("podnet-netns: a datagram written on a UDP socket is dropped: {error}");
        }
    }
}
