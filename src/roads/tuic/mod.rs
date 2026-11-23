//! The TUIC road of podnet (PodUtils/podnet#486, QUIC-8): a client of TUIC
//! version 5 over the QUIC stack of `pod-quic`, with the bounded pending
//! queue of the UDP mode.
//!
//! The road is the implementation of `podssh-ws`'s TUIC, moved into podnet
//! so that the org has one TUIC implementation: [`command`] is podssh's
//! command layout, [`link`] its link grammar and credential types, and
//! [`stream`] its session stream. Podnet adds the UDP `quic` relay mode and
//! the bounded queue. `docs/quic.md` records the move, the differences and
//! the follow-up that removes the podssh copy.
//!
//! UDP is the `quic` relay mode of the specification: each datagram is one
//! command on a unidirectional stream, so QUIC's own retransmission carries
//! it and a loss of a QUIC packet loses no datagram. The stream from the
//! local socket to that command is a bounded queue. When the queue is full
//! the reader stops reading the local socket (the stop-read signal of
//! QUIC-8), and the next read waits for capacity.

pub mod command;
pub mod link;
pub mod stream;

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use pod_quic::noq;
use pod_quic::noq::crypto::rustls::QuicClientConfig;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use tokio::sync::mpsc;

pub use command::{packet_payload, AUTHENTICATE, CONNECT, HEARTBEAT, PACKET, TOKEN_LEN, VERSION};
pub use link::{Link, Password, Uuid, ALPN};

/// The most datagrams that wait for a stream: the bound of QUIC-8.
pub const PENDING_MAX: usize = 64;

/// The statistics of the bounded queue of the UDP mode (QUIC-8).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct QueueStats {
    /// The most datagrams that waited at one time.
    pub max_pending: usize,
    /// The times the queue was full. A full queue never drops a datagram:
    /// the reader stops reading the local socket until there is room.
    pub full: u64,
}

/// The state of one bounded queue.
#[derive(Debug, Default)]
pub struct QueueState {
    pending: AtomicUsize,
    max: AtomicUsize,
    full: AtomicU64,
    stopped: AtomicBool,
}

impl QueueState {
    pub fn stats(&self) -> QueueStats {
        QueueStats {
            max_pending: self.max.load(Ordering::Relaxed),
            full: self.full.load(Ordering::Relaxed),
        }
    }

    pub fn stopped(&self) -> bool {
        self.stopped.load(Ordering::Relaxed)
    }

    fn sent(&self, pending: usize) {
        self.pending.store(pending, Ordering::Relaxed);
        self.max.fetch_max(pending, Ordering::Relaxed);
    }

    fn full(&self) {
        self.full.fetch_add(1, Ordering::Relaxed);
        self.stopped.store(true, Ordering::Relaxed);
    }
}

/// A bounded queue of datagrams: the reader pushes, the writer takes. The
/// queue's capacity is [`PENDING_MAX`].
pub struct Pending {
    tx: mpsc::Sender<Vec<u8>>,
    state: Arc<QueueState>,
}

impl Pending {
    pub fn new(capacity: usize) -> (Pending, mpsc::Receiver<Vec<u8>>, Arc<QueueState>) {
        let (tx, rx) = mpsc::channel(capacity);
        let state = Arc::new(QueueState::default());
        (
            Pending {
                tx,
                state: state.clone(),
            },
            rx,
            state,
        )
    }

    /// Push a datagram if there is room. `false` means full: the caller
    /// then stops reading its socket, and waits on [`push`](Pending::push).
    pub fn try_push(&self, datagram: Vec<u8>) -> bool {
        match self.tx.try_send(datagram) {
            Ok(()) => {
                self.state.stopped.store(false, Ordering::Relaxed);
                self.state.sent(self.tx.max_capacity() - self.tx.capacity());
                true
            }
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.state.full();
                false
            }
            Err(mpsc::error::TrySendError::Closed(_)) => false,
        }
    }

    /// Push a datagram, waiting for room: while it waits, the caller reads
    /// no socket, which is the stop-read signal.
    pub async fn push(&self, datagram: Vec<u8>) {
        if self.tx.send(datagram).await.is_ok() {
            self.state.stopped.store(false, Ordering::Relaxed);
            self.state.sent(self.tx.max_capacity() - self.tx.capacity());
        }
    }
}

