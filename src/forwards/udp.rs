//! UDP forwards (T-099): each client of a UDP listener has its own flow
//! over [`Dial::datagrams`] of the network. A flow ends after an idle time.
//! A queue of one flow has a bound. A datagram for a full queue, and a
//! datagram for a new flow over the cap of flows, are dropped: UDP has no
//! flow control.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::net::UdpSocket;
use tokio::sync::mpsc;

use crate::network::{recv_from, send_to, Dial, Target};
use podnet_vnet::dns::Resolver;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

/// The idle time after which a flow ends.
pub const IDLE: Duration = Duration::from_secs(60);
/// The most flows at the same time.
pub const MAX_FLOWS: usize = 1024;
/// The datagrams that wait for one flow.
const QUEUE: usize = 64;
/// The largest datagram.
const MAX_DATAGRAM: usize = 65535;

/// The flows of one listener.
#[derive(Clone)]
pub(super) struct Flows {
    idle: Duration,
    cap: usize,
    open: Arc<Mutex<HashMap<SocketAddr, mpsc::Sender<Vec<u8>>>>>,
    /// The names that a DNS door answers itself (T-404).
    local: Option<Arc<Resolver>>,
    /// A DNS door: a query that cannot go to the upstream gets SERVFAIL
    /// (GitHub #638), not a silent timeout.
    servfail: bool,
}

impl Flows {
    pub(super) fn new(idle: Duration, cap: usize) -> Flows {
        Flows {
            idle,
            cap,
            open: Arc::default(),
            local: None,
            servfail: false,
        }
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.open.lock().unwrap().len()
    }
}

/// Carry each datagram of `socket` through `dial` to `target`, and each
/// answer back to its client, until the socket fails.
pub async fn serve_udp(socket: Arc<UdpSocket>, dial: Arc<dyn Dial>, target: Target) -> Result<()> {
    serve_udp_with(socket, dial, target, Flows::new(IDLE, MAX_FLOWS)).await
}

/// The address of a target: a name resolves with the resolver of the
/// host (the datagrams of a network take an address).
async fn resolve(target: &Target) -> Result<SocketAddr> {
    match target {
        Target::Addr(a) => Ok(*a),
        Target::Name { host, port } => tokio::net::lookup_host((host.as_str(), *port))
            .await?
            .next()
            .with_context(|| format!("no address for {target}")),
    }
}

pub(super) async fn serve_udp_with(
    socket: Arc<UdpSocket>,
    dial: Arc<dyn Dial>,
    target: Target,
    flows: Flows,
) -> Result<()> {
    let mut buf = vec![0u8; MAX_DATAGRAM];
    loop {
        let (n, client) = socket
            .recv_from(&mut buf)
            .await
            .context("the forward listener")?;
        if let Some(reply) = flows.local.as_ref().and_then(|t| t.handle(&buf[..n])) {
            // egress-exempt: the answer to the client that sent the datagram
            let _ = socket.send_to(&reply, client).await;
            continue;
        }
        let datagram = buf[..n].to_vec();
        let queue = {
            let mut open = flows.open.lock().unwrap();
            match open.get(&client) {
                Some(queue) => queue.clone(),
                None if open.len() >= flows.cap => {
                    tracing::debug!(%client, "too many UDP flows; a datagram dropped");
                    continue;
                }
                None => {
                    let (queue, rx) = mpsc::channel(QUEUE);
                    open.insert(client, queue.clone());
                    tokio::spawn(flow(
                        socket.clone(),
                        dial.clone(),
                        target.clone(),
                        client,
                        rx,
                        flows.clone(),
                    ));
                    queue
                }
            }
        };
        if queue.try_send(datagram).is_err() {
            tracing::debug!(%client, "a UDP flow is full or ended; a datagram dropped");
        }
    }
}

/// A DNS door: the TCP and the UDP listener on `addr` carry each query
/// through `dial` to `upstream`. The upstream is an address of the far end
/// of the network.
pub async fn serve_dns(addr: SocketAddr, dial: Arc<dyn Dial>, upstream: Target) -> Result<()> {
    serve_dns_with(addr, dial, upstream, None).await
}

/// [`serve_dns`] that answers the names of `table` itself (T-404): over UDP,
/// and over TCP when the first query of a connection asks for one of them.
pub async fn serve_dns_with(
    addr: SocketAddr,
    dial: Arc<dyn Dial>,
    upstream: Target,
    table: Option<Arc<Resolver>>,
) -> Result<()> {
    let (udp, tcp) = bind_dns(addr).await?;
    serve_dns_on(udp, tcp, dial, upstream, table).await
}

/// The listeners of a DNS door on `addr` (GitHub #638): UDP, and TCP on the
/// same port. An address in use exits 69. The caller binds them before it
/// says that the door serves.
pub async fn bind_dns(addr: SocketAddr) -> Result<(Arc<UdpSocket>, tokio::net::TcpListener)> {
    // egress-exempt: the DNS door's listener
    let udp = UdpSocket::bind(addr)
        .await
        .with_context(|| format!("--dns {addr}"))
        .map_err(crate::net::bind_fault)?;
    // The TCP door takes the port of the UDP one (port 0: the port that the
    // kernel chose), so it needs the bound address.
    let bound = udp.local_addr().with_context(|| {
        format!("--dns {addr}: the bound address of the UDP door (getsockname)")
    })?;
    let tcp = tokio::net::TcpListener::bind(bound)
        .await
        .with_context(|| format!("--dns {addr}"))
        .map_err(crate::net::bind_fault)?;
    Ok((Arc::new(udp), tcp))
}

