use super::*;

// ── the reactor core ────────────────────────────────────────────────────────

pub(super) struct Conn {
    pub(super) handle: SocketHandle,
    pub(super) to_app: mpsc::Sender<AppEvent>,
    pub(super) from_app: mpsc::Receiver<Vec<u8>>,
    pub(super) write_waker: Arc<AtomicWaker>,
    pub(super) ctl: mpsc::Receiver<ConnCmd>,
    pub(super) closing: bool,
    pub(super) recv_eof_sent: bool,
    /// [`AppEvent::Connected`] was sent.
    pub(super) announced: bool,
    /// Bytes from the app that the socket did not accept yet. `send_slice`
    /// can take part of a chunk when the send buffer is full.
    pub(super) pending: Vec<u8>,
}

pub(super) struct UdpConn {
    pub(super) handle: SocketHandle,
    pub(super) local: VirtAddr,
    pub(super) to_app: mpsc::Sender<(Vec<u8>, VirtAddr)>,
    pub(super) from_app: mpsc::Receiver<(Vec<u8>, VirtAddr)>,
}

/// The virtual resolver served at the gateway.
pub(super) struct DnsService {
    pub(super) handle: SocketHandle,
    pub(super) resolver: Arc<Resolver>,
}

pub(super) struct Listener {
    pub(super) addr: VirtAddr,
    pub(super) handles: Vec<SocketHandle>,
    pub(super) accept_tx: mpsc::Sender<TcpStream>,
}

pub(super) enum Command {
    Listen {
        addr: VirtAddr,
        reply: oneshot::Sender<io::Result<TcpListener>>,
    },
    Connect {
        remote: VirtAddr,
        local_port: u16,
        reply: oneshot::Sender<io::Result<TcpStream>>,
    },
    BindUdp {
        addr: VirtAddr,
        reply: oneshot::Sender<io::Result<UdpSocket>>,
    },
    Shutdown,
}

pub(super) struct Core {
    pub(super) iface: Interface,
    pub(super) device: LinkDevice,
    pub(super) sockets: SocketSet<'static>,
    /// Live and high-water socket counts, shared with the handle.
    ///
    /// A `SocketSet` only grows: `add` reuses a vacant slot but nothing reclaims
    /// one on its own, so a set that is not pruned on close grows without bound.
    /// The set itself lives inside the reactor task, so the count is what makes
    /// that observable from outside, and a test can assert on it instead of
    /// inferring a leak from a connection failing later.
    pub(super) socket_count: Arc<SocketCount>,
    pub(super) clock: Clock,
    pub(super) config: StackConfig,
    pub(super) listeners: Vec<Listener>,
    pub(super) conns: HashMap<u32, Conn>,
    pub(super) udp: HashMap<u32, UdpConn>,
    pub(super) next_id: u32,
    pub(super) events: mpsc::UnboundedSender<StackEvent>,
    pub(super) stats: Arc<NetStats>,
    pub(super) kick: Arc<Notify>,
    pub(super) dns: Option<DnsService>,
}

pub(super) fn new_tcp_socket(config: &StackConfig) -> tcp::Socket<'static> {
    let size = config.tcp_buffer;
    let mut socket = tcp::Socket::new(
        tcp::SocketBuffer::new(vec![0u8; size]),
        tcp::SocketBuffer::new(vec![0u8; size]),
    );
    // As before smoltcp 0.12: no congestion controller (issue 134).
    socket.set_congestion_control(tcp::CongestionControl::None);
    socket
}

pub(super) fn new_udp_socket(config: &StackConfig) -> udp::Socket<'static> {
    udp::Socket::new(
        udp::PacketBuffer::new(
            vec![udp::PacketMetadata::EMPTY; config.udp_packets],
            vec![0u8; config.udp_payload],
        ),
        udp::PacketBuffer::new(
            vec![udp::PacketMetadata::EMPTY; config.udp_packets],
            vec![0u8; config.udp_payload],
        ),
    )
}

