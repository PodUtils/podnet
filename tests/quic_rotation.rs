//! The rotation gate of PodUtils/podnet#486 (QUIC-5 and QUIC-9): a live
//! QUIC connection survives two rotations of its local connection IDs and
//! its local address with no byte lost, and a peer that refuses migration
//! gets a new connection and a resumed session.
//!
//! The switch of datagrams is in memory (no socket of the system), and the
//! clock is the clock of tokio, paused by the test, so the 10 minutes and
//! the two rotations are simulated time. The test measures what the session
//! does, not the wall clock.
//!
//! The session is the one of `pod-quic`'s own migration test, ported here
//! to the certificate of podnet's tests.

use std::collections::HashMap;
use std::io;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use pod_core::{BoxFuture, Class, Datagrams, Failure, Stage};
use pod_quic::migrate::{self, Carried, Local, Locals, Migrator, Outcome, Schedule};
use pod_quic::noq;
use pod_quic::noq::crypto::rustls::QuicServerConfig;
use pod_quic::queued::{Link, QueuedDatagrams};
use sha2::{Digest as _, Sha256};
use tokio::sync::mpsc;

/// The name of the certificate of the test server.
const NAME: &str = "quic.example";
/// The ALPN of the test session.
const ALPN: &[u8] = b"pod-quic-test";

/// The provider of the test, and the certificate of its server.
fn crypto() -> (
    Arc<rustls::crypto::CryptoProvider>,
    Arc<QuicServerConfig>,
    Arc<rustls::RootCertStore>,
) {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let ca_key = rcgen::KeyPair::generate().unwrap();
    let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "podnet rotation test CA");
    let ca = params.self_signed(&ca_key).unwrap();
    let issuer = rcgen::Issuer::new(params, ca_key);
    let leaf_key = rcgen::KeyPair::generate().unwrap();
    let leaf = rcgen::CertificateParams::new(vec![NAME.to_string()])
        .unwrap()
        .signed_by(&leaf_key, &issuer)
        .unwrap();
    let mut tls = rustls::ServerConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![leaf.der().clone(), ca.der().clone()],
            rustls::pki_types::PrivateKeyDer::Pkcs8(leaf_key.serialize_der().into()),
        )
        .unwrap();
    tls.alpn_protocols = vec![ALPN.to_vec()];
    let mut roots = rustls::RootCertStore::empty();
    roots.add(ca.der().clone()).unwrap();
    (
        provider,
        Arc::new(QuicServerConfig::try_from(Arc::new(tls)).unwrap()),
        Arc::new(roots),
    )
}

/// The address of a port of the switch.
fn addr(port: u16) -> SocketAddr {
    SocketAddr::new(Ipv4Addr::LOCALHOST.into(), port)
}

/// The queue of one port of the switch: the datagram and the source.
type Queue = (Vec<u8>, SocketAddr);

/// A switch of datagrams between the ports of the test, in memory. A port
/// that refuses migration drops each datagram of a source address other
/// than the first that reached it, for a time; after that time it takes the
/// new source, as a path that opens again.
#[derive(Default)]
struct Wire {
    ports: Mutex<HashMap<SocketAddr, mpsc::UnboundedSender<Queue>>>,
    first: Mutex<HashMap<SocketAddr, SocketAddr>>,
    refusal: Mutex<HashMap<SocketAddr, tokio::time::Instant>>,
    dropped: AtomicU64,
}

impl Wire {
    fn port(self: &Arc<Self>, at: SocketAddr) -> Port {
        let (tx, rx) = mpsc::unbounded_channel();
        self.ports
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(at, tx);
        Port {
            me: at,
            wire: self.clone(),
            rx: tokio::sync::Mutex::new(rx),
        }
    }

    /// The port `at` drops each datagram from a source address other than
    /// the first that reached it, for `within`.
    fn refuse_migration(&self, at: SocketAddr, within: Duration) {
        let until = tokio::time::Instant::now() + within;
        self.refusal
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(at, until);
    }

    fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

struct Port {
    me: SocketAddr,
    wire: Arc<Wire>,
    rx: tokio::sync::Mutex<mpsc::UnboundedReceiver<Queue>>,
}

impl Link for Port {
    fn send_to(&self, datagram: Vec<u8>, to: SocketAddr) -> BoxFuture<'_, io::Result<()>> {
        Box::pin(async move {
            let tx = self
                .wire
                .ports
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(&to)
                .cloned();
            let Some(tx) = tx else {
                self.wire.dropped.fetch_add(1, Ordering::Relaxed);
                return Ok(());
            };
            // The source address that the port knows: the first that reached it.
            let known = {
                let mut first = self
                    .wire
                    .first
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                *first.entry(to).or_insert(self.me)
            };
            let refused = {
                let mut refusal = self
                    .wire
                    .refusal
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                match refusal.get(&to) {
                    Some(until) if tokio::time::Instant::now() < *until => true,
                    Some(_) => {
                        refusal.remove(&to);
                        self.wire
                            .first
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .remove(&to);
                        false
                    }
                    None => false,
                }
            };
            if refused && known != self.me {
                self.wire.dropped.fetch_add(1, Ordering::Relaxed);
                return Ok(());
            }
            let _ = tx.send((datagram, self.me));
            Ok(())
        })
    }

    fn recv_from(&self, max: usize) -> BoxFuture<'_, io::Result<(Vec<u8>, SocketAddr)>> {
        Box::pin(async move {
            let mut rx = self.rx.lock().await;
            match rx.recv().await {
                Some((mut data, from)) => {
                    data.truncate(max);
                    Ok((data, from))
                }
                None => Err(io::Error::from(io::ErrorKind::BrokenPipe)),
            }
        })
    }

    fn local_addr(&self) -> SocketAddr {
        self.me
    }
}

/// The ports of the local addresses of a client: a new one for each
/// rotation, and each kept alive for the endpoint that it served.
struct Ports {
    wire: Arc<Wire>,
    next: AtomicU64,
    kept: Mutex<Vec<Arc<QueuedDatagrams>>>,
}

impl Ports {
    fn new(wire: Arc<Wire>, from: u16) -> Arc<Ports> {
        Arc::new(Ports {
            wire,
            next: AtomicU64::new(u64::from(from)),
            kept: Mutex::new(Vec::new()),
        })
    }
}

impl Locals for Ports {
    fn next(&self) -> BoxFuture<'_, Result<Local, Failure>> {
        Box::pin(async move {
            let n = self.next.fetch_add(1, Ordering::Relaxed);
            let port = u16::try_from(n)
                .map_err(|_| Failure::new(Stage::Connect, Class::Usage, "no port left"))?;
            let datagrams = Arc::new(QueuedDatagrams::new(Arc::new(self.wire.port(addr(port)))));
            self.kept
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(datagrams.clone());
            Ok(Local::Datagrams(datagrams as Arc<dyn Datagrams>))
        })
    }
}

/// The byte of offset `i` of the transfer: a value that the two sides
/// compute alone, so that a session resumes at any offset.
fn byte_at(i: u64) -> u8 {
    let mut z = i.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    (z ^ (z >> 31)) as u8
}

/// The frame of the test session: the kind, the length (u32 LE), the
/// payload.
async fn frame(send: &mut noq::SendStream, kind: u8, payload: &[u8]) -> Result<(), Failure> {
    let mut out = Vec::with_capacity(payload.len() + 5);
    out.push(kind);
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(payload);
    send.write_all(&out).await.map_err(|e| {
        Failure::caused(
            Stage::Quic,
            Class::Unavailable,
            "a write of the test session",
            e,
        )
    })
}