/// [`serve_dns_with`] on the listeners of [`bind_dns`].
pub async fn serve_dns_on(
    udp: Arc<UdpSocket>,
    tcp: tokio::net::TcpListener,
    dial: Arc<dyn Dial>,
    upstream: Target,
    table: Option<Arc<Resolver>>,
) -> Result<()> {
    let mut flows = Flows::new(IDLE, MAX_FLOWS);
    flows.servfail = true;
    let Some(table) = table else {
        return tokio::select! {
            r = serve_udp_with(udp, dial.clone(), upstream.clone(), flows) => r,
            r = super::serve_tcp(tcp, dial, upstream) => r,
        };
    };
    flows.local = Some(table.clone());
    tokio::select! {
        r = serve_udp_with(udp, dial.clone(), upstream.clone(), flows) => r,
        r = async {
            loop {
                let (stream, _) = tcp.accept().await.context("the DNS door")?;
                tokio::spawn(dns_tcp(stream, dial.clone(), upstream.clone(), table.clone()));
            }
        } => r,
    }
}

/// One TCP connection of the DNS door: each query of the table gets its
/// answer here; at the first other query the connection goes to the
/// upstream, with that query first.
async fn dns_tcp(
    mut stream: tokio::net::TcpStream,
    dial: Arc<dyn Dial>,
    upstream: Target,
    table: Arc<Resolver>,
) {
    loop {
        let mut len = [0u8; 2];
        if stream.read_exact(&mut len).await.is_err() {
            return;
        }
        let mut query = vec![0u8; usize::from(u16::from_be_bytes(len))];
        if stream.read_exact(&mut query).await.is_err() {
            return;
        }
        if let Some(reply) = table.handle(&query) {
            let Ok(n) = u16::try_from(reply.len()) else {
                return;
            };
            if stream.write_all(&n.to_be_bytes()).await.is_err()
                || stream.write_all(&reply).await.is_err()
            {
                return;
            }
            continue;
        }
        let deadline = tokio::time::Instant::now() + super::DIAL_LIMIT;
        match dial.dial(&upstream, deadline).await {
            Ok(mut conn) => {
                if conn.io.write_all(&len).await.is_ok() && conn.io.write_all(&query).await.is_ok()
                {
                    let _ = tokio::io::copy_bidirectional(&mut stream, &mut conn.io).await;
                }
            }
            Err(e) => {
                tracing::warn!(%upstream, way = %dial.label(), "the DNS door did not reach its upstream: {e}");
                if let Some(reply) = servfail(&query).and_then(|r| {
                    let n = u16::try_from(r.len()).ok()?;
                    Some([&n.to_be_bytes()[..], &r].concat())
                }) {
                    let _ = stream.write_all(&reply).await;
                }
            }
        }
        return;
    }
}

async fn flow(
    socket: Arc<UdpSocket>,
    dial: Arc<dyn Dial>,
    target: Target,
    client: SocketAddr,
    mut queue: mpsc::Receiver<Vec<u8>>,
    flows: Flows,
) {
    let mut failed = None;
    if let Err(e) = carry(
        &socket,
        &*dial,
        &target,
        client,
        &mut queue,
        flows.idle,
        &mut failed,
    )
    .await
    {
        tracing::warn!(%target, way = %dial.label(), %client, "a UDP flow ended: {e:#}");
        if flows.servfail {
            // GitHub #638: each query that did not go out gets SERVFAIL.
            flows.open.lock().unwrap().remove(&client);
            queue.close();
            while let Some(query) = failed.take().or_else(|| queue.try_recv().ok()) {
                if let Some(reply) = servfail(&query) {
                    // egress-exempt: the SERVFAIL answer to the querying client
                    let _ = socket.send_to(&reply, client).await;
                }
            }
            return;
        }
    }
    flows.open.lock().unwrap().remove(&client);
}

/// The SERVFAIL answer to the DNS query `query`; `None` when it does not
/// parse.
fn servfail(query: &[u8]) -> Option<Vec<u8>> {
    use podnet_vnet::dns::{build_response, ParsedQuery, RCODE_SERVFAIL};
    let parsed = ParsedQuery::parse(query).ok()?;
    Some(build_response(&parsed, &[], RCODE_SERVFAIL))
}

async fn carry(
    socket: &UdpSocket,
    dial: &dyn Dial,
    target: &Target,
    client: SocketAddr,
    queue: &mut mpsc::Receiver<Vec<u8>>,
    idle: Duration,
    failed: &mut Option<Vec<u8>>,
) -> Result<()> {
    let to = resolve(target).await?;
    let datagrams = dial.datagrams().await?;
    let mut buf = vec![0u8; MAX_DATAGRAM];
    loop {
        let step = async {
            tokio::select! {
                out = queue.recv() => match out {
                    Some(payload) => {
                        // egress-exempt: `dial.datagrams` above was approved by the policy
                        let sent = send_to(&*datagrams, &payload, to).await;
                        if sent.is_err() {
                            *failed = Some(payload);
                        }
                        sent.map(|()| true)
                    }
                    None => Ok(false),
                },
                back = recv_from(&*datagrams, &mut buf) => {
                    let (n, _) = back?;
                    // egress-exempt: the answer to the client of the forward
                    socket.send_to(&buf[..n], client).await.map(|_| true)
                }
            }
        };
        match tokio::time::timeout(idle, step).await {
            Err(_) | Ok(Ok(false)) => return Ok(()),
            Ok(Ok(true)) => {}
            Ok(Err(e)) => return Err(e.into()),
        }
    }
}

#[cfg(test)]
#[path = "udp_tests.rs"]
mod tests;
