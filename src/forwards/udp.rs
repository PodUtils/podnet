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
}

impl Flows {
    pub(super) fn new(idle: Duration, cap: usize) -> Flows {
        Flows {
            idle,
            cap,
            open: Arc::default(),
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
    let udp = Arc::new(
        UdpSocket::bind(addr)
            .await
            .with_context(|| format!("--dns {addr}"))?,
    );
    let tcp = tokio::net::TcpListener::bind(udp.local_addr()?)
        .await
        .with_context(|| format!("--dns {addr}"))?;
    tokio::select! {
        r = serve_udp(udp, dial.clone(), upstream.clone()) => r,
        r = super::serve_tcp(tcp, dial, upstream) => r,
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
    if let Err(e) = carry(&socket, &*dial, &target, client, &mut queue, flows.idle).await {
        tracing::warn!(%target, way = %dial.label(), %client, "a UDP flow ended: {e:#}");
    }
    flows.open.lock().unwrap().remove(&client);
}

async fn carry(
    socket: &UdpSocket,
    dial: &dyn Dial,
    target: &Target,
    client: SocketAddr,
    queue: &mut mpsc::Receiver<Vec<u8>>,
    idle: Duration,
) -> Result<()> {
    let to = resolve(target).await?;
    let datagrams = dial.datagrams().await?;
    let mut buf = vec![0u8; MAX_DATAGRAM];
    loop {
        let step = async {
            tokio::select! {
                out = queue.recv() => match out {
                    Some(payload) => send_to(&*datagrams, &payload, to).await.map(|()| true),
                    None => Ok(false),
                },
                back = recv_from(&*datagrams, &mut buf) => {
                    let (n, _) = back?;
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