/// The server of the test: it counts and hashes the data frames, echoes a
/// probe, and answers a count. Each connection starts with an 8-byte
/// resume offset: the client's count of sent bytes; the answer is the
/// count of bytes received.
async fn serve<F>(
    conn: noq::Connection,
    count: Arc<AtomicU64>,
    hash: Arc<Mutex<Sha256>>,
    on_open: F,
) where
    F: FnOnce(u64) -> u64 + Send + 'static,
{
    let Ok((mut send, mut recv)) = conn.accept_bi().await else {
        return;
    };
    let mut offset = [0u8; 8];
    if recv.read_exact(&mut offset).await.is_err() {
        return;
    }
    let resumed = on_open(u64::from_le_bytes(offset));
    if send.write_all(&resumed.to_le_bytes()).await.is_err() {
        return;
    }
    let mut head = [0u8; 5];
    loop {
        if recv.read_exact(&mut head).await.is_err() {
            return;
        }
        let len = u32::from_le_bytes([head[1], head[2], head[3], head[4]]) as usize;
        let mut payload = vec![0u8; len];
        if recv.read_exact(&mut payload).await.is_err() {
            return;
        }
        match head[0] {
            0 => {
                hash.lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .update(&payload);
                count.fetch_add(len as u64, Ordering::SeqCst);
            }
            1 => {
                if send.write_all(&payload).await.is_err() {
                    return;
                }
            }
            2 => {
                let now = count.load(Ordering::SeqCst).to_le_bytes();
                if send.write_all(&now).await.is_err() {
                    return;
                }
            }
            _ => return,
        }
    }
}

/// A session of the test on one connection: a framed stream both ways, and
/// the way to open the next connection at the byte where this one stopped.
struct Session {
    conn: noq::Connection,
    send: noq::SendStream,
    recv: noq::RecvStream,
    /// The offset of the next data byte that the caller gives the session.
    sent: u64,
    /// The opens of the connections, and the number of their resumes.
    opens: Arc<AtomicU64>,
    resume: Arc<Resume>,
}

struct Resume {
    endpoint: noq::Endpoint,
    tls: Arc<rustls::ClientConfig>,
    settings: pod_quic::Settings,
    door: SocketAddr,
}

impl Resume {
    /// A connection, its stream, and the count of bytes that the peer has.
    async fn open(
        &self,
        sent: u64,
    ) -> Result<(noq::Connection, noq::SendStream, noq::RecvStream, u64), Failure> {
        let conn = pod_quic::connect::connect(
            &self.endpoint,
            self.door,
            NAME,
            self.tls.clone(),
            &self.settings,
        )
        .await?;
        let (mut send, mut recv) = conn.open_bi().await.map_err(|e| {
            Failure::caused(
                Stage::Quic,
                Class::Unavailable,
                "the stream of the test session",
                e,
            )
        })?;
        send.write_all(&sent.to_le_bytes()).await.map_err(|e| {
            Failure::caused(Stage::Quic, Class::Unavailable, "the resume offset", e)
        })?;
        let mut got = [0u8; 8];
        recv.read_exact(&mut got).await.map_err(|e| {
            Failure::caused(
                Stage::Quic,
                Class::Unavailable,
                "the answer of the resume",
                e,
            )
        })?;
        Ok((conn, send, recv, u64::from_le_bytes(got)))
    }
}

impl Session {
    async fn start(resume: Arc<Resume>, opens: Arc<AtomicU64>) -> Result<Session, Failure> {
        let (conn, send, recv, received) = resume.open(0).await?;
        opens.fetch_add(1, Ordering::SeqCst);
        Ok(Session {
            conn,
            send,
            recv,
            sent: received,
            opens,
            resume,
        })
    }

    /// Send `chunk` bytes from the offset of now, and raise the offset.
    async fn data(&mut self, chunk: usize) -> Result<(), Failure> {
        let payload: Vec<u8> = (0..chunk as u64).map(|i| byte_at(self.sent + i)).collect();
        frame(&mut self.send, 0, &payload).await?;
        self.sent += chunk as u64;
        Ok(())
    }

    /// The count of bytes that the peer has.
    async fn count(&mut self) -> Result<u64, Failure> {
        frame(&mut self.send, 2, &[]).await?;
        let mut got = [0u8; 8];
        self.recv.read_exact(&mut got).await.map_err(|e| {
            Failure::caused(Stage::Quic, Class::Unavailable, "the count of the peer", e)
        })?;
        Ok(u64::from_le_bytes(got))
    }
}

impl Carried for Session {
    fn connection(&self) -> noq::Connection {
        self.conn.clone()
    }

