//! Generic UDP of the netns backend (T-111).
//!
//! The stack takes each IPv4 UDP datagram of the program from the TUN
//! before `smoltcp`, as [`super::Backend::answer_dns`] does. A table keeps
//! one flow for each (source, source port, destination, destination port):
//! two flows to one port stay apart. A flow is a task that carries its
//! datagrams to the first proxy of its chain: in the stream of the proxy
//! (SOCKS5 command 0x83) for a proxy on a unix socket, else with
//! `UDP ASSOCIATE`; a `direct` rule sends them from a UDP socket of the
//! host. A fake address of the pod goes to the proxy as its name.
//!
//! Each reply becomes an IPv4 packet with the two addresses of the flow
//! swapped, so the program sees the reply from the address that it sent
//! to.
//!
//! Each flow has a bounded queue (GitHub #372). When the queue is full, the
//! datagram waits in the stack, and the stack stops the read of the TUN
//! until the queue has room: the kernel of the pod then counts the loss.
//! A pause does not end the flow. The frame code writes each frame in
//! full (`pod_proxy::udp::write_frame`), so a partial write of the stream
//! does not cut a frame.

#[allow(unused_imports)]
use crate::prelude::*;
use std::time::Instant;

/// A flow: (source, source port, destination, destination port).
pub(crate) type UdpKey = (Ipv4Addr, u16, Ipv4Addr, u16);

/// A flow with no datagram in either direction for this time ends.
pub(crate) const UDP_IDLE: Duration = Duration::from_secs(60);
/// The most flows. A new flow above it ends the flow that was idle longest.
pub(crate) const MAX_UDP_FLOWS: usize = 256;
/// The datagrams that wait for one flow.
pub(crate) const UDP_QUEUE: usize = 64;
/// The replies that wait for the TUN, for all flows.
const UDP_REPLIES: usize = 1024;

/// The flows by key, with the time of the last datagram of each.
pub(crate) struct FlowTable<T> {
    entries: HashMap<UdpKey, (T, Instant)>,
    idle: Duration,
    cap: usize,
}

