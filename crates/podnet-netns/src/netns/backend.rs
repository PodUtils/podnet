//! The router of the netns backend: the stack of `pod-netstack` on the wire
//! (the TUN or the veth pair), the held SYNs, the upstream dials and the
//! flows.
//!
//! The pump of the wire ([`Tun`]) reads each packet of the program first.
//! It answers the DNS queries and takes the UDP flows, holds each new SYN
//! while its upstream is dialed (issue 190), and gives the rest to the link
//! of the stack. The stack answers for each destination address (`any_ip`),
//! and a listener on any address of the destination port accepts each
//! connection. The `local_addr` of the accepted stream is the target of the
//! flow.

#[allow(unused_imports)]
use crate::prelude::*;
use pod_netstack::smoltcp::phy::{Checksum, ChecksumCapabilities};
use pod_netstack::{NetStack, PacketQueue, StackConfig, Subnet, TcpListener, TcpStream};
use std::collections::HashSet;
use std::sync::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt, ReadHalf, WriteHalf};
use tokio::sync::{mpsc, Notify};

/// The addresses of a TCP connection of the program: (source, source port,
/// destination, destination port).
pub(crate) type SynKey = (Ipv4Addr, u16, Ipv4Addr, u16);

/// A SYN of the program and the dial of its upstream.
pub(crate) struct HeldSyn {
    pub(crate) pkt: Vec<u8>,
    pub(crate) target: String,
    pub(crate) task: tokio::task::JoinHandle<Result<Upstream>>,
    /// The number of the flow in the record of `--flow-log` (T-157).
    pub(crate) flow: Option<u64>,
}

/// The dial of an upstream.
pub(crate) type DialTask = tokio::task::JoinHandle<Result<Upstream>>;

/// The time that the upstream of a released SYN waits for its socket.
const PREDIALED_TTL: std::time::Duration = std::time::Duration::from_secs(30);

/// The time that a released SYN waits for its stream. A handshake that never
/// ends frees its entry after this time.
const HANDSHAKE_TTL: std::time::Duration = std::time::Duration::from_secs(10);

/// The record of an open flow (T-157): its target, its number in the record
/// of `--flow-log` and its byte counts.
struct FlowRecord {
    target: String,
    flow: Option<u64>,
    counts: crate::flowlog::Counts,
}

impl FlowRecord {
    fn close(&self) {
        self.counts.close(self.flow, "tcp", &self.target);
    }
}

/// The connections of the program that the stack accepted, by their
/// addresses. An entry has no record while its upstream is dialed.
type OpenFlows = Arc<Mutex<HashMap<SynKey, Option<FlowRecord>>>>;

/// A stream that the stack accepted.
type Accepted = TcpStream;

/// Where the upstream of an accepted connection comes from: dialed before
/// the handshake (the usual case), or by a dial that runs now.
enum Source {
    Ready(Upstream),
    Dial(DialTask),
}

pub(crate) struct Backend {
    pub(crate) device: Tun,
    stack: NetStack,
    /// The packets of the program for the stack, and from it.
    link_in: PacketQueue,
    link_out: PacketQueue,
    /// The destination ports with a listener, and the task of each that
    /// takes its accepted streams.
    ports: HashMap<u16, tokio::task::JoinHandle<()>>,
    accepted_tx: mpsc::UnboundedSender<Accepted>,
    accepted_rx: mpsc::UnboundedReceiver<Accepted>,
    /// Wakes the loop when the stack accepts a stream.
    accepted: Arc<Notify>,
    /// The SYNs that the stack has, until their stream is accepted.
    handshakes: HashMap<SynKey, std::time::Instant>,
    /// The flows of the program, until they end.
    open: OpenFlows,
    pub(crate) dns: FakeDns,
    /// Issue 190: each new SYN waits here while its upstream is dialed. The
    /// stack sees the SYN only when the dial succeeds; a failed dial gets a
    /// RST, so `connect()` of the program fails.
    pub(crate) held: HashMap<SynKey, HeldSyn>,
    /// The upstreams of released SYNs, until their stream is accepted.
    pub(crate) predialed: HashMap<SynKey, (String, Upstream, std::time::Instant, Option<u64>)>,
    /// The UDP flows of the program other than DNS (T-111).
    pub(crate) udp: UdpFlows,
    /// `--host-gateway` (T-112): a flow to the gateway goes to the loopback
    /// of the host. Off: a TCP connect to the gateway gets a RST, and a
    /// datagram to it (not DNS) is dropped.
    pub(crate) host_gateway: bool,
    pub(crate) proxies: Vec<Proxy>,
    pub(crate) rules: Vec<Rule>,
    pub(crate) verbose: bool,
}