impl Core {
    pub(super) fn new(
        config: StackConfig,
        mut device: LinkDevice,
        events: mpsc::UnboundedSender<StackEvent>,
        kick: Arc<Notify>,
        socket_count: Arc<SocketCount>,
    ) -> Self {
        let mut iface_config = IfaceConfig::new(HardwareAddress::Ip);
        iface_config.random_seed = config.random_seed;
        let mut iface = Interface::new(iface_config, &mut device, SmolInstant::ZERO);
        let gateway_v4 = config.subnet.gateway(crate::addr::Family::V4);
        let gateway_v6 = config.subnet.gateway(crate::addr::Family::V6);
        iface.update_ip_addrs(|addrs| {
            let prefix = config.subnet.v4_prefix;
            let _ = addrs.push(IpCidr::new(IpAddress::Ipv4(config.local_v4), prefix));
            let v6_prefix = config.subnet.v6_prefix;
            let _ = addrs.push(IpCidr::new(IpAddress::Ipv6(config.local_v6), v6_prefix));
            // The stack also answers for the gateway, where the virtual
            // resolver lives, so a client can reach `10.66.0.1:53`.
            if let IpAddr::V4(gateway) = gateway_v4 {
                if gateway != config.local_v4 {
                    let _ = addrs.push(IpCidr::new(IpAddress::Ipv4(gateway), prefix));
                }
            }
            if let IpAddr::V6(gateway) = gateway_v6 {
                if gateway != config.local_v6 {
                    let _ = addrs.push(IpCidr::new(IpAddress::Ipv6(gateway), v6_prefix));
                }
            }
        });
        let mut sockets: SocketSet<'static> = SocketSet::new(Vec::new());
        let dns = config.dns.clone().and_then(|resolver| {
            let handle = sockets.add(new_udp_socket(&config));
            let socket = sockets.get_mut::<udp::Socket>(handle);
            let listen = IpListenEndpoint {
                addr: Some(to_smoltcp(gateway_v4)),
                port: 53,
            };
            socket.bind(listen).ok()?;
            Some(DnsService { handle, resolver })
        });
        Self {
            iface,
            device,
            sockets,
            socket_count: socket_count.clone(),
            clock: Clock::starting_at(0),
            config,
            listeners: Vec::new(),
            conns: HashMap::new(),
            udp: HashMap::new(),
            next_id: 1,
            events,
            stats: Arc::new(NetStats::default()),
            kick,
            dns,
        }
    }

    pub(super) fn poll_iface(&mut self) {
        let now = self.clock.now();
        let _ = self.iface.poll(now, &mut self.device, &mut self.sockets);
    }

    pub(super) fn next_delay(&mut self) -> Option<Duration> {
        let now = self.clock.now();
        self.iface
            .poll_delay(now, &self.sockets)
            .map(|delay| Duration::from_millis(delay.total_millis()))
            .map(|delay| delay.min(self.config.max_poll_interval))
    }

