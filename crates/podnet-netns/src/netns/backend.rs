//! The router of the netns backend: the hold of `pod_hold::syn_hold` on the
//! wire (the TUN or the veth pair), the upstream dials and the flows.
//!
//! The pump of the wire ([`Tun`]) reads each packet of the program first.
//! It answers the DNS queries and takes the UDP flows. The hold of each SYN
//! (`pod_hold::syn_hold`, PodUtils/podcrates) holds each new SYN while its
//! upstream is dialed (issue 190), and gives the rest to the link of its
//! stack of `pod-netstack`. The stack answers for each destination address
//! (`any_ip`), and a listener on any address of the destination port accepts
//! each connection. The `local_addr` of the accepted stream is the target of
//! the flow.

#[allow(unused_imports)]
use crate::prelude::*;
use pod_hold::syn_hold::{Dial, Refused, Settings, Syn, SynHold};
use pod_netstack::smoltcp::phy::{Checksum, ChecksumCapabilities};
use pod_netstack::{StackConfig, Subnet, TcpStream};
use std::sync::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt, ReadHalf, WriteHalf};

/// The dial of an upstream, before its task.
type DialFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Result<Upstream>> + Send>>;

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

/// The records of the flows of the program, by their addresses, until they
/// end.
type OpenFlows = Arc<Mutex<HashMap<Syn, FlowRecord>>>;

/// What the dial of a flow gives: the upstream, the target and the number of
/// the flow in the record of `--flow-log` (T-157).
pub(crate) struct Dialed {
    upstream: Upstream,
    target: String,
    flow: Option<u64>,
}

pub(crate) struct Backend {
    pub(crate) device: Tun,
    /// Issue 190: the hold of each new SYN while its upstream is dialed
    /// (pod-hold). The stack sees the SYN only when the dial succeeds; a
    /// failed dial gets a RST, so `connect()` of the program fails.
    hold: SynHold<Dialed, anyhow::Error>,
    /// The records of the flows of the program, for their close lines.
    open: OpenFlows,
    pub(crate) dns: FakeDns,
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
        // destination address (the hold turns `any_ip` on): the guest
        // connects to real addresses, and the stack impersonates all of them.
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
            checksum,
            ..StackConfig::default()
        };
        let settings = Settings {
            max_flows: MAX_TRACKED_FDS,
            ..Settings::default()
        };
        let mut dns = FakeDns::new();
        for (name, ip) in &maps {
            dns.pin(name, *ip);
        }
        Ok(Self {
            device,
            hold: SynHold::new(config, settings),
            open: Arc::new(Mutex::new(HashMap::new())),
            dns,
            udp: UdpFlows::new(),
            host_gateway: false,
            proxies,
            rules,
            verbose,
        })
    }

    /// Wait until the stack has a packet for the wire, or the hold has a
    /// stream, the end of a dial or the end of a flow.
    pub(crate) async fn activity(&self) {
        self.hold.activity().await
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
            host_gateway: self.host_gateway,
            verbose: self.verbose,
        };
        self.udp
            .relay(&mut self.device.rx, &mut self.device.tx, &route);
        let tcp = TcpRoute {
            proxies: &self.proxies,
            rules: &self.rules,
            dns: &self.dns,
            host_gateway: self.host_gateway,
            verbose: self.verbose,
        };
        self.hold
            .pump(&mut self.device.rx, &mut self.device.tx, &tcp)
            .await;
        self.accept_flows();
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

    /// Start a flow for each connection that the hold accepted with its
    /// upstream. A flow is a task, so a slow proxy does not stop the other
    /// flows.
    pub(crate) fn accept_flows(&mut self) {
        while let Some(accepted) = self.hold.next_flow() {
            let pod_hold::syn_hold::Flow {
                syn,
                stream,
                upstream:
                    Dialed {
                        upstream,
                        target,
                        flow,
                    },
                open: entry,
            } = accepted;
            let record = FlowRecord {
                target,
                flow,
                counts: Default::default(),
            };
            let flow = Flow {
                stream,
                upstream,
                record,
                open: self.open.clone(),
                key: syn,
                entry,
            };
            tokio::spawn(flow.run());
        }
    }
}

