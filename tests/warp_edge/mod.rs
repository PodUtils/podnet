//! A stand-in of WARP's edge on the loopback, from podssh's stand-in edge:
//! TLS 1.3 with ALPN `h2`, a certificate of its own key, which a device
//! pins, and a client's certificate required, whose key must be one that
//! the stand-in API enrolled, else the alert `access_denied`, as the edge
//! refuses a key that it does not know. The HTTP/2 and capsule part is
//! `pod_testkit::masque`. Inside, a userspace stack of its own answers at
//! each address (any IP): each route's port is carried to a service of the
//! loopback, as Cloudflare's network carries a connection to its target;
//! port 53 answers a DNS question over TCP and over UDP with [`NAME_IP`];
//! UDP port 7 echoes; a UDP route carries a port to a UDP service of the
//! loopback ([`Edge::start_with_udp`]). A test can change the key that it shows, as WARP's
//! front door showed a new device another key than its record's, end each
//! open session ([`Edge::end_sessions`]), and read what the far end saw
//! ([`Watch`]).

use std::collections::hash_map::Entry;
use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use base64::Engine as _;
use pod_netstack::{NetStack, StackConfig, Subnet};
use pod_testkit::masque::{Far, Seen};
use pod_testkit::stand::warp::Api as ApiStandIn;
use pod_warp::key::DeviceKey;
use rustls::crypto::WebPkiSupportedAlgorithms;
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use rustls::{CertificateError, DigitallySignedStruct, DistinguishedName, Error, SignatureScheme};
use rustls_pki_types::{CertificateDer, UnixTime};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::task::AbortHandle;

/// The address that the DNS of the far end gives each name.
pub const NAME_IP: Ipv4Addr = Ipv4Addr::new(198, 51, 100, 7);
/// The UDP echo port of the far end.
pub const ECHO_PORT: u16 = 7;

/// A connection inside the tunnel to each address on `port`, carried to
/// `outside` on the loopback.
#[derive(Debug, Clone, Copy)]
pub struct Route {
    pub port: u16,
    pub outside: SocketAddr,
}

/// What the far end of each tunnel saw, and the questions that it drops.
#[derive(Debug, Default)]
pub struct Watch {
    /// The DNS questions on UDP port 53.
    pub dns_questions: AtomicUsize,
    /// The DNS questions on UDP to drop first, with no answer.
    pub dns_drop: AtomicUsize,
    /// The largest TCP window field of a packet of the client.
    pub window_max: AtomicUsize,
    /// The source and the destination of each UDP datagram of the client.
    pub udp: Mutex<Vec<(IpAddr, SocketAddr)>>,
    /// The packets of the client of each family: IPv4, IPv6.
    pub families: [AtomicUsize; 2],
}

impl Watch {
    /// Note the family of a packet of the client, the ends of a UDP
    /// datagram, and the TCP window of an IPv4 packet.
    fn note(&self, packet: &[u8]) {
        match packet.first().map(|b| b >> 4) {
            Some(4) => self.families[0].fetch_add(1, Ordering::SeqCst),
            _ => self.families[1].fetch_add(1, Ordering::SeqCst),
        };
        if let Some(ends) = udp_ends(packet) {
            self.udp
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(ends);
        }
        let header = usize::from(packet.first().map_or(0, |b| b & 0x0f)) * 4;
        if packet.first().map(|b| b >> 4) == Some(4)
            && packet.get(9) == Some(&6)
            && packet.len() >= header + 20
        {
            let window = u16::from_be_bytes([packet[header + 14], packet[header + 15]]);
            self.window_max
                .fetch_max(usize::from(window), Ordering::SeqCst);
        }
    }

    /// True when the next DNS question on UDP gets no answer.
    fn drops_question(&self) -> bool {
        self.dns_questions.fetch_add(1, Ordering::SeqCst);
        self.dns_drop
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok()
    }
}

/// The source and the destination of the UDP datagram of an IPv4 or an
/// IPv6 packet with no extension header.
fn udp_ends(packet: &[u8]) -> Option<(IpAddr, SocketAddr)> {
    let (from, to, udp): (IpAddr, IpAddr, &[u8]) = match packet.first()? >> 4 {
        4 if *packet.get(9)? == 17 => {
            let header = usize::from(packet[0] & 0x0f) * 4;
            let from: [u8; 4] = packet.get(12..16)?.try_into().ok()?;
            let to: [u8; 4] = packet.get(16..20)?.try_into().ok()?;
            (from.into(), to.into(), packet.get(header..)?)
        }
        6 if *packet.get(6)? == 17 => {
            let from: [u8; 16] = packet.get(8..24)?.try_into().ok()?;
            let to: [u8; 16] = packet.get(24..40)?.try_into().ok()?;
            (from.into(), to.into(), packet.get(40..)?)
        }
        _ => return None,
    };
    let port = u16::from_be_bytes([*udp.get(2)?, *udp.get(3)?]);
    Some((from, SocketAddr::new(to, port)))
}

