//! The stack of the netns backend: the `smoltcp` interface on the TUN,
//! the held SYNs, the upstream dials and the flows.

#[allow(unused_imports)]
use crate::prelude::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// The addresses of a TCP connection of the program: (source, source port,
/// destination, destination port).
pub(crate) type SynKey = (Ipv4Addr, u16, Ipv4Addr, u16);

/// A SYN of the program and the dial of its upstream.
pub(crate) struct HeldSyn {
    pub(crate) pkt: Vec<u8>,
    pub(crate) target: String,
    pub(crate) task: tokio::task::JoinHandle<Result<Upstream>>,
}

/// The time that the upstream of a released SYN waits for its socket.
const PREDIALED_TTL: std::time::Duration = std::time::Duration::from_secs(30);

/// One outbound flow: a smoltcp TCP socket and the upstream proxy connection.
pub(crate) struct Flow {
    pub(crate) upstream: Upstream,
    pub(crate) to_upstream: Vec<u8>,
    pub(crate) to_app: Vec<u8>,
    pub(crate) up_eof: bool,
}

pub(crate) struct Backend {
    pub(crate) device: Tun,
    pub(crate) interface: Interface,
    pub(crate) sockets: SocketSet<'static>,
    pub(crate) dns: FakeDns,
    /// destination port → listener handle created for it
    /// Every TCP socket of the stack, with its destination port. A socket
    /// starts in the listen state and becomes one connection.
    pub(crate) listeners: HashMap<SocketHandle, u16>,
    /// Upstream dials in progress, for connections that are established in
    /// the stack but have no flow yet.
    pub(crate) dialing: HashMap<SocketHandle, (String, tokio::task::JoinHandle<Result<Upstream>>)>,
    pub(crate) flows: HashMap<SocketHandle, Flow>,
    /// Issue 190: each new SYN waits here while its upstream is dialed. The
    /// stack sees the SYN only when the dial succeeds; a failed dial gets a
    /// RST, so `connect()` of the program fails.
    pub(crate) held: HashMap<SynKey, HeldSyn>,
    /// The upstreams of released SYNs, until their socket is established.
    pub(crate) predialed: HashMap<SynKey, (String, Upstream, std::time::Instant)>,
    /// Sockets aborted after the last poll. They stay until the next poll
    /// has sent their RST.
    pub(crate) aborting: Vec<SocketHandle>,
    /// The UDP flows of the program other than DNS (T-111).
    pub(crate) udp: UdpFlows,
    pub(crate) proxies: Vec<Proxy>,
    pub(crate) rules: Vec<Rule>,
    pub(crate) verbose: bool,
}

impl Backend {
    pub(crate) fn new(
        fd: RawFd,
        proxies: Vec<Proxy>,
        rules: Vec<Rule>,
        maps: Vec<(String, IpAddr)>,
        verbose: bool,
    ) -> Result<Self> {
        let mut device = Tun::new(fd, TUN_MTU);
        let mut config = IfaceConfig::new(HardwareAddress::Ip);
        config.random_seed = seed();
        let mut interface = Interface::new(config, &mut device, SmolInstant::now());
        interface.update_ip_addrs(|addrs| {
            addrs
                .push(IpCidr::new(IpAddress::Ipv4(smol_v4(TUN_GW)), TUN_PREFIX))
                .unwrap();
        });
        interface
            .routes_mut()
            .add_default_ipv4_route(smol_v4(TUN_GW))
            .unwrap();
        // Accept packets addressed to any destination: the guest connects to
        // real addresses, and we impersonate all of them.
        interface.set_any_ip(true);

        let sockets = SocketSet::new(Vec::new());
        let mut dns = FakeDns::new();
        for (name, ip) in &maps {
            dns.pin(name, *ip);
        }
        Ok(Self {
            device,
            interface,
            sockets,
            dns,
            listeners: HashMap::new(),
            dialing: HashMap::new(),
            flows: HashMap::new(),
            held: HashMap::new(),
            predialed: HashMap::new(),
            aborting: Vec::new(),
            udp: UdpFlows::new(),
            proxies,
            rules,
            verbose,
        })
    }

    /// The target of a connection to `dst:port`: the name of a fake address,
    /// else the address.
    fn target_of(&self, dst: Ipv4Addr, port: u16) -> String {
        match self.dns.name_for(dst) {
            Some(name) => format!("{name}:{port}"),
            None => format!("{dst}:{port}"),
        }
    }