impl<T> FlowTable<T> {
    pub(crate) fn new(idle: Duration, cap: usize) -> Self {
        Self {
            entries: HashMap::new(),
            idle,
            cap,
        }
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    /// The flow of `key`, marked as used at `now`.
    pub(crate) fn touch(&mut self, key: &UdpKey, now: Instant) -> Option<&mut T> {
        let (value, at) = self.entries.get_mut(key)?;
        *at = now;
        Some(value)
    }

    /// Add a flow. At the cap, remove and return the flow that was idle
    /// longest.
    pub(crate) fn insert(&mut self, key: UdpKey, value: T, now: Instant) -> Option<T> {
        let mut evicted = None;
        if self.entries.len() >= self.cap && !self.entries.contains_key(&key) {
            let oldest = self.entries.iter().min_by_key(|(_, (_, at))| *at);
            if let Some(oldest) = oldest.map(|(k, _)| *k) {
                evicted = self.entries.remove(&oldest).map(|(v, _)| v);
            }
        }
        self.entries.insert(key, (value, now));
        evicted
    }

    pub(crate) fn remove(&mut self, key: &UdpKey) -> Option<T> {
        self.entries.remove(key).map(|(v, _)| v)
    }

    /// Remove and return each flow that was idle for the idle time at `now`.
    pub(crate) fn reap(&mut self, now: Instant) -> Vec<T> {
        let idle = self.idle;
        let old: Vec<UdpKey> = self
            .entries
            .iter()
            .filter(|(_, (_, at))| now.saturating_duration_since(*at) >= idle)
            .map(|(k, _)| *k)
            .collect();
        old.into_iter().filter_map(|k| self.remove(&k)).collect()
    }
}

/// One flow: its queue and its task. The drop of the flow stops the task,
/// and the task closes its upstream.
pub(crate) struct UdpFlow {
    queue: tokio::sync::mpsc::Sender<Vec<u8>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for UdpFlow {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// The UDP side of the stack.
pub(crate) struct UdpFlows {
    pub(crate) table: FlowTable<UdpFlow>,
    replies_tx: tokio::sync::mpsc::Sender<(UdpKey, Vec<u8>)>,
    replies: tokio::sync::mpsc::Receiver<(UdpKey, Vec<u8>)>,
    /// The datagrams that wait for a full queue, in their order.
    held: VecDeque<(UdpKey, Vec<u8>)>,
}

impl UdpFlows {
    pub(crate) fn new() -> Self {
        let (replies_tx, replies) = tokio::sync::mpsc::channel(UDP_REPLIES);
        Self {
            table: FlowTable::new(UDP_IDLE, MAX_UDP_FLOWS),
            replies_tx,
            replies,
            held: VecDeque::new(),
        }
    }

    /// True when a datagram waits for a full queue: the stack does not read
    /// the TUN.
    pub(crate) fn paused(&self) -> bool {
        !self.held.is_empty()
    }
}

/// The parts of the stack that a new flow needs.
pub(crate) struct UdpRoute<'a> {
    pub(crate) proxies: &'a [Proxy],
    pub(crate) rules: &'a [Rule],
    pub(crate) dns: &'a FakeDns,
    pub(crate) verbose: bool,
}

/// The flow and the payload of a UDP datagram of the program: IPv4, not a
/// fragment, not to the gateway, not to a broadcast or multicast address.
pub(crate) fn parse_udp(pkt: &[u8]) -> Option<(UdpKey, &[u8])> {
    if pkt.len() < 28 || pkt[0] >> 4 != 4 || pkt[9] != libc::IPPROTO_UDP as u8 {
        return None;
    }
    let ihl = usize::from(pkt[0] & 0x0f) * 4;
    let fragment = u16::from_be_bytes([pkt[6], pkt[7]]) & 0x3fff;
    if ihl < 20 || fragment != 0 || pkt.len() < ihl + 8 {
        return None;
    }
    let src = Ipv4Addr::new(pkt[12], pkt[13], pkt[14], pkt[15]);
    let dst = Ipv4Addr::new(pkt[16], pkt[17], pkt[18], pkt[19]);
    if dst == TUN_GW || dst.is_broadcast() || dst.is_multicast() || dst.is_loopback() {
        return None;
    }
    let udp = &pkt[ihl..];
    let len = usize::from(u16::from_be_bytes([udp[4], udp[5]]));
    if len < 8 || len > udp.len() {
        return None;
    }
    let sport = u16::from_be_bytes([udp[0], udp[1]]);
    let dport = u16::from_be_bytes([udp[2], udp[3]]);
    Some(((src, sport, dst, dport), &udp[8..len]))
}

/// The IPv4 packets of a reply of the flow `key`: from the destination of
/// the flow to its source. A reply larger than `mtu` becomes fragments.
pub(crate) fn reply_packets(key: UdpKey, payload: &[u8], mtu: usize, id: u16) -> Vec<Vec<u8>> {
    let (src, sport, dst, dport) = key;
    let mut udp = Vec::with_capacity(8 + payload.len());
    udp.extend_from_slice(&dport.to_be_bytes());
    udp.extend_from_slice(&sport.to_be_bytes());
    udp.extend_from_slice(&((8 + payload.len()) as u16).to_be_bytes());
    // A zero UDP checksum means "none" on IPv4.
    udp.extend_from_slice(&[0, 0]);
    udp.extend_from_slice(payload);
    // Each fragment but the last holds a multiple of 8 bytes.
    let room = (mtu - 20) & !7;
    let mut packets = Vec::with_capacity(udp.len() / room + 1);
    let mut offset = 0;
    loop {
        let end = (offset + room).min(udp.len());
        let more = end < udp.len();
        let total = 20 + end - offset;
        let flags = (u16::from(more) << 13) | (offset / 8) as u16;
        let mut out = Vec::with_capacity(total);
        out.extend_from_slice(&[0x45, 0]);
        out.extend_from_slice(&(total as u16).to_be_bytes());
        out.extend_from_slice(&id.to_be_bytes());
        out.extend_from_slice(&flags.to_be_bytes());
        out.extend_from_slice(&[64, libc::IPPROTO_UDP as u8, 0, 0]);
        out.extend_from_slice(&dst.octets());
        out.extend_from_slice(&src.octets());
        let checksum = ipv4_checksum(&out[..20]);
        out[10..12].copy_from_slice(&checksum.to_be_bytes());
        out.extend_from_slice(&udp[offset..end]);
        packets.push(out);
        if !more {
            return packets;
        }
        offset = end;
    }
}

impl UdpFlows {
    /// Take each UDP datagram of the program from `rx`, give it to its
    /// flow, and put the replies of the flows in `tx`.
    pub(crate) fn relay(
        &mut self,
        rx: &mut VecDeque<Vec<u8>>,
        tx: &mut VecDeque<Vec<u8>>,
        route: &UdpRoute<'_>,
    ) {
        let now = Instant::now();
        let mut id: u16 = 0;
        while let Ok((key, payload)) = self.replies.try_recv() {
            if self.table.touch(&key, now).is_some() {
                id = id.wrapping_add(1);
                tx.extend(reply_packets(key, &payload, TUN_MTU, id));
            }
        }
        while let Some((key, payload)) = self.held.pop_front() {
            if let Err(payload) = self.give(key, payload, now, route) {
                self.held.push_front((key, payload));
                break;
            }
        }
        let mut kept = VecDeque::with_capacity(rx.len());
        while let Some(pkt) = rx.pop_front() {
            let Some((key, payload)) = parse_udp(&pkt) else {
                kept.push_back(pkt);
                continue;
            };
            let payload = payload.to_vec();
            if self.paused() {
                self.held.push_back((key, payload));
            } else if let Err(payload) = self.give(key, payload, now, route) {
                if route.verbose {
                    eprintln!(
                        "{}: the UDP queue of {}:{} is full; the pod stops the read of the TUN",
                        program_name(),
                        key.2,
                        key.3
                    );
                }
                self.held.push_back((key, payload));
            }
        }
        *rx = kept;
        self.table.reap(now);
    }