/// The keys that the stand-in shows, one for each TLS handshake in turn,
/// the last one kept for each after it.
#[derive(Debug)]
struct Shown {
    keys: Mutex<VecDeque<(DeviceKey, Arc<CertifiedKey>)>>,
    hellos: AtomicUsize,
}

impl Shown {
    fn set(&self, keys: Vec<DeviceKey>) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let certified = keys.into_iter().map(|key| {
            let certified = Arc::new(key.certified(now - 60));
            (key, certified)
        });
        *self.keys.lock().unwrap_or_else(PoisonError::into_inner) = certified.collect();
    }

    fn last(&self) -> DeviceKey {
        self.keys
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .back()
            .map(|(key, _)| key.clone())
            .unwrap()
    }
}

impl ResolvesServerCert for Shown {
    fn resolve(&self, _hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        self.hellos.fetch_add(1, Ordering::SeqCst);
        let mut keys = self.keys.lock().unwrap_or_else(PoisonError::into_inner);
        if keys.len() > 1 {
            keys.pop_front().map(|(_, certified)| certified)
        } else {
            keys.front().map(|(_, certified)| certified.clone())
        }
    }
}

/// The check of a client's key: one that the stand-in API enrolled.
#[derive(Debug)]
struct Enrolled {
    api: ApiStandIn,
    algorithms: WebPkiSupportedAlgorithms,
}

impl ClientCertVerifier for Enrolled {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, Error> {
        let spki = pod_warp::der::spki_of_certificate(end_entity);
        let spki = spki.map(|s| base64::engine::general_purpose::STANDARD.encode(s));
        match spki {
            Some(spki) if self.api.enrolled().contains(&spki) => {
                Ok(ClientCertVerified::assertion())
            }
            // rustls answers this with the alert access_denied.
            _ => Err(Error::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure,
            )),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

/// The stand-in, which serves until it is dropped.
pub struct Edge {
    pub addr: SocketAddr,
    pub seen: Arc<Seen>,
    pub watch: Arc<Watch>,
    shown: Arc<Shown>,
    /// A second handle of the TCP connection of each session, for
    /// [`Edge::end_sessions`].
    sessions: Arc<Mutex<Vec<std::net::TcpStream>>>,
    task: AbortHandle,
}

impl Drop for Edge {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Edge {
    /// A stand-in on 127.0.0.1 that takes the keys that `api` enrolled,
    /// with `routes` inside.
    pub async fn start(api: ApiStandIn, routes: Vec<Route>) -> Edge {
        Edge::start_with_udp(api, routes, vec![]).await
    }

    /// [`Edge::start`] with the UDP routes `udp` inside.
    pub async fn start_with_udp(api: ApiStandIn, routes: Vec<Route>, udp: Vec<Route>) -> Edge {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let verifier = Arc::new(Enrolled {
            api,
            algorithms: provider.signature_verification_algorithms,
        });
        let shown = Arc::new(Shown {
            keys: Mutex::new(VecDeque::new()),
            hellos: AtomicUsize::new(0),
        });
        shown.set(vec![DeviceKey::generate().unwrap()]);
        let mut config = rustls::ServerConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS13])
            .unwrap()
            .with_client_cert_verifier(verifier)
            .with_cert_resolver(shown.clone());
        config.alpn_protocols = vec![b"h2".to_vec()];
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(Seen::default());
        let watch = Arc::new(Watch::default());
        let (routes, counted, watched) = (Arc::new((routes, udp)), seen.clone(), watch.clone());
        let sessions = Arc::new(Mutex::new(Vec::new()));
        let served = sessions.clone();
        let task = tokio::spawn(async move {
            while let Ok((tcp, _)) = listener.accept().await {
                let (acceptor, routes, seen) = (acceptor.clone(), routes.clone(), counted.clone());
                let watch = watched.clone();
                let Ok(tcp) = tcp.into_std() else { continue };
                let Ok(second) = tcp.try_clone() else {
                    continue;
                };
                served
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(second);
                let Ok(tcp) = tokio::net::TcpStream::from_std(tcp) else {
                    continue;
                };
                tokio::spawn(async move {
                    let Ok(tls) = acceptor.accept(tcp).await else {
                        return;
                    };
                    let (to_far, from_client) = mpsc::channel(256);
                    let (to_client, from_far) = mpsc::channel(256);
                    let far = tokio::spawn(far_end(
                        Ends {
                            from_client,
                            to_client,
                        },
                        routes,
                        watch,
                    ));
                    pod_testkit::masque::serve(tls, Far { to_far, from_far }, seen).await;
                    far.abort();
                });
            }
        });
        Edge {
            addr,
            seen,
            watch,
            shown,
            sessions,
            task: task.abort_handle(),
        }
    }