    /// Start the dial of the upstream of `target`.
    fn spawn_dial(&self, target: &str) -> tokio::task::JoinHandle<Result<Upstream>> {
        let chain = select_chain(&self.rules, &self.proxies, target).to_vec();
        if self.verbose {
            let hops: Vec<String> = chain.iter().map(Proxy::endpoint).collect();
            eprintln!("podnet-netns: {target} via {}", hops.join(" -> "));
        }
        let dial_target = target.to_string();
        tokio::spawn(async move {
            within_timeout(
                "upstream connect",
                connect_upstream_async(&chain, &dial_target),
            )
            .await
        })
    }

    /// A socket of the stack has the addresses of `key`.
    fn has_connection(&self, key: &SynKey) -> bool {
        let (src, sport, dst, dport) = *key;
        self.listeners.keys().any(|handle| {
            let socket = self.sockets.get::<tcp::Socket>(*handle);
            socket
                .remote_endpoint()
                .is_some_and(|r| r.addr == IpAddress::Ipv4(src) && r.port == sport)
                && socket
                    .local_endpoint()
                    .is_some_and(|l| l.addr == IpAddress::Ipv4(dst) && l.port == dport)
        })
    }

    /// Issue 190: hold each new SYN until its upstream is dialed. Then give
    /// the SYN to the stack, or answer it with a RST when the dial fails.
    pub(crate) async fn hold_syns(&mut self) {
        let mut kept = VecDeque::with_capacity(self.device.rx.len());
        while let Some(pkt) = self.device.rx.pop_front() {
            let Some(key) = parse_tcp_syn(&pkt) else {
                kept.push_back(pkt);
                continue;
            };
            if self.predialed.contains_key(&key) || self.has_connection(&key) {
                // A released SYN, or a SYN again for a known connection.
                kept.push_back(pkt);
            } else if self.held.contains_key(&key) {
                // A SYN again while the dial runs: the first one is held.
            } else if self.held.len() + self.listeners.len() >= MAX_TRACKED_FDS {
                // The table is full: the stack drops the SYN.
                kept.push_back(pkt);
            } else {
                let target = self.target_of(key.2, key.3);
                let task = self.spawn_dial(&target);
                self.held.insert(key, HeldSyn { pkt, target, task });
            }
        }
        self.device.rx = kept;

        let done: Vec<SynKey> = self
            .held
            .iter()
            .filter(|(_, held)| held.task.is_finished())
            .map(|(key, _)| *key)
            .collect();
        for key in done {
            let held = self.held.remove(&key).expect("listed above");
            let result = match held.task.await {
                Ok(result) => result,
                Err(e) => Err(anyhow::anyhow!("dial task: {e}")),
            };
            match result {
                Ok(upstream) => {
                    self.predialed
                        .insert(key, (held.target, upstream, std::time::Instant::now()));
                    self.device.rx.push_front(held.pkt);
                }
                Err(e) => {
                    if self.verbose {
                        eprintln!(
                            "podnet-netns: upstream for {} failed: {e:#}; the connect of the program is refused",
                            held.target
                        );
                    }
                    if let Some(rst) = rst_reply_for_syn(&held.pkt) {
                        self.device.tx.push_back(rst);
                    }
                }
            }
        }
        self.predialed
            .retain(|_, (_, _, at)| at.elapsed() < PREDIALED_TTL);
    }

    /// Each SYN needs a socket in the listen state for its port before
    /// smoltcp processes the packet. A listen socket becomes the connection,
    /// so add one listen socket for each SYN that has no free one.
    pub(crate) fn observe_syns(&mut self) {
        let mut wanted: HashMap<u16, usize> = HashMap::new();
        for pkt in &self.device.rx {
            if let Some((_src, _sport, _dst, dport)) = parse_tcp_syn(pkt) {
                *wanted.entry(dport).or_default() += 1;
            }
        }
        for (port, count) in wanted {
            let free = self
                .listeners
                .iter()
                .filter(|(handle, p)| {
                    **p == port
                        && self.sockets.get::<tcp::Socket>(**handle).state() == tcp::State::Listen
                })
                .count();
            for _ in free..count {
                // Bound the table: a program that sweeps ports must not grow
                // it without limit.
                if self.listeners.len() >= MAX_TRACKED_FDS {
                    return;
                }
                let mut socket = tcp_socket();
                if socket.listen(IpListenEndpoint { addr: None, port }).is_ok() {
                    let handle = self.sockets.add(socket);
                    self.listeners.insert(handle, port);
                    if self.verbose {
                        eprintln!("podnet-netns: listening for outbound port {port}");
                    }
                }
            }
        }
    }