impl Backend {
    /// The backend on `fd`. Call it inside a runtime of tokio: the stack
    /// spawns its reactor.
    pub(crate) fn new(
        fd: RawFd,
        proxies: Vec<Proxy>,
        rules: Vec<Rule>,
        maps: Vec<(String, IpAddr)>,
        verbose: bool,
    ) -> Result<Self> {
        let device = Tun::new(fd, TUN_MTU);
        let mut checksum = ChecksumCapabilities::default();
        if !device.wire.checks_checksums() {
            // The program end can send a partial checksum (an offload that
            // stays on): compute the checksums, do not check them.
            checksum.ipv4 = Checksum::Tx;
            checksum.tcp = Checksum::Tx;
            checksum.udp = Checksum::Tx;
            checksum.icmpv4 = Checksum::Tx;
        }
        // The stack owns the gateway of the TUN and answers for each
        // destination address: the guest connects to real addresses, and
        // the stack impersonates all of them.
        let config = StackConfig {
            subnet: Subnet {
                v4: TUN_GW,
                v4_prefix: TUN_PREFIX,
                ..Subnet::default()
            },
            local_v4: TUN_GW,
            mtu: TUN_MTU,
            tcp_buffer: TCP_BUF,
            random_seed: seed(),
            any_ip: true,
            checksum,
            ..StackConfig::default()
        };
        let stack = NetStack::over_link(config);
        let (accepted_tx, accepted_rx) = mpsc::unbounded_channel();
        let mut dns = FakeDns::new();
        for (name, ip) in &maps {
            dns.pin(name, *ip);
        }
        Ok(Self {
            device,
            link_in: stack.link_in(),
            link_out: stack.link_out(),
            stack,
            ports: HashMap::new(),
            accepted_tx,
            accepted_rx,
            accepted: Arc::new(Notify::new()),
            handshakes: HashMap::new(),
            open: Arc::new(Mutex::new(HashMap::new())),
            dns,
            held: HashMap::new(),
            predialed: HashMap::new(),
            udp: UdpFlows::new(),
            host_gateway: false,
            proxies,
            rules,
            verbose,
        })
    }

    /// Wait until the stack has a packet for the wire or a stream for the
    /// backend.
    pub(crate) async fn activity(&self) {
        tokio::select! {
            _ = self.link_out.wait() => {}
            _ = self.accepted.notified() => {}
        }
    }

    /// The target of a connection to `dst:port`: the name of a fake address,
    /// else the address.
    fn target_of(&self, dst: Ipv4Addr, port: u16) -> String {
        match self.dns.name_for(dst) {
            Some(name) => format!("{name}:{port}"),
            None => format!("{dst}:{port}"),
        }
    }

    /// Start the dial of the upstream of `target`. Give the number of the
    /// flow in the record too.
    fn spawn_dial(&self, target: &str) -> (DialTask, Option<u64>) {
        let picked = crate::learn::select(&self.rules, &self.proxies, target, "tcp");
        let flow = crate::flowlog::decided("tcp", target, &picked);
        let Some(chain) = picked.chain().map(<[Proxy]>::to_vec) else {
            // T-156: the dial fails at once, and the program gets a RST.
            crate::rules::note_refusal("connect", target, picked.rule_name(), self.verbose);
            return (
                tokio::spawn(async { bail!("a rule refuses the target") }),
                flow,
            );
        };
        if self.verbose {
            let hops: Vec<String> = chain.iter().map(Proxy::endpoint).collect();
            eprintln!(
                "{}: {target} via {}",
                crate::cli::program_name(),
                hops.join(" -> ")
            );
        }
        let dial_target = target.to_string();
        let task = tokio::spawn(async move {
            within_timeout(
                "upstream connect",
                connect_upstream_async(&chain, &dial_target),
            )
            .await
        });
        (task, flow)
    }