    fn probe(&mut self) -> BoxFuture<'_, Result<(), Failure>> {
        Box::pin(async move {
            let token = self.sent.to_le_bytes();
            frame(&mut self.send, 1, &token).await?;
            let mut back = [0u8; 8];
            tokio::time::timeout(
                Duration::from_millis(PROBE),
                self.recv.read_exact(&mut back),
            )
            .await
            .map_err(|_| Failure::new(Stage::Quic, Class::Timeout, "no answer of the probe"))?
            .map_err(|e| Failure::caused(Stage::Quic, Class::Unavailable, "the probe", e))?;
            if back != token {
                return Err(Failure::new(
                    Stage::Quic,
                    Class::Protocol,
                    "the probe came back changed",
                ));
            }
            Ok(())
        })
    }

    fn reopen(&mut self) -> BoxFuture<'_, Result<noq::Connection, Failure>> {
        Box::pin(async move {
            let (conn, send, recv, received) = self.resume.open(self.sent).await?;
            self.opens.fetch_add(1, Ordering::SeqCst);
            self.conn = conn.clone();
            self.send = send;
            self.recv = recv;
            self.sent = received;
            Ok(conn)
        })
    }
}

/// The probe's own limit, below the limit of [`migrate::PROBE_WAIT`].
const PROBE: u64 = 400;

/// The server of the test on its port, and the counts that it keeps.
struct Edge {
    count: Arc<AtomicU64>,
    hash: Arc<Mutex<Sha256>>,
    resumptions: Arc<Mutex<Vec<u64>>>,
}

async fn edge(wire: Arc<Wire>, crypto: Arc<QuicServerConfig>) -> Edge {
    let port = wire.port(addr(40000));
    let server_config = pod_quic::endpoint::server_config(crypto).unwrap();
    let datagrams: Arc<dyn Datagrams> = Arc::new(QueuedDatagrams::new(Arc::new(port)));
    let server = pod_quic::endpoint::over(datagrams, Some(server_config)).unwrap();
    let count = Arc::new(AtomicU64::new(0));
    let hash = Arc::new(Mutex::new(Sha256::new()));
    let resumptions = Arc::new(Mutex::new(Vec::new()));
    let (c, h, r) = (count.clone(), hash.clone(), resumptions.clone());
    tokio::spawn(async move {
        while let Some(incoming) = server.accept().await {
            let Ok(conn) = incoming.await else { continue };
            let (c, h, r) = (c.clone(), h.clone(), r.clone());
            tokio::spawn(async move {
                let r2 = r.clone();
                serve(conn, c.clone(), h, move |sent: u64| {
                    r2.lock().unwrap_or_else(PoisonError::into_inner).push(sent);
                    c.load(Ordering::SeqCst)
                })
                .await;
            });
        }
    });
    Edge {
        count,
        hash,
        resumptions,
    }
}

/// The client of the test on `ports`, with the settings of a rotation.
async fn client(
    wire: Arc<Wire>,
    schedule: Schedule,
    provider: Arc<rustls::crypto::CryptoProvider>,
    roots: Arc<rustls::RootCertStore>,
) -> (noq::Endpoint, Arc<Resume>, Arc<Ports>) {
    let ports = Ports::new(wire, 40100);
    let first = ports.next().await.unwrap();
    let Local::Datagrams(first) = first else {
        unreachable!()
    };
    let endpoint =
        pod_quic::endpoint::over_with(migrate::endpoint_config(schedule).unwrap(), first, None)
            .unwrap();
    let tls = pod_quic::connect::tls(provider, roots, &[ALPN]).unwrap();
    let resume = Arc::new(Resume {
        endpoint: endpoint.clone(),
        tls,
        settings: pod_quic::Settings::default(),
        door: addr(40000),
    });
    (endpoint, resume, ports)
}

