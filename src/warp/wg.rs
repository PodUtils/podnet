//! WireGuard for the WARP tunnel (issue 152). The Noise IKpsk2 handshake,
//! the cookie, the transport messages and the timers are the crate pod-wg
//! of PodUtils/podcrates, sans I/O (the WireGuard paper, section 5.4, and
//! wireguard.com/protocol). This module keeps the device of WARP (`Peer`),
//! the UDP socket and the tasks of a tunnel.
//!
//! The device is a WireGuard device of the WARP service (`warp export`):
//! its X25519 key, the key of the peer, and the three `reserved` bytes of
//! `client_id`, which WARP reads in bytes 1 to 3 of each message.

use anyhow::{Context as _, Result};
pub use pod_wg::{
    initiate, initiate_with, Cookie, Initiation, Keys, Received, Session, Stamps, Tunnel,
    HANDSHAKE_DEAD, INITIATION_LEN, KEEPALIVE, MSG_COOKIE, MSG_DATA, MSG_INITIATION, MSG_RESPONSE,
    REKEY_AFTER, REKEY_TIMEOUT, RESPONSE_LEN,
};

// ── The transport of the tunnel ───────────────────────────────────────────

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

/// A WireGuard peer of the WARP service: the keys, the front door and the
/// addresses of the device.
#[derive(Clone)]
pub struct Peer {
    pub keys: Keys,
    pub endpoint: SocketAddr,
    pub v4: Option<std::net::Ipv4Addr>,
    pub v6: Option<std::net::Ipv6Addr>,
}

impl Peer {
    /// The peer of a device of `warp export` (issue 148). `endpoint`
    /// replaces the IPv4 address of the record on port 2408.
    pub fn from_exported(
        exported: &super::export::Exported,
        endpoint: Option<SocketAddr>,
    ) -> Result<Self> {
        use base64::Engine as _;
        let b64 = base64::engine::general_purpose::STANDARD;
        let device = &exported.device;
        let private: [u8; 32] = b64
            .decode(&exported.private_key)
            .ok()
            .and_then(|k| k.try_into().ok())
            .context("the private key of the WireGuard device")?;
        let peer = device
            .config
            .peers
            .first()
            .context("the WireGuard device has no peer")?;
        let peer_public: [u8; 32] = b64
            .decode(&peer.public_key)
            .ok()
            .and_then(|k| k.try_into().ok())
            .context("the key of the WireGuard peer")?;
        let endpoint = match endpoint {
            Some(e) => e,
            None => super::export::wireguard_socket_addr(&peer.endpoint)?,
        };
        Ok(Peer {
            keys: Keys {
                private,
                peer_public,
                reserved: device.reserved().unwrap_or([0; 3]),
            },
            endpoint,
            v4: device.config.interface.addresses.v4.parse().ok(),
            v6: device.config.interface.addresses.v6.parse().ok(),
        })
    }
}

/// The UDP socket of a WireGuard transport: a socket of the host, or a
/// socket of the stack of another session (issues 154 and 155: a tunnel in
/// a tunnel).
#[derive(Clone)]
pub enum Sock {
    Host(Arc<tokio::net::UdpSocket>),
    Stack(Arc<super::ip::UdpConn>, SocketAddr),
}

impl Sock {
    async fn send(&self, message: &[u8]) -> Result<()> {
        match self {
            Sock::Host(s) => {
                s.send(message)
                    .await
                    .context("UDP to the WireGuard front door")?;
            }
            Sock::Stack(c, to) => {
                c.send_to(message.to_vec(), *to)
                    .await
                    .context("UDP to the WireGuard front door, inside the tunnel")?;
            }
        }
        Ok(())
    }