/// The client of one TUIC connection: the authentication stream stays open
/// for the life of the connection.
pub struct Tuic {
    endpoint: noq::Endpoint,
    conn: noq::Connection,
    _auth: noq::SendStream,
}

impl Tuic {
    /// A connection to `link`, authenticated with the token of its TLS
    /// exporter.
    pub async fn connect(link: &Link) -> Result<Tuic> {
        let addr = resolve(&link.host, link.port).await?;
        // No one-packet probe of UDP before the handshake: podssh sends one,
        // but a single probe packet is lost on the lossy link of the QUIC-8
        // gate, and QUIC's own handshake retries its Initials.
        let endpoint = pod_quic::endpoint::system(addr.is_ipv6())
            .context("the endpoint of the TUIC client")?;
        // The road speaks the `quic` relay mode: a `Packet` on its own
        // unidirectional stream. TUIC's reference server sends a reply on a
        // QUIC datagram whenever the peer advertises DATAGRAM support, and
        // reads no stream; a client that advertises it and reads only
        // streams receives no echo. The road advertises none, so the server
        // takes the `quic` mode of the first `Packet` it saw.
        let settings = pod_quic::Settings::default();
        let tls = tls(link)?;
        let crypto = QuicClientConfig::try_from(tls).context("the TLS of TUIC with QUIC")?;
        let mut config = noq::ClientConfig::new(Arc::new(crypto));
        let mut transport = settings.transport(false);
        // `Settings::datagram_buffer: None` keeps noq's default (on), so the
        // transport takes the field directly: the road sends and receives
        // `Packet` on unidirectional streams (`quic` relay mode), and a
        // server that sees DATAGRAM support replies with datagrams, which
        // this road does not read and which QUIC may lose.
        transport.datagram_receive_buffer_size(None);
        config.transport_config(Arc::new(transport));
        let connecting = endpoint
            .connect_with(config, addr, &link.sni)
            .with_context(|| format!("the TUIC connection to {addr}"))?;
        let conn = tokio::time::timeout(settings.handshake_wait, connecting)
            .await
            .context("no answer of the TUIC server")?
            .with_context(|| format!("the TUIC handshake with {addr}"))?;
        let token = command::token(&link.uuid, &link.password, |out, label, context| {
            conn.export_keying_material(out, label, context)
                .map_err(|e| format!("{e:?}"))
        })
        .map_err(|e| anyhow!("the token of TUIC: {e}"))?;
        let mut auth = conn
            .open_uni()
            .await
            .context("the authentication stream of TUIC")?;
        auth.write_all(&command::authenticate(&link.uuid, &token))
            .await
            .context("the authentication of TUIC")?;
        Ok(Tuic {
            endpoint,
            conn,
            _auth: auth,
        })
    }

    /// A TCP connection to `host:port` through the road: the stream, with
    /// the `Connect` command already written.
    pub async fn tcp(&self, host: &str, port: u16) -> Result<(noq::SendStream, noq::RecvStream)> {
        let cmd = command::connect(host, port).map_err(|e| anyhow!("{e}"))?;
        let (mut send, recv) = self.conn.open_bi().await.context("a stream of TUIC")?;
        send.write_all(&cmd)
            .await
            .context("the Connect command of TUIC")?;
        Ok((send, recv))
    }

    /// The same TCP stream as one [`stream::TuicStream`] for a byte pipe.
    pub async fn tcp_io(&self, host: &str, port: u16) -> Result<stream::TuicStream> {
        let (send, recv) = self.tcp(host, port).await?;
        Ok(stream::TuicStream::new(
            self.endpoint.clone(),
            self.conn.clone(),
            send,
            recv,
        ))
    }