    /// Remove the sockets of finished connections from the stack.
    pub(crate) fn reap_closed(&mut self) {
        let closed: Vec<SocketHandle> = self
            .listeners
            .keys()
            .copied()
            .filter(|handle| {
                !self.flows.contains_key(handle)
                    && !self.dialing.contains_key(handle)
                    && !self.aborting.contains(handle)
                    && self.sockets.get::<tcp::Socket>(*handle).state() == tcp::State::Closed
            })
            .collect();
        for handle in closed {
            self.listeners.remove(&handle);
            self.sockets.remove(handle);
        }
    }

    /// Read the TUN, unless a full UDP queue pauses the read.
    pub(crate) fn read_tun(&mut self) {
        if !self.udp.paused() {
            self.device.read_ready();
        }
    }

    pub(crate) async fn tick(&mut self) -> Result<()> {
        self.answer_dns();
        let route = UdpRoute {
            proxies: &self.proxies,
            rules: &self.rules,
            dns: &self.dns,
            verbose: self.verbose,
        };
        self.udp
            .relay(&mut self.device.rx, &mut self.device.tx, &route);
        self.hold_syns().await;
        self.observe_syns();
        self.interface
            .poll(SmolInstant::now(), &mut self.device, &mut self.sockets);
        // Issue 190: this poll sent the RST of each aborted socket. Before,
        // the socket was removed first, and a client that waits (TLS) hung.
        self.aborting.clear();
        self.accept_flows().await;
        self.shuttle().await;
        self.reap_closed();
        self.device.flush();
        Ok(())
    }

    /// Answer each IPv4 UDP query to port 53 from the fake-IP pool, before
    /// smoltcp sees it.
    ///
    /// The reply must come from the address that the client sent to (the
    /// resolver in its resolv.conf), or a connected UDP socket drops it. A
    /// smoltcp UDP socket cannot set that source address, so the reply packet
    /// is made here with the two addresses swapped.
    pub(crate) fn answer_dns(&mut self) {
        let mut kept = VecDeque::with_capacity(self.device.rx.len());
        while let Some(pkt) = self.device.rx.pop_front() {
            match dns_reply_packet(&pkt, &mut self.dns) {
                Some(reply) => self.device.tx.push_back(reply),
                None => kept.push_back(pkt),
            }
        }
        self.device.rx = kept;
    }

    /// Start an upstream dial for each new connection, and make a flow when
    /// a dial completes. A dial runs in its own task, so a slow proxy does not
    /// stop the other flows.
    pub(crate) async fn accept_flows(&mut self) {
        let handles: Vec<SocketHandle> = self.listeners.keys().copied().collect();
        for handle in handles {
            if self.flows.contains_key(&handle) || self.dialing.contains_key(&handle) {
                continue;
            }
            // The destination is the local endpoint of the socket.
            let (key, target) = {
                let socket = self.sockets.get::<tcp::Socket>(handle);
                if !matches!(socket.state(), tcp::State::Established) {
                    continue;
                }
                let (Some(local), Some(remote)) =
                    (socket.local_endpoint(), socket.remote_endpoint())
                else {
                    continue;
                };
                let (IpAddress::Ipv4(dst_ip), IpAddress::Ipv4(src_ip)) = (local.addr, remote.addr)
                else {
                    continue;
                };
                (
                    (src_ip, remote.port, dst_ip, local.port),
                    self.target_of(dst_ip, local.port),
                )
            };
            // The usual case: the upstream was dialed before the handshake.
            if let Some((_, upstream, _)) = self.predialed.remove(&key) {
                self.flows.insert(
                    handle,
                    Flow {
                        upstream,
                        to_upstream: Vec::new(),
                        to_app: Vec::new(),
                        up_eof: false,
                    },
                );
                continue;
            }
            let task = self.spawn_dial(&target);
            self.dialing.insert(handle, (target, task));
        }

        let ready: Vec<SocketHandle> = self
            .dialing
            .iter()
            .filter(|(_, (_, task))| task.is_finished())
            .map(|(handle, _)| *handle)
            .collect();
        for handle in ready {
            let (target, task) = self.dialing.remove(&handle).expect("listed above");
            match task.await {
                Ok(Ok(upstream)) => {
                    self.flows.insert(
                        handle,
                        Flow {
                            upstream,
                            to_upstream: Vec::new(),
                            to_app: Vec::new(),
                            up_eof: false,
                        },
                    );
                }
                Ok(Err(e)) => {
                    if self.verbose {
                        eprintln!("podnet-netns: upstream for {target} failed: {e:#}");
                    }
                    self.sockets.get_mut::<tcp::Socket>(handle).abort();
                    self.aborting.push(handle);
                }
                Err(e) => {
                    if self.verbose {
                        eprintln!("podnet-netns: dial task for {target} failed: {e}");
                    }
                    self.sockets.get_mut::<tcp::Socket>(handle).abort();
                    self.aborting.push(handle);
                }
            }
        }
    }