    async fn recv(&self, buf: &mut [u8]) -> Result<usize> {
        match self {
            Sock::Host(s) => Ok(s
                .recv(buf)
                .await
                .context("UDP from the WireGuard front door")?),
            Sock::Stack(c, from) => loop {
                let (data, source) = c
                    .recv_from(buf.len())
                    .await
                    .context("UDP from the WireGuard front door, inside the tunnel")?;
                if source != *from {
                    continue;
                }
                let n = data.len().min(buf.len());
                buf[..n].copy_from_slice(&data[..n]);
                return Ok(n);
            },
        }
    }
}

/// The write half of a WireGuard transport.
pub struct WgTx {
    sock: Sock,
    state: Arc<Mutex<Tunnel>>,
}

/// The read half of a WireGuard transport.
pub struct WgRx {
    sock: Sock,
    state: Arc<Mutex<Tunnel>>,
    buf: Vec<u8>,
}

/// The liveness check: keepalives, the rekey, and a dead path.
pub struct WgKeepalive {
    sock: Sock,
    state: Arc<Mutex<Tunnel>>,
}

fn lock(state: &Mutex<Tunnel>) -> std::sync::MutexGuard<'_, Tunnel> {
    state.lock().unwrap_or_else(|e| e.into_inner())
}

/// The handshake with `peer` on a UDP socket of the host.
pub async fn connect(peer: &Peer) -> Result<(WgTx, WgRx, WgKeepalive)> {
    let bind: SocketAddr = if peer.endpoint.is_ipv4() {
        "0.0.0.0:0".parse().expect("an address")
    } else {
        "[::]:0".parse().expect("an address")
    };
    let sock = tokio::net::UdpSocket::bind(bind)
        .await
        .context("a UDP socket for WireGuard")?;
    sock.connect(peer.endpoint)
        .await
        .with_context(|| format!("UDP to the WireGuard front door {}", peer.endpoint))?;
    connect_on(peer, Sock::Host(Arc::new(sock))).await
}

/// The handshake with `peer` on a UDP socket of `stack` (a tunnel in a
/// tunnel).
pub async fn connect_inside(
    peer: &Peer,
    stack: &super::ip::WarpStack,
) -> Result<(WgTx, WgRx, WgKeepalive)> {
    let conn = stack
        .udp_bind()
        .await
        .context("a UDP socket in the outer tunnel")?;
    connect_on(peer, Sock::Stack(Arc::new(conn), peer.endpoint)).await
}

/// The handshake with `peer` on `sock`: three tries, [`REKEY_TIMEOUT`]
/// apart. A cookie reply of a front door under load gives its mac2 to the
/// next try.
async fn connect_on(peer: &Peer, sock: Sock) -> Result<(WgTx, WgRx, WgKeepalive)> {
    let mut buf = vec![0u8; 2048];
    let mut last_err = None;
    let mut stamps = Stamps::new(SystemTime::now(), Instant::now());
    let mut cookie: Option<Cookie> = None;
    for _ in 0..3 {
        let mut init = pod_wg::initiate_from(
            &peer.keys,
            &mut pod_core::entropy::OsEntropy,
            stamps.next(Instant::now()),
        )?;
        if let Some(cookie) = &cookie {
            init.add_mac2(cookie, Instant::now());
        }
        sock.send(&init.message)
            .await
            .with_context(|| format!("the front door {}", peer.endpoint))?;
        let deadline = tokio::time::Instant::now() + REKEY_TIMEOUT;
        loop {
            let n = match tokio::time::timeout_at(deadline, sock.recv(&mut buf)).await {
                Err(_) => break,
                Ok(Err(err)) => {
                    last_err = Some(err);
                    break;
                }
                Ok(Ok(n)) => n,
            };
            // An empty datagram has no type byte.
            match buf[..n].first() {
                Some(&MSG_RESPONSE) => match init.finish(&peer.keys, &buf[..n], Instant::now()) {
                    Ok(session) => {
                        let now = Instant::now();
                        let state = Arc::new(Mutex::new(Tunnel::new(
                            peer.keys.clone(),
                            session,
                            stamps,
                            now,
                        )));
                        tracing::debug!(endpoint = %peer.endpoint, "the WireGuard handshake is done");
                        return Ok((
                            WgTx {
                                sock: sock.clone(),
                                state: state.clone(),
                            },
                            WgRx {
                                sock: sock.clone(),
                                state: state.clone(),
                                buf: vec![0u8; 2048],
                            },
                            WgKeepalive { sock, state },
                        ));
                    }
                    Err(err) => last_err = Some(err.into()),
                },
                Some(&MSG_COOKIE) => {
                    match Cookie::open(&peer.keys, &init, &buf[..n], Instant::now()) {
                        Ok(fresh) => {
                            cookie = Some(fresh);
                            last_err = Some(anyhow::anyhow!(
                                "the WireGuard front door is under load (a cookie reply)"
                            ));
                        }
                        Err(err) => last_err = Some(err.into()),
                    }
                }
                _ => {}
            }
        }
    }
    Err(last_err.unwrap_or_else(|| {
        anyhow::anyhow!(
            "no WireGuard handshake answer from {} (UDP) in {} s",
            peer.endpoint,
            3 * REKEY_TIMEOUT.as_secs()
        )
    }))
}