#[tokio::test(start_paused = true)]
async fn ten_minutes_and_two_rotations_lose_no_byte() {
    let (provider, crypto, roots) = crypto();
    let wire = Arc::new(Wire::default());
    let edge = edge(wire.clone(), crypto).await;
    let schedule = Schedule::new(Duration::from_secs(180), Duration::from_secs(240), 100).unwrap();
    let (endpoint, resume, ports) = client(wire.clone(), schedule, provider, roots).await;
    let opens = Arc::new(AtomicU64::new(0));
    let mut session = Session::start(resume, opens.clone()).await.unwrap();
    let migrator = Migrator::new(schedule).unwrap();
    let start = tokio::time::Instant::now();
    let mut next = migrator.interval().unwrap();
    let mut rotations = 0;
    // The transfer: 8 KiB each 4 s of simulated time, for 10 minutes.
    while start.elapsed() < Duration::from_secs(600) {
        let wait = next
            .saturating_sub(start.elapsed())
            .min(Duration::from_secs(4));
        tokio::time::sleep(wait).await;
        if start.elapsed() >= next {
            match migrator
                .rotate(&endpoint, ports.as_ref(), &mut session)
                .await
                .unwrap()
            {
                Outcome::Migrated => {}
                Outcome::Reconnected => panic!("the peer accepted every new path"),
            }
            rotations += 1;
            next += migrator.interval().unwrap();
        }
        session.data(8 * 1024).await.unwrap();
    }
    let elapsed = start.elapsed();
    let received = session.count().await.unwrap();
    assert_eq!(
        received, session.sent,
        "the peer has each byte that the client sent"
    );
    let want: Vec<u8> = (0..received).map(byte_at).collect();
    let got = edge
        .hash
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
        .finalize();
    assert_eq!(
        got.as_slice(),
        Sha256::digest(&want).as_slice(),
        "the bytes of the peer are the bytes of the client"
    );
    assert_eq!(edge.count.load(Ordering::SeqCst), received);
    assert!(elapsed >= Duration::from_secs(600), "{elapsed:?}");
    assert!(rotations >= 2, "{rotations} rotations");
    assert_eq!(
        opens.load(Ordering::SeqCst),
        1,
        "one connection carried each rotation"
    );
    assert_eq!(
        edge.resumptions.lock().unwrap().len(),
        1,
        "the session did not resume"
    );
    // Two batches of local connection IDs were retired and issued again.
    let stats = session.conn.stats();
    assert!(
        stats.frame_tx.new_connection_id >= 10,
        "{:?}",
        stats.frame_tx
    );
    assert!(
        stats.frame_rx.retire_connection_id >= 10,
        "{:?}",
        stats.frame_rx
    );
    assert_eq!(wire.dropped(), 0, "the switch dropped no datagram");
}

#[tokio::test(start_paused = true)]
async fn a_peer_that_refuses_migration_gets_a_new_connection() {
    let (provider, crypto, roots) = crypto();
    let wire = Arc::new(Wire::default());
    let edge = edge(wire.clone(), crypto).await;
    let schedule = Schedule::new(Duration::from_secs(4), Duration::from_secs(6), 100).unwrap();
    let (endpoint, resume, ports) = client(wire.clone(), schedule, provider, roots).await;
    let opens = Arc::new(AtomicU64::new(0));
    let mut session = Session::start(resume, opens.clone()).await.unwrap();
    let migrator = Migrator::new(schedule)
        .unwrap()
        .with_probe_wait(Duration::from_millis(600));
    // The data before the rotation, and the refusal of the new path that a
    // peer answers for two seconds.
    for _ in 0..4 {
        session.data(4096).await.unwrap();
    }
    let before = session.count().await.unwrap();
    wire.refuse_migration(addr(40000), Duration::from_secs(2));
    let outcome = migrator
        .rotate(&endpoint, ports.as_ref(), &mut session)
        .await
        .unwrap();
    assert_eq!(
        outcome,
        Outcome::Reconnected,
        "the peer refused the migrated path"
    );
    // The session continues from the byte where the peer stopped, on a new
    // connection, and each byte arrives.
    session.data(4096).await.unwrap();
    let after = session.count().await.unwrap();
    assert_eq!(after, session.sent);
    assert!(after >= before + 4096, "{after} < {before} + 4096");
    assert_eq!(opens.load(Ordering::SeqCst), 2, "one new connection");
    let resumed = edge
        .resumptions
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    assert_eq!(
        resumed,
        [0, before],
        "the new connection resumed at the offset of the peer"
    );
    let want: Vec<u8> = (0..after).map(byte_at).collect();
    let got = edge
        .hash
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
        .finalize();
    assert_eq!(
        got.as_slice(),
        Sha256::digest(&want).as_slice(),
        "no byte lost and no byte twice"
    );
    assert!(
        wire.dropped() > 0,
        "the peer dropped the datagrams of the new path"
    );
}