    /// Give `payload` to the flow of `key`; make the flow when it does not
    /// exist. `Err(payload)` when the queue of the flow is full.
    fn give(
        &mut self,
        key: UdpKey,
        payload: Vec<u8>,
        now: Instant,
        route: &UdpRoute<'_>,
    ) -> Result<(), Vec<u8>> {
        use tokio::sync::mpsc::error::TrySendError;
        if self.table.touch(&key, now).is_none() {
            let flow = self.start(key, route);
            self.table.insert(key, flow, now);
        }
        let Some(flow) = self.table.touch(&key, now) else {
            return Ok(());
        };
        match flow.queue.try_send(payload) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(payload)) => Err(payload),
            Err(TrySendError::Closed(_)) => {
                // The flow ended (its upstream failed): the next datagram
                // makes a new one. This datagram is lost, as on a network.
                self.table.remove(&key);
                Ok(())
            }
        }
    }

    /// Start the task of a new flow.
    fn start(&self, key: UdpKey, route: &UdpRoute<'_>) -> UdpFlow {
        let name = route.dns.name_for(key.2).map(str::to_string);
        let target = match &name {
            Some(name) => format!("{name}:{}", key.3),
            None => format!("{}:{}", key.2, key.3),
        };
        let chain = select_chain(route.rules, route.proxies, &target).to_vec();
        if route.verbose {
            let hops: Vec<String> = chain.iter().take(1).map(Proxy::endpoint).collect();
            let via = match hops.is_empty() {
                true => "direct".to_string(),
                false => hops.join(""),
            };
            eprintln!("{}: udp {target} via {via}", program_name());
        }
        let (queue, datagrams) = tokio::sync::mpsc::channel(UDP_QUEUE);
        let replies = self.replies_tx.clone();
        let verbose = route.verbose;
        let task = tokio::spawn(async move {
            let ends = Ends { key, name, replies };
            if let Err(e) = carry(chain.first(), datagrams, ends).await {
                if verbose {
                    eprintln!("{}: udp {target}: {e:#}", program_name());
                }
            }
        });
        UdpFlow { queue, task }
    }
}

/// The datagrams of the program for a flow.
type Datagrams = tokio::sync::mpsc::Receiver<Vec<u8>>;

/// The flow of a task, and the way back to the stack.
struct Ends {
    key: UdpKey,
    name: Option<String>,
    replies: tokio::sync::mpsc::Sender<(UdpKey, Vec<u8>)>,
}

impl Ends {
    /// The SOCKS5 UDP datagram of `payload`: a name of the pod as the name.
    fn wrap(&self, payload: &[u8]) -> Result<Vec<u8>> {
        let (_, _, ip, port) = self.key;
        Ok(match &self.name {
            Some(name) => pod_proxy::udp::wrap_name(payload, name, port)?,
            None => pod_proxy::udp::wrap(payload, IpAddr::V4(ip), port),
        })
    }