impl WgTx {
    /// Seal and send each packet.
    pub async fn send_packets<P: AsRef<[u8]>>(&mut self, packets: &[P]) -> Result<()> {
        let out = lock(&self.state).send(packets, Instant::now())?;
        if let Some(m) = out.handshake {
            self.sock.send(&m).await.context("a WireGuard handshake")?;
        }
        for m in out.messages {
            self.sock.send(&m).await.context("a WireGuard message")?;
        }
        Ok(())
    }
}

impl WgRx {
    /// The next IP packet; a handshake response, a cookie reply, a keepalive
    /// and a message that does not authenticate are taken and not returned.
    pub async fn recv_packet(&mut self) -> Result<Option<bytes::Bytes>> {
        loop {
            let n = self.sock.recv(&mut self.buf).await?;
            let received = lock(&self.state).receive(&self.buf[..n], Instant::now());
            match received {
                Received::Packet(packet) => return Ok(Some(packet.into())),
                Received::Rekeyed => tracing::debug!("a new WireGuard session (rekey)"),
                Received::Cookie => {
                    tracing::debug!(
                        "a WireGuard cookie reply: the next try of the rekey has its mac2"
                    )
                }
                Received::Dropped(err) => {
                    tracing::debug!(error = %err, "a WireGuard message is dropped")
                }
                Received::Keepalive | Received::Ignored => {}
            }
        }
    }
}

impl WgKeepalive {
    /// Ready with an error when the path is dead: a handshake had no answer
    /// in [`HANDSHAKE_DEAD`]. Sends a keepalive when nothing went out for
    /// [`KEEPALIVE`], and the handshake of the rekey.
    pub async fn run(self) -> Result<()> {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            let messages = lock(&self.state).tick(Instant::now())?;
            for m in &messages {
                let _ = self.sock.send(m).await;
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod standin;

#[cfg(test)]
mod tests {
    use super::*;

    fn random<const N: usize>() -> [u8; N] {
        let mut b = [0u8; N];
        getrandom::fill(&mut b).expect("OS entropy");
        b
    }

    #[tokio::test]
    async fn no_answer_is_an_error_that_names_the_door() {
        crate::test_host::need_loopback!();
        let sock = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let peer = Peer {
            keys: Keys {
                private: random::<32>(),
                peer_public: random::<32>(),
                reserved: [0; 3],
            },
            endpoint: sock.local_addr().unwrap(),
            v4: None,
            v6: None,
        };
        tokio::time::pause();
        let task =
            tokio::spawn(async move { connect(&peer).await.err().map(|e| format!("{e:#}")) });
        let err = task.await.unwrap().expect("no answer fails");
        assert!(err.contains("no WireGuard handshake answer"), "{err}");
        drop(sock);
    }
}