    /// A UDP flow of the `quic` relay mode: one unidirectional stream for
    /// each datagram, and the echoes on the server's streams.
    pub fn udp(&self, host: &str, port: u16) -> Result<Udp> {
        // The address is written into each `Packet`: a target that cannot go
        // in one is refused before the first datagram, not after it.
        command::target(host, port).map_err(|e| anyhow!("{e}"))?;
        let (tx, rx) = mpsc::channel(64);
        let conn = self.conn.clone();
        let next = Arc::new(AtomicU64::new(0));
        let assoc = 1u16;
        let accept = {
            let conn = conn.clone();
            tokio::spawn(async move {
                while let Ok(mut stream) = conn.accept_uni().await {
                    let tx = tx.clone();
                    tokio::spawn(async move {
                        let mut buf = Vec::new();
                        while let Ok(Some(chunk)) = stream.read_chunk(4096).await {
                            buf.extend_from_slice(&chunk);
                        }
                        if let Some(payload) = packet_payload(&buf) {
                            let _ = tx.send(payload.to_vec()).await;
                        }
                    });
                }
            })
        };
        Ok(Udp {
            conn,
            host: host.to_string(),
            port,
            next,
            assoc,
            rx: tokio::sync::Mutex::new(rx),
            accept: Arc::new(accept),
        })
    }
}

/// One UDP flow of the road.
pub struct Udp {
    conn: noq::Connection,
    host: String,
    port: u16,
    next: Arc<AtomicU64>,
    assoc: u16,
    rx: tokio::sync::Mutex<mpsc::Receiver<Vec<u8>>>,
    accept: Arc<tokio::task::JoinHandle<()>>,
}

impl Udp {
    /// The two channels of a pipe to stdin and stdout: the datagrams to
    /// send, and those that came back. A task sends each datagram of the
    /// first channel; the task holds the flow, so the reader stays alive
    /// until the first channel closes.
    pub fn into_ends(mut self) -> (mpsc::Sender<Vec<u8>>, mpsc::Receiver<Vec<u8>>) {
        let (_tx, placeholder) = mpsc::channel(1);
        let rx = std::mem::replace(self.rx.get_mut(), placeholder);
        let flow = Arc::new(self);
        let (out, mut input) = mpsc::channel::<Vec<u8>>(PENDING_MAX);
        tokio::spawn(async move {
            while let Some(datagram) = input.recv().await {
                if flow.send(&datagram).await.is_err() {
                    return;
                }
            }
        });
        (out, rx)
    }

    /// Send one datagram as one `Packet` command on a new stream.
    pub async fn send(&self, payload: &[u8]) -> Result<()> {
        let id = self.next.fetch_add(1, Ordering::Relaxed) as u16;
        let cmd = command::packet(self.assoc, id, &self.host, self.port, payload)
            .map_err(|e| anyhow!("{e}"))?;
        let mut stream = self
            .conn
            .open_uni()
            .await
            .context("a stream of a TUIC datagram")?;
        stream.write_all(&cmd).await.context("a TUIC datagram")?;
        stream.finish().context("the end of a TUIC datagram")?;
        Ok(())
    }

    /// The next echo.
    pub async fn recv(&self) -> Option<Vec<u8>> {
        self.rx.lock().await.recv().await
    }
}

impl Drop for Udp {
    fn drop(&mut self) {
        self.accept.abort();
    }
}

/// The statistics of one local UDP relay.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RelayStats {
    pub sent: u64,
    pub received: u64,
    pub queue: QueueStats,
}