    /// Give the payload of the SOCKS5 UDP datagram `datagram` to the stack.
    async fn reply(&self, datagram: &[u8]) -> Result<()> {
        if let Some((_, off)) = crate::inbox::udp_source(datagram) {
            let payload = datagram[off..].to_vec();
            self.replies
                .send((self.key, payload))
                .await
                .map_err(|_| anyhow::anyhow!("the stack ended"))?;
        }
        Ok(())
    }
}

/// Carry the datagrams of a flow until its queue or its upstream ends.
async fn carry(proxy: Option<&Proxy>, mut datagrams: Datagrams, ends: Ends) -> Result<()> {
    let Some(proxy) = proxy else {
        return carry_direct(datagrams, ends).await;
    };
    if proxy.kind != ProxyKind::Socks5 {
        bail!("UDP needs a SOCKS5 proxy");
    }
    if proxy.is_unix() {
        let control = within_timeout("udp in the stream dial", dial_proxy(proxy)).await?;
        if let Ok(stream) = within_timeout(
            "udp in the stream",
            crate::upstream::socks5_udp_in_stream(control, proxy),
        )
        .await
        {
            return carry_in_stream(stream, datagrams, ends).await;
        }
    }
    let control = within_timeout("udp associate dial", dial_proxy(proxy)).await?;
    let (mut control, relay) =
        within_timeout("udp associate", socks5_udp_associate(control, proxy)).await?;
    let local = match relay {
        SocketAddr::V4(_) => "0.0.0.0:0",
        SocketAddr::V6(_) => "[::]:0",
    };
    let socket = tokio::net::UdpSocket::bind(local).await?;
    let out = async {
        while let Some(payload) = datagrams.recv().await {
            socket.send_to(&ends.wrap(&payload)?, relay).await?;
        }
        Ok::<(), anyhow::Error>(())
    };
    let back = async {
        let mut buf = vec![0u8; 65536];
        loop {
            let (n, _) = socket.recv_from(&mut buf).await?;
            ends.reply(&buf[..n]).await?;
        }
    };
    // The association lives while its control connection is open.
    let control = async {
        use tokio::io::AsyncReadExt;
        let mut byte = [0u8; 1];
        while control.read(&mut byte).await? > 0 {}
        bail!("the proxy closed the UDP association")
    };
    tokio::select! {
        r = out => r,
        r = back => r,
        r = control => r,
    }
}

/// Carry the datagrams as frames on the stream of the proxy.
async fn carry_in_stream(stream: Upstream, mut datagrams: Datagrams, ends: Ends) -> Result<()> {
    let (mut read, mut write) = tokio::io::split(stream);
    let out = async {
        while let Some(payload) = datagrams.recv().await {
            pod_proxy::udp::write_frame(&mut write, &ends.wrap(&payload)?).await?;
        }
        Ok::<(), anyhow::Error>(())
    };
    let back = async {
        while let Some(datagram) = pod_proxy::udp::read_frame(&mut read).await? {
            ends.reply(&datagram).await?;
        }
        bail!("the proxy closed the UDP stream")
    };
    tokio::select! {
        r = out => r,
        r = back => r,
    }
}

/// A `direct` rule: send the datagrams from a UDP socket of the host.
async fn carry_direct(mut datagrams: Datagrams, ends: Ends) -> Result<()> {
    let (_, _, ip, port) = ends.key;
    let socket = tokio::net::UdpSocket::bind("0.0.0.0:0").await?;
    match &ends.name {
        Some(name) => socket.connect((name.as_str(), port)).await?,
        None => socket.connect((ip, port)).await?,
    }
    let out = async {
        while let Some(payload) = datagrams.recv().await {
            socket.send(&payload).await?;
        }
        Ok::<(), anyhow::Error>(())
    };
    let back = async {
        let mut buf = vec![0u8; 65536];
        loop {
            let n = socket.recv(&mut buf).await?;
            ends.replies
                .send((ends.key, buf[..n].to_vec()))
                .await
                .map_err(|_| anyhow::anyhow!("the stack ended"))?;
        }
    };
    tokio::select! {
        r = out => r,
        r = back => r,
    }
}

#[cfg(test)]
mod tests;