    /// End each session that the stand-in serves now: its TCP connection
    /// closes with no TLS alert, as a front door that drops a session.
    pub fn end_sessions(&self) {
        for session in self
            .sessions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .drain(..)
        {
            let _ = session.shutdown(std::net::Shutdown::Both);
        }
    }

    /// Stop the stand-in: it accepts no new connection (a connect gets a
    /// refusal), and each session that it serves ends.
    pub fn stop(&self) {
        self.task.abort();
        self.end_sessions();
    }

    /// The `PUBLIC KEY` PEM of the key that the stand-in shows last.
    pub fn pem(&self) -> String {
        pod_warp::der::pem(&self.shown.last().spki(), "PUBLIC KEY")
    }

    /// Show `keys` from now on, one for each TLS handshake in turn, the last
    /// one for each after it.
    pub fn show(&self, keys: Vec<DeviceKey>) {
        self.shown.set(keys);
    }

    /// The TLS handshakes that asked for the stand-in's certificate.
    pub fn hellos(&self) -> usize {
        self.shown.hellos.load(Ordering::SeqCst)
    }

    pub fn tunnels(&self) -> usize {
        self.seen.tunnels.load(Ordering::SeqCst)
    }
}

/// The packets of one tunnel, each way.
struct Ends {
    from_client: mpsc::Receiver<Vec<u8>>,
    to_client: mpsc::Sender<Vec<u8>>,
}

/// The far end of one tunnel: a stack that takes each address.
async fn far_end(ends: Ends, routes: Arc<(Vec<Route>, Vec<Route>)>, watch: Arc<Watch>) {
    let Ends {
        mut from_client,
        to_client,
    } = ends;
    let config = StackConfig {
        subnet: Subnet {
            v4: Ipv4Addr::new(198, 18, 0, 0),
            v4_prefix: 0,
            v6: Ipv6Addr::new(0xfd00, 0x66, 0, 0, 0, 0, 0, 0),
            v6_prefix: 0,
        },
        local_v4: Ipv4Addr::new(198, 18, 0, 1),
        local_v6: Ipv6Addr::new(0xfd00, 0x66, 0, 0, 0, 0, 0, 1),
        any_ip: true,
        ..StackConfig::default()
    };
    let stack = NetStack::over_link(config);
    let (link_in, link_out) = (stack.link_in(), stack.link_out());
    let up = tokio::spawn(async move {
        loop {
            let packet = link_out.recv().await;
            if to_client.send(packet).await.is_err() {
                return;
            }
        }
    });
    let mut tasks = vec![up];
    for route in routes.1.iter().copied() {
        let socket = stack.bind_udp_any(route.port).await.unwrap();
        tasks.push(tokio::spawn(udp_route(socket, route.outside)));
    }
    for route in routes.0.iter().copied() {
        let mut listener = stack.listen_any(route.port).await.unwrap();
        tasks.push(tokio::spawn(async move {
            while let Ok(mut inside) = listener.accept().await {
                tokio::spawn(async move {
                    let Ok(mut outside) = tokio::net::TcpStream::connect(route.outside).await
                    else {
                        return;
                    };
                    let _ = tokio::io::copy_bidirectional(&mut inside, &mut outside).await;
                });
            }
        }));
    }
    let mut dns = stack.listen_any(53).await.unwrap();
    tasks.push(tokio::spawn(async move {
        while let Ok(mut stream) = dns.accept().await {
            tokio::spawn(async move {
                let mut len = [0u8; 2];
                if stream.read_exact(&mut len).await.is_err() {
                    return;
                }
                let mut query = vec![0u8; usize::from(u16::from_be_bytes(len))];
                if stream.read_exact(&mut query).await.is_err() {
                    return;
                }
                let answer = dns_answer(&query);
                let mut framed = (answer.len() as u16).to_be_bytes().to_vec();
                framed.extend_from_slice(&answer);
                let _ = stream.write_all(&framed).await;
            });
        }
    }));
    let mut udp_dns = stack.bind_udp_any(53).await.unwrap();
    let asked = watch.clone();
    tasks.push(tokio::spawn(async move {
        let mut buf = vec![0u8; 2048];
        while let Ok((n, from, to)) = udp_dns.recv_from_to(&mut buf).await {
            if !asked.drops_question() {
                let _ = udp_dns
                    .send_to_from(&dns_answer(&buf[..n]), from, to.ip())
                    .await;
            }
        }
    }));
    let mut echo = stack.bind_udp_any(ECHO_PORT).await.unwrap();
    tasks.push(tokio::spawn(async move {
        let mut buf = vec![0u8; 2048];
        while let Ok((n, from, to)) = echo.recv_from_to(&mut buf).await {
            let _ = echo.send_to_from(&buf[..n], from, to.ip()).await;
        }
    }));
    while let Some(packet) = from_client.recv().await {
        watch.note(&packet);
        link_in.push(packet);
    }
    for task in tasks {
        task.abort();
    }
}

/// The tasks of a UDP route, aborted with it.
#[derive(Default)]
struct Readers(Vec<AbortHandle>);

impl Drop for Readers {
    fn drop(&mut self) {
        for reader in &self.0 {
            reader.abort();
        }
    }
}

/// Carry each datagram to the port of `inside` to `outside`, from a socket
/// of the loopback for each sender, and each answer back from the address
/// that the sender asked.
async fn udp_route(mut inside: pod_netstack::UdpSocket, outside: SocketAddr) {
    let (answers, mut back) = mpsc::channel::<(Vec<u8>, SocketAddr, IpAddr)>(256);
    let mut out: HashMap<(SocketAddr, IpAddr), Arc<tokio::net::UdpSocket>> = HashMap::new();
    let mut readers = Readers::default();
    let mut buf = vec![0u8; 2048];
    loop {
        tokio::select! {
            got = inside.recv_from_to(&mut buf) => {
                let Ok((n, from, to)) = got else { return };
                let key = (from, to.ip());
                if let Entry::Vacant(slot) = out.entry(key) {
                    let Ok(socket) = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await else { continue };
                    if socket.connect(outside).await.is_err() {
                        continue;
                    }
                    let socket = Arc::new(socket);
                    let (reader, answers) = (socket.clone(), answers.clone());
                    let task = tokio::spawn(async move {
                        let mut buf = vec![0u8; 2048];
                        while let Ok(n) = reader.recv(&mut buf).await {
                            if answers.send((buf[..n].to_vec(), key.0, key.1)).await.is_err() {
                                return;
                            }
                        }
                    });
                    readers.0.push(task.abort_handle());
                    slot.insert(socket);
                }
                let _ = out[&key].send(&buf[..n]).await;
            }
            Some((data, to, source)) = back.recv() => {
                let _ = inside.send_to_from(&data, to, source).await;
            }
        }
    }
}

/// The answer to a DNS question: [`NAME_IP`] for an A question, no address
/// for another. A datagram that holds no question gets an empty answer, so
/// that the far end does not stop.
pub fn dns_answer(query: &[u8]) -> Vec<u8> {
    question_answer(query).unwrap_or_default()
}

fn question_answer(query: &[u8]) -> Option<Vec<u8>> {
    // The question ends after the name (labels to a zero byte) and the type
    // and the class.
    let mut end = 12;
    while *query.get(end)? != 0 {
        end += 1 + usize::from(query[end]);
    }
    let qtype = u16::from_be_bytes([*query.get(end + 1)?, *query.get(end + 2)?]);
    let end = end + 5;
    let a = qtype == 1;
    let mut out = query[..2].to_vec();
    out.extend_from_slice(&[0x81, 0x80, 0, 1, 0, u8::from(a), 0, 0, 0, 0]);
    out.extend_from_slice(query.get(12..end)?);
    if a {
        out.extend_from_slice(&[0xc0, 0x0c, 0, 1, 0, 1, 0, 0, 0, 60, 0, 4]);
        out.extend_from_slice(&NAME_IP.octets());
    }
    Some(out)
}

/// An echo server on the loopback: the address of a route.
pub async fn echo_server() -> SocketAddr {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut tcp, _)) = listener.accept().await {
            tokio::spawn(async move {
                let (mut read, mut write) = tcp.split();
                let _ = tokio::io::copy(&mut read, &mut write).await;
            });
        }
    });
    addr
}

/// The address of the record's IPv6 door, which the moved dial carries to
/// the edge.
pub fn door_v6() -> SocketAddr {
    SocketAddr::new(
        IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1)),
        443,
    )
}

/// The address of the record's IPv4 door.
pub fn door_v4() -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)), 443)
}