    pub(super) fn tcp_socket(&self) -> tcp::Socket<'static> {
        new_tcp_socket(&self.config)
    }

    pub(super) fn udp_socket(&self) -> udp::Socket<'static> {
        new_udp_socket(&self.config)
    }

    pub(super) fn emit(&self, event: StackEvent) {
        let _ = self.events.send(event);
    }

    pub(super) fn acl_allows(&self, addr: &VirtAddr) -> bool {
        match &self.config.acl {
            Some(acl) => acl.allows(&addr.ip, addr.port),
            None => true,
        }
    }

    pub(super) fn in_subnet(&self, addr: &VirtAddr) -> bool {
        self.config.subnet.contains(&addr.ip)
    }

    pub(super) fn handle(&mut self, command: Command) {
        match command {
            Command::Listen { addr, reply } => {
                let _ = reply.send(self.listen(addr));
            }
            Command::Connect {
                remote,
                local_port,
                reply,
            } => {
                let _ = reply.send(self.connect(remote, local_port));
            }
            Command::BindUdp { addr, reply } => {
                let _ = reply.send(self.bind_udp(addr));
            }
            Command::Shutdown => {}
        }
    }

    pub(super) fn listen(&mut self, addr: VirtAddr) -> io::Result<TcpListener> {
        if !self.in_subnet(&addr) {
            return Err(io::Error::new(
                io::ErrorKind::AddrNotAvailable,
                format!("{addr} is outside the virtual subnet"),
            ));
        }
        if !self.acl_allows(&addr) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("ACL denies binding {addr}"),
            ));
        }
        if self.listeners.iter().any(|listener| listener.addr == addr) {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                format!("{addr} is already bound"),
            ));
        }
        // A listener on an address that the stack does not own yet: own it,
        // so that a SYN to it gets an answer. Before, a --forward to an
        // address other than the stack's own got no SYN-ACK, and each client
        // waited for its own time limit.
        let ip = to_smoltcp(addr.ip);
        if !self.iface.has_ip_addr(ip) && !ip.is_unspecified() {
            let prefix = match addr.ip {
                IpAddr::V4(_) => self.config.subnet.v4_prefix,
                IpAddr::V6(_) => self.config.subnet.v6_prefix,
            };
            let mut full = false;
            self.iface.update_ip_addrs(|addrs| {
                full = addrs.push(IpCidr::new(ip, prefix)).is_err();
            });
            if !full {
                self.device.add_local(addr.ip);
            }
            if full {
                return Err(io::Error::new(
                    io::ErrorKind::AddrNotAvailable,
                    format!(
                        "{addr}: the stack owns at most 8 addresses (its own, the gateways and the listeners)"
                    ),
                ));
            }
        }
        let (accept_tx, accept_rx) = mpsc::channel(self.config.accept_pool.max(1) * 4);
        let mut handles = Vec::with_capacity(self.config.accept_pool.max(1));
        for _ in 0..self.config.accept_pool.max(1) {
            handles.push(self.spawn_listener_socket(addr)?);
        }
        self.listeners.push(Listener {
            addr,
            handles,
            accept_tx,
        });
        self.emit(StackEvent::ListenerBound { addr });
        Ok(TcpListener {
            local: addr,
            rx: accept_rx,
        })
    }

    pub(super) fn spawn_listener_socket(&mut self, addr: VirtAddr) -> io::Result<SocketHandle> {
        let handle = self.sockets.add(self.tcp_socket());
        self.socket_count.add();
        let socket = self.sockets.get_mut::<tcp::Socket>(handle);
        socket
            .listen(IpListenEndpoint {
                addr: Some(to_smoltcp(addr.ip)),
                port: addr.port,
            })
            .map_err(|err| io::Error::new(io::ErrorKind::AddrInUse, err.to_string()))?;
        Ok(handle)
    }

    pub(super) fn connect(&mut self, remote: VirtAddr, local_port: u16) -> io::Result<TcpStream> {
        if !self.in_subnet(&remote) {
            self.stats.refused.fetch_add(1, Ordering::Relaxed);
            self.emit(StackEvent::Refused {
                remote,
                reason: "outside the virtual subnet".into(),
            });
            return Err(io::Error::new(
                io::ErrorKind::AddrNotAvailable,
                format!("{remote} is outside the virtual subnet"),
            ));
        }
        if !self.acl_allows(&remote) {
            self.stats.refused.fetch_add(1, Ordering::Relaxed);
            self.emit(StackEvent::Refused {
                remote,
                reason: "denied by ACL".into(),
            });
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("ACL denies connecting to {remote}"),
            ));
        }
        if self.conns.len() >= self.config.max_connections {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "virtual connection limit reached",
            ));
        }
        let local_port = if local_port == 0 {
            ephemeral_port(self.next_id, self.config.random_seed)
        } else {
            local_port
        };
        // The local address must share the remote's family: smoltcp will not
        // connect a v4 socket to a v6 peer (or the other way round).
        let local_ip = if remote.ip.is_ipv4() {
            IpAddr::V4(self.config.local_v4)
        } else {
            IpAddr::V6(self.config.local_v6)
        };
        let local = VirtAddr::new(local_ip, local_port);
        let handle = self.sockets.add(self.tcp_socket());
        self.socket_count.add();
        {
            let socket = self.sockets.get_mut::<tcp::Socket>(handle);
            socket
                .connect(
                    self.iface.context(),
                    IpEndpoint::new(to_smoltcp(remote.ip), remote.port),
                    IpListenEndpoint {
                        addr: Some(to_smoltcp(local.ip)),
                        port: local.port,
                    },
                )
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err.to_string()))?;
        }
        let id = self.next_id;
        self.next_id += 1;
        let (to_app_tx, to_app_rx) = mpsc::channel(64);
        let (from_app_tx, from_app_rx) = mpsc::channel(64);
        let (ctl_tx, ctl_rx) = mpsc::channel(16);
        let write_waker = Arc::new(AtomicWaker::new());
        self.conns.insert(
            id,
            Conn {
                handle,
                to_app: to_app_tx,
                from_app: from_app_rx,
                write_waker: write_waker.clone(),
                ctl: ctl_rx,
                closing: false,
                recv_eof_sent: false,
                announced: false,
                pending: Vec::new(),
            },
        );
        self.emit(StackEvent::Connected { id, local, remote });
        self.stats.connected.fetch_add(1, Ordering::Relaxed);
        Ok(TcpStream {
            id,
            local,
            remote,
            read_rx: to_app_rx,
            write_tx: from_app_tx,
            write_waker,
            ctl_tx,
            kick: self.kick.clone(),
            read_buf: Vec::new(),
            read_pos: 0,
            eof: false,
            nodelay: false,
        })
    }

    pub(super) fn bind_udp(&mut self, addr: VirtAddr) -> io::Result<UdpSocket> {
        if !self.in_subnet(&addr) {
            return Err(io::Error::new(
                io::ErrorKind::AddrNotAvailable,
                format!("{addr} is outside the virtual subnet"),
            ));
        }
        if !self.acl_allows(&addr) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("ACL denies binding {addr}"),
            ));
        }
        if self.udp.values().any(|conn| conn.local == addr) {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                format!("{addr} is already bound"),
            ));
        }
        let handle = self.sockets.add(self.udp_socket());
        self.socket_count.add();
        {
            let socket = self.sockets.get_mut::<udp::Socket>(handle);
            socket
                .bind(IpListenEndpoint {
                    addr: Some(to_smoltcp(addr.ip)),
                    port: addr.port,
                })
                .map_err(|err| io::Error::new(io::ErrorKind::AddrInUse, err.to_string()))?;
        }
        let id = self.next_id;
        self.next_id += 1;
        let (to_app_tx, to_app_rx) = mpsc::channel(64);
        let (send_tx, send_rx) = mpsc::channel(64);
        self.udp.insert(
            id,
            UdpConn {
                handle,
                local: addr,
                to_app: to_app_tx,
                from_app: send_rx,
            },
        );
        Ok(UdpSocket {
            id,
            local: addr,
            recv_rx: to_app_rx,
            send_tx,
            kick: self.kick.clone(),
        })
    }
}

pub(super) fn ephemeral_port(id: u32, seed: u64) -> u16 {
    // 49152..=65535 is the dynamic range; mix the id and seed so restarts do
    // not collide.
    let mixed = (id as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ seed;
    49152 + (mixed % (65535 - 49152 + 1)) as u16
}