/// Relay one local UDP socket through the road to `host:port`, for
/// `run_for`. The reader of the local socket uses a bounded queue: a full
/// queue stops the read, and a datagram is dropped only when the stop-read
/// wait itself is over.
pub async fn udp_relay(
    link: &Link,
    host: &str,
    port: u16,
    local: tokio::net::UdpSocket,
    run_for: Duration,
) -> Result<RelayStats> {
    let client = Arc::new(Tuic::connect(link).await?);
    let flow = Arc::new(client.udp(host, port)?);
    let (pending, mut rx, state) = Pending::new(PENDING_MAX);
    let pending = Arc::new(pending);
    let stats = Arc::new(Mutex::new(RelayStats::default()));
    let writer = tokio::spawn({
        let flow = flow.clone();
        async move {
            while let Some(datagram) = rx.recv().await {
                if flow.send(&datagram).await.is_err() {
                    break;
                }
            }
        }
    });
    let local = Arc::new(local);
    let peer: Arc<Mutex<Option<SocketAddr>>> = Arc::new(Mutex::new(None));
    let echo = {
        let local = local.clone();
        let peer = peer.clone();
        let stats = stats.clone();
        let flow = flow.clone();
        tokio::spawn(async move {
            while let Some(datagram) = flow.recv().await {
                let to = *peer.lock().unwrap_or_else(PoisonError::into_inner);
                if let Some(to) = to {
                    // egress-exempt: a reply on the UDP flow that egress::dial opened
                    let _ = local.send_to(&datagram, to).await;
                    stats
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .received += 1;
                }
            }
        })
    };
    let reader = {
        let pending = pending.clone();
        let peer = peer.clone();
        let stats = stats.clone();
        tokio::spawn(async move {
            let mut buf = vec![0u8; 65535];
            loop {
                let (n, from) = match local.recv_from(&mut buf).await {
                    Ok(got) => got,
                    Err(_) => break,
                };
                *peer.lock().unwrap_or_else(PoisonError::into_inner) = Some(from);
                let datagram = buf[..n].to_vec();
                if pending.try_push(datagram.clone()) {
                    stats.lock().unwrap_or_else(PoisonError::into_inner).sent += 1;
                } else {
                    pending.push(datagram).await;
                    stats.lock().unwrap_or_else(PoisonError::into_inner).sent += 1;
                }
            }
        })
    };
    let _ = tokio::time::timeout(run_for, reader).await;
    writer.abort();
    echo.abort();
    let mut out = *stats.lock().unwrap_or_else(PoisonError::into_inner);
    out.queue = state.stats();
    Ok(out)
}

/// The TLS of the client: the system's roots, or no check for a link that
/// says `allow_insecure=1`.
fn tls(link: &Link) -> Result<Arc<rustls::ClientConfig>> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let alpn: Vec<Vec<u8>> = link.alpn.clone();
    if link.allow_insecure {
        let mut config = rustls::ClientConfig::builder_with_provider(provider.clone())
            .with_protocol_versions(&[&rustls::version::TLS13])
            .context("the TLS 1.3 of TUIC")?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AcceptAny(provider)))
            .with_no_client_auth();
        config.alpn_protocols = alpn;
        return Ok(Arc::new(config));
    }
    let roots = Arc::new(rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    });
    let refs: Vec<&[u8]> = link.alpn.iter().map(|a| a.as_slice()).collect();
    pod_quic::connect::tls(provider, roots, &refs).map_err(|e| anyhow!(e))
}

/// A verifier of the certificate of the road that accepts any certificate:
/// the link asked for it, and the password still authenticates the session.
#[derive(Debug)]
struct AcceptAny(Arc<rustls::crypto::CryptoProvider>);

impl ServerCertVerifier for AcceptAny {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// The address of a link: a literal as it is, else the system's resolver.
async fn resolve(host: &str, port: u16) -> Result<SocketAddr> {
    if let Ok(addr) = host.parse::<std::net::IpAddr>() {
        return Ok(SocketAddr::new(addr, port));
    }
    let mut addrs = tokio::net::lookup_host((host, port))
        .await
        .with_context(|| format!("the address of {host}"))?;
    addrs.next().ok_or_else(|| anyhow!("no address of {host}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The queue stops the read at its bound, counts the stop, and takes
    /// datagrams again when the writer drains it.
    #[tokio::test]
    async fn the_queue_stops_the_read_at_its_bound() {
        let (pending, mut rx, state) = Pending::new(4);
        for n in 0..4 {
            assert!(pending.try_push(vec![n]));
        }
        assert!(!pending.try_push(vec![9]));
        let stats = state.stats();
        assert_eq!(stats.max_pending, 4);
        assert_eq!(stats.full, 1);
        assert!(state.stopped());
        let _ = rx.recv().await;
        assert!(pending.try_push(vec![10]));
        assert!(!state.stopped());
    }
}