    /// The dial of a connection to the gateway on `port` (T-112): TCP to
    /// 127.0.0.1 and `port` on the host with `--host-gateway`, else a
    /// refusal (the program gets a RST).
    fn spawn_gateway_dial(&self, port: u16) -> DialTask {
        let on = self.host_gateway;
        if self.verbose {
            let what = match on {
                true => format!("127.0.0.1:{port} of the host (--host-gateway)"),
                false => "a refusal (no --host-gateway)".to_string(),
            };
            eprintln!(
                "{}: {TUN_GW}:{port} (the gateway) goes to {what}",
                crate::cli::program_name()
            );
        }
        tokio::spawn(async move {
            if !on {
                bail!("the gateway refuses the connect (no --host-gateway)");
            }
            let target = format!("127.0.0.1:{port}");
            let stream = within_timeout(
                "host gateway connect",
                crate::upstream::connect_direct(&target),
            )
            .await?;
            Ok(Box::new(stream) as Upstream)
        })
    }

    /// A connection of the program has the addresses of `key`.
    fn has_connection(&self, key: &SynKey) -> bool {
        self.open.lock().is_ok_and(|open| open.contains_key(key))
    }

    /// The flows of the program now.
    fn open_count(&self) -> usize {
        self.open.lock().map_or(0, |open| open.len())
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
            } else if self.held.len() + self.open_count() >= MAX_TRACKED_FDS {
                // The table is full: the stack drops the SYN.
                kept.push_back(pkt);
            } else {
                let target = self.target_of(key.2, key.3);
                let (task, flow) = match key.2 == TUN_GW {
                    true => (self.spawn_gateway_dial(key.3), None),
                    false => self.spawn_dial(&target),
                };
                let held = HeldSyn {
                    pkt,
                    target,
                    task,
                    flow,
                };
                self.held.insert(key, held);
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
                    let at = std::time::Instant::now();
                    let dialed = (held.target, upstream, at, held.flow);
                    self.predialed.insert(key, dialed);
                    self.device.rx.push_front(held.pkt);
                }
                Err(e) => {
                    if self.verbose {
                        eprintln!(
                            "{}: upstream for {} failed: {e:#}; the connect of the program is refused", crate::cli::program_name(),
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
            .retain(|_, (_, _, at, _)| at.elapsed() < PREDIALED_TTL);
        self.handshakes.retain(|_, at| at.elapsed() < HANDSHAKE_TTL);
    }

    /// Give each packet of the program to the stack. A new SYN needs a
    /// listener on its port. The stack adds a listen socket for each SYN, so
    /// a burst to one port gets no RST.
    pub(crate) async fn feed(&mut self) {
        while let Some(pkt) = self.device.rx.pop_front() {
            if let Some(key) = parse_tcp_syn(&pkt) {
                let known = self.handshakes.contains_key(&key) || self.has_connection(&key);
                if !known {
                    if !self.listen_on(key.3).await {
                        // The stack drops the SYN.
                        continue;
                    }
                    self.handshakes.insert(key, std::time::Instant::now());
                }
            }
            // A full queue drops the packet: TCP sends it again.
            self.link_in.push(pkt);
        }
    }

    /// Open a listener on any address of `port`, when there is none. False
    /// when the stack refuses.
    async fn listen_on(&mut self, port: u16) -> bool {
        if self.ports.contains_key(&port) {
            return true;
        }
        let mut listener: TcpListener = match self.stack.listen_any(port).await {
            Ok(listener) => listener,
            Err(e) => {
                if self.verbose {
                    eprintln!(
                        "{}: no listener for outbound port {port}: {e}",
                        crate::cli::program_name()
                    );
                }
                return false;
            }
        };
        let tx = self.accepted_tx.clone();
        let wake = self.accepted.clone();
        let task = tokio::spawn(async move {
            while let Ok(stream) = listener.accept().await {
                if tx.send(stream).is_err() {
                    break;
                }
                wake.notify_one();
            }
        });
        self.ports.insert(port, task);
        if self.verbose {
            eprintln!(
                "{}: listening for outbound port {port}",
                crate::cli::program_name()
            );
        }
        true
    }

    /// Close the listener of each port that has no SYN in a handshake and no
    /// flow. The close frees the port in the stack, so the table does not
    /// grow with the ports that the program used.
    fn close_idle_ports(&mut self) {
        if self.ports.is_empty() {
            return;
        }
        let mut busy: HashSet<u16> = self.handshakes.keys().map(|key| key.3).collect();
        if let Ok(open) = self.open.lock() {
            busy.extend(open.keys().map(|key| key.3));
        }
        self.ports.retain(|port, task| {
            let keep = busy.contains(port);
            if !keep {
                task.abort();
            }
            keep
        });
    }

    /// Take the packets that the stack sends to the program.
    pub(crate) fn collect(&mut self) {
        while let Some(pkt) = self.link_out.pop() {
            self.device.tx.push_back(pkt);
        }
    }

    /// Read the TUN, unless a full UDP queue pauses the read.
    pub(crate) fn read_tun(&mut self) {
        if !self.udp.paused() {
            self.device.read_ready();
        }
    }

    pub(crate) async fn tick(&mut self) -> Result<()> {
        self.accept_flows();
        self.answer_dns();
        let route = UdpRoute {
            proxies: &self.proxies,
            rules: &self.rules,
            dns: &self.dns,
            host_gateway: self.host_gateway,
            verbose: self.verbose,
        };
        self.udp
            .relay(&mut self.device.rx, &mut self.device.tx, &route);
        self.hold_syns().await;
        self.feed().await;
        self.close_idle_ports();
        self.collect();
        self.device.flush();
        Ok(())
    }

    /// Answer each IPv4 UDP query to port 53 from the fake-IP pool, before
    /// the stack sees it.
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

    /// Start a flow for each stream that the stack accepted. The upstream
    /// was dialed before the handshake (the usual case), else a dial starts
    /// now. A flow is a task, so a slow proxy does not stop the other flows.
    pub(crate) fn accept_flows(&mut self) {
        while let Ok(stream) = self.accepted_rx.try_recv() {
            let (IpAddr::V4(src_ip), IpAddr::V4(dst_ip)) =
                (stream.peer_addr().ip(), stream.local_addr().ip())
            else {
                stream.abort();
                continue;
            };
            let key = (
                src_ip,
                stream.peer_addr().port(),
                dst_ip,
                stream.local_addr().port(),
            );
            self.handshakes.remove(&key);
            let (target, source, flow) = match self.predialed.remove(&key) {
                Some((target, upstream, _, flow)) => (target, Source::Ready(upstream), flow),
                None => {
                    let target = self.target_of(dst_ip, key.3);
                    let (task, flow) = self.spawn_dial(&target);
                    (target, Source::Dial(task), flow)
                }
            };
            if let Ok(mut open) = self.open.lock() {
                open.insert(key, None);
            }
            let record = FlowRecord {
                target,
                flow,
                counts: Default::default(),
            };
            let open = self.open.clone();
            let flow = Flow {
                stream,
                source,
                record,
                open,
                key,
                verbose: self.verbose,
            };
            tokio::spawn(flow.run());
        }
    }
}

/// One accepted connection of the program and its upstream.
struct Flow {
    stream: TcpStream,
    source: Source,
    record: FlowRecord,
    open: OpenFlows,
    key: SynKey,
    verbose: bool,
}

impl Flow {
    /// Copy between the stream of the program and the upstream, until the
    /// upstream ends or one side fails. When the program closes its side,
    /// the upstream gets the end too, and the flow goes on until the
    /// upstream ends.
    async fn run(self) {
        let Flow {
            stream,
            source,
            record,
            open,
            key,
            verbose,
        } = self;
        let upstream = match source {
            Source::Ready(upstream) => Some(upstream),
            Source::Dial(task) => match task.await {
                Ok(Ok(upstream)) => Some(upstream),
                Ok(Err(e)) => {
                    if verbose {
                        eprintln!(
                            "{}: upstream for {} failed: {e:#}",
                            crate::cli::program_name(),
                            record.target
                        );
                    }
                    None
                }
                Err(e) => {
                    if verbose {
                        eprintln!(
                            "{}: dial task for {} failed: {e}",
                            crate::cli::program_name(),
                            record.target
                        );
                    }
                    None
                }
            },
        };
        let Some(upstream) = upstream else {
            stream.abort();
            if let Ok(mut open) = open.lock() {
                open.remove(&key);
            }
            return;
        };
        let counts = record.counts.clone();
        if let Ok(mut open) = open.lock() {
            open.insert(key, Some(record));
        }
        let (mut app_read, mut app_write) = tokio::io::split(stream);
        relay(&mut app_read, &mut app_write, upstream, counts).await;
        // The flow is over: write its close line.
        let ended = open
            .lock()
            .ok()
            .and_then(|mut open| open.get_mut(&key).and_then(Option::take));
        if let Some(record) = ended {
            record.close();
        }
        // The drop of the stream closes the connection with no wait: the
        // stack sends each queued byte, then the FIN.
        drop(app_read);
        drop(app_write);
        let mut flows = open.lock().unwrap_or_else(|e| e.into_inner());
        flows.remove(&key);
    }
}

/// Copy the bytes of the flow in both directions, until the upstream ends or
/// one side fails. When the program closes its side, the upstream gets the
/// end too, and the flow goes on until the upstream ends.
async fn relay(
    app_read: &mut ReadHalf<TcpStream>,
    app_write: &mut WriteHalf<TcpStream>,
    upstream: Upstream,
    counts: crate::flowlog::Counts,
) {
    let (mut up_read, mut up_write) = tokio::io::split(upstream);
    let out = counts.clone();
    let to_upstream = async {
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            let n = app_read.read(&mut buf).await?;
            if n == 0 {
                // The program closed its side.
                return up_write.shutdown().await;
            }
            up_write.write_all(&buf[..n]).await?;
            out.add_out(n);
        }
    };
    let to_app = async {
        let mut buf = vec![0u8; 16 * 1024];
        let result: std::io::Result<()> = loop {
            match up_read.read(&mut buf).await {
                Ok(0) => break Ok(()),
                Ok(n) => {
                    counts.add_in(n);
                    if let Err(e) = app_write.write_all(&buf[..n]).await {
                        break Err(e);
                    }
                }
                Err(e) => break Err(e),
            }
        };
        // The end of the upstream, or a failure: close the side of the
        // program after the bytes that it has.
        let _ = app_write.shutdown().await;
        result
    };
    let mut to_upstream = std::pin::pin!(to_upstream);
    let mut to_app = std::pin::pin!(to_app);
    tokio::select! {
        _ = &mut to_app => {}
        result = &mut to_upstream => {
            if result.is_ok() {
                let _ = to_app.await;
            }
        }
    }
}

impl Drop for Backend {
    /// The flows that are open at the end of the pod get their close line.
    fn drop(&mut self) {
        for task in self.ports.values() {
            task.abort();
        }
        if let Ok(mut open) = self.open.lock() {
            for (_, record) in open.drain() {
                if let Some(record) = record {
                    record.close();
                }
            }
        }
    }
}

pub(crate) fn seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9e37_79b9_7f4a_7c15)
}