    pub(crate) async fn shuttle(&mut self) {
        let handles: Vec<SocketHandle> = self.flows.keys().copied().collect();
        for handle in handles {
            let mut flow = self.flows.remove(&handle).unwrap();
            let socket = self.sockets.get_mut::<tcp::Socket>(handle);
            let mut close = false;

            // app → upstream
            while socket.can_recv() {
                match socket.recv(|buf| {
                    let n = buf.len().min(64 * 1024);
                    flow.to_upstream.extend_from_slice(&buf[..n]);
                    (n, ())
                }) {
                    Ok(()) => {}
                    Err(_) => break,
                }
            }
            while !flow.to_upstream.is_empty() {
                match poll_once(flow.upstream.write(&flow.to_upstream)).unwrap_or_else(would_block)
                {
                    Ok(0) => break,
                    Ok(n) => {
                        flow.to_upstream.drain(..n);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(_) => {
                        close = true;
                        break;
                    }
                }
            }

            // upstream → app
            let mut buf = [0u8; 16 * 1024];
            loop {
                match poll_once(flow.upstream.read(&mut buf)).unwrap_or_else(would_block) {
                    Ok(0) => {
                        flow.up_eof = true;
                        break;
                    }
                    Ok(n) => flow.to_app.extend_from_slice(&buf[..n]),
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(_) => {
                        close = true;
                        break;
                    }
                }
            }
            while !flow.to_app.is_empty() && socket.can_send() {
                match socket.send_slice(&flow.to_app) {
                    Ok(n) => {
                        flow.to_app.drain(..n);
                    }
                    Err(_) => break,
                }
            }

            if (flow.up_eof && flow.to_app.is_empty()) || close || !socket.may_send() {
                socket.close();
                self.flows.remove(&handle);
                continue;
            }
            self.flows.insert(handle, flow);
        }
    }
}

pub(crate) fn tcp_socket() -> tcp::Socket<'static> {
    let rx = tcp::SocketBuffer::new(vec![0u8; TCP_BUF]);
    let tx = tcp::SocketBuffer::new(vec![0u8; TCP_BUF]);
    let mut socket = tcp::Socket::new(rx, tx);
    socket.set_nagle_enabled(false);
    // As before smoltcp 0.12: no congestion controller. A build of the
    // workspace turns on the controllers of podnet, and smoltcp then takes
    // cubic by default.
    socket.set_congestion_control(tcp::CongestionControl::None);
    socket
}

/// Poll `future` one time. Return its output, or `None` when it is pending.
///
/// The stack loop wakes at a short interval, so a no-op waker is sufficient:
/// the I/O driver still records readiness for the next poll.
pub(crate) fn poll_once<F: std::future::Future>(future: F) -> Option<F::Output> {
    let mut future = std::pin::pin!(future);
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    match future.as_mut().poll(&mut context) {
        std::task::Poll::Ready(output) => Some(output),
        std::task::Poll::Pending => None,
    }
}

/// The result of an I/O call that did not complete.
pub(crate) fn would_block() -> std::io::Result<usize> {
    Err(std::io::ErrorKind::WouldBlock.into())
}

pub(crate) fn seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9e37_79b9_7f4a_7c15)
}