/// The dial of the TCP flows of the program, for the hold of pod-hold: the
/// rules, the proxies, the fake names and `--host-gateway`.
struct TcpRoute<'a> {
    proxies: &'a [Proxy],
    rules: &'a [Rule],
    dns: &'a FakeDns,
    host_gateway: bool,
    verbose: bool,
}

impl TcpRoute<'_> {
    /// The target of a connection to `dst:port`: the name of a fake address,
    /// else the address.
    fn target_of(&self, dst: Ipv4Addr, port: u16) -> String {
        match self.dns.name_for(dst) {
            Some(name) => format!("{name}:{port}"),
            None => format!("{dst}:{port}"),
        }
    }

    /// The dial of the upstream of `target`, and the number of the flow in
    /// the record.
    fn spawn_dial(&self, target: &str) -> (DialFuture, Option<u64>) {
        let picked = crate::learn::select(self.rules, self.proxies, target, "tcp");
        let flow = crate::flowlog::decided("tcp", target, &picked);
        let Some(chain) = picked.chain().map(<[Proxy]>::to_vec) else {
            // T-156: the dial fails at once, and the program gets a RST.
            crate::rules::note_refusal("connect", target, picked.rule_name(), self.verbose);
            return (Box::pin(async { bail!("a rule refuses the target") }), flow);
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
        let dial = async move {
            within_timeout(
                "upstream connect",
                connect_upstream_async(&chain, &dial_target),
            )
            .await
        };
        (Box::pin(dial), flow)
    }

    /// The dial of a connection to the gateway on `port` (T-112): TCP to
    /// 127.0.0.1 and `port` on the host with `--host-gateway`, else a
    /// refusal (the program gets a RST).
    fn spawn_gateway_dial(&self, port: u16) -> DialFuture {
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
        Box::pin(async move {
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
}

impl Dial for TcpRoute<'_> {
    type Upstream = Dialed;
    type Error = anyhow::Error;

    fn dial(&self, syn: Syn) -> impl std::future::Future<Output = Result<Dialed>> + Send + 'static {
        let (dst, port) = (*syn.target.ip(), syn.target.port());
        let target = self.target_of(dst, port);
        let (dial, flow) = match dst == TUN_GW {
            true => (self.spawn_gateway_dial(port), None),
            false => self.spawn_dial(&target),
        };
        async move {
            let upstream = dial.await?;
            Ok(Dialed {
                upstream,
                target,
                flow,
            })
        }
    }

    fn refused(&self, syn: Syn, refused: Refused, e: anyhow::Error) {
        if !self.verbose {
            return;
        }
        let target = self.target_of(*syn.target.ip(), syn.target.port());
        let tail = match refused {
            Refused::Reset => "; the connect of the program is refused",
            Refused::Aborted => "",
        };
        eprintln!(
            "{}: upstream for {target} failed: {e:#}{tail}",
            crate::cli::program_name()
        );
    }
}

/// One accepted connection of the program and its upstream.
struct Flow {
    stream: TcpStream,
    upstream: Upstream,
    record: FlowRecord,
    open: OpenFlows,
    key: Syn,
    /// The entry of the flow in the hold: its drop ends the flow there.
    entry: pod_hold::syn_hold::Open,
}

impl Flow {
    /// Copy between the stream of the program and the upstream, until the
    /// upstream ends or one side fails. When the program closes its side,
    /// the upstream gets the end too, and the flow goes on until the
    /// upstream ends.
    async fn run(self) {
        let Flow {
            stream,
            upstream,
            record,
            open,
            key,
            entry,
        } = self;
        let counts = record.counts.clone();
        if let Ok(mut open) = open.lock() {
            open.insert(key, record);
        }
        let (mut app_read, mut app_write) = tokio::io::split(stream);
        relay(&mut app_read, &mut app_write, upstream, counts).await;
        // The flow is over: write its close line.
        let ended = open.lock().ok().and_then(|mut open| open.remove(&key));
        if let Some(record) = ended {
            record.close();
        }
        // The drop of the stream closes the connection with no wait: the
        // stack sends each queued byte, then the FIN. Then the hold can
        // close the listener of the port.
        drop(app_read);
        drop(app_write);
        drop(entry);
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
    /// The hold stops its listeners and its dials.
    fn drop(&mut self) {
        if let Ok(mut open) = self.open.lock() {
            for (_, record) in open.drain() {
                record.close();
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
