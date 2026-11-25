#![cfg(test)]
//! A stand-in TURN server on the loopback for the tests of `turn://`
//! (T-321). It reads RFC 8656 over UDP for one user: the 401 challenge with
//! a realm and a nonce, the 438 on request of a test, Allocate, Refresh,
//! CreatePermission, ChannelBind, ChannelData to a relay socket on the
//! loopback, and the data of a peer back to the client in ChannelData. It
//! uses the codec of `pod-turn` and no TURN client code. No account and no
//! public server: the user, the password and the realm are made-up values.
//!
//! The file is used by the unit tests (`networks::turn_tests`) and by the
//! test of the command (`tests/turn_network.rs`, by `#[path]`).
#![allow(dead_code)]

use podnet_core::state::temp;
use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use pod_link::BoxIo;
use pod_tls::server::TlsAcceptor;
use pod_turn::stun::{self, message_type, Class};
use rustls::crypto::CryptoProvider;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};

/// The user of the stand-in.
pub const USER: &str = "turn-user";
/// The password of the stand-in (a made-up value).
pub const PASSWORD: &str = "turn-secret-for-tests";
/// The realm of the stand-in.
pub const REALM: &str = "turn.test";

/// What a test changes while the server runs, and what it saw.
#[derive(Default)]
pub struct Control {
    /// The next authenticated requests (not Allocate) to answer with 438
    /// and a fresh nonce.
    pub stale_next: u32,
    /// Answer each Refresh that asks for a lifetime over 0 with this code.
    pub refresh_code: Option<u16>,
    /// Answer each ChannelBind with this code.
    pub bind_code: Option<u16>,
    /// The requests that it read, in order: the method and the code that it
    /// answered (0 for a success).
    pub seen: Vec<(u16, u16)>,
    /// The ChannelData of clients that it relayed to peers.
    pub channel_data: u32,
    /// The TCP allocations that a Refresh with lifetime 0 ended.
    pub releases: u32,
    /// The control connections that ended while their allocation lived
    /// (the aborted keeper of a dropped stream).
    pub closed: u32,
}

pub type Shared = Arc<Mutex<Control>>;

pub struct Mock {
    pub addr: SocketAddr,
    /// The plain-TCP face of RFC 6062: the same user, realm and password.
    pub tcp: SocketAddr,
    /// The TLS face of `turns://`: the same user, realm and password.
    pub tls: SocketAddr,
    pub control: Shared,
}

impl Mock {
    /// The methods read, with the code of the answer.
    pub fn seen(&self) -> Vec<(u16, u16)> {
        self.control.lock().unwrap().seen.clone()
    }

    /// How many requests of `method` were answered with `code`.
    pub fn count(&self, method: u16, code: u16) -> usize {
        self.seen().iter().filter(|s| **s == (method, code)).count()
    }

    /// How many TCP allocations ended by a Refresh with lifetime 0.
    pub fn releases(&self) -> u32 {
        self.control.lock().unwrap().releases
    }

    /// How many control connections of a live allocation ended.
    pub fn closed(&self) -> u32 {
        self.control.lock().unwrap().closed
    }
}

/// The certificate of the TLS face of the stand: a CA and a leaf for
/// `localhost` and `127.0.0.1` that it signs. It authenticates nothing
/// beyond this process.
pub struct Certificates {
    pub leaf: String,
    pub key: String,
    pub ca: String,
}

/// The certificate of the stand, made once.
pub fn certificates() -> &'static Certificates {
    use std::sync::OnceLock;

    use rcgen::{BasicConstraints, CertificateParams, ExtendedKeyUsagePurpose, IsCa, KeyPair};
    static MADE: OnceLock<Certificates> = OnceLock::new();
    MADE.get_or_init(|| {
        let ca_key = KeyPair::generate().expect("a key of the CA of the TURN stand");
        let mut ca_params = CertificateParams::new(vec!["podnet TURN stand CA".to_string()])
            .expect("the CA parameters");
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let ca = ca_params.self_signed(&ca_key).expect("the CA of the stand");
        let issuer = rcgen::Issuer::new(ca_params, ca_key);
        let leaf_key = KeyPair::generate().expect("a key of the leaf of the TURN stand");
        let mut leaf_params =
            CertificateParams::new(vec!["localhost".to_string(), "127.0.0.1".to_string()])
                .expect("the leaf parameters");
        leaf_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        let leaf = leaf_params
            .signed_by(&leaf_key, &issuer)
            .expect("the leaf of the stand");
        Certificates {
            leaf: leaf.pem(),
            key: leaf_key.serialize_pem(),
            ca: ca.pem(),
        }
    })
}

/// The provider of the TLS of the stand.
pub fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

fn acceptor() -> TlsAcceptor {
    let made = certificates();
    pod_tls::server::acceptor(provider(), made.leaf.as_bytes(), made.key.as_bytes(), &[])
        .expect("the certificate of the TURN stand")
}

/// The trust of the client of `turns://`: the CA of the stand alone. The
/// CA goes to a file beside the temporary files of this process, because
/// the context of the trust reads a file.
pub fn client_context() -> Arc<pod_tls::Context> {
    // T-545: one random name for each process (not the pid).
    static PATH: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    let at = PATH.get_or_init(|| temp::random_path(&std::env::temp_dir(), "pn-ca-", ".pem"));
    let path = at.clone();
    std::fs::write(at, certificates().ca.as_bytes()).expect("the CA of the TURN stand");
    let mut settings = pod_tls::Settings::new(pod_tls::Words::new("turn stand"));
    settings.ca_file = Some(pod_tls::CaFile::new("the CA of the TURN stand", path));
    Arc::new(pod_tls::Context::new(provider(), settings))
}

/// The state of an allocation, shared with the task of its relay socket.
struct Alloc {
    relay: Arc<UdpSocket>,
    permissions: HashSet<IpAddr>,
    channels: HashMap<u16, SocketAddr>,
}

#[derive(Default)]
struct Session {
    nonce: Vec<u8>,
    nonces: u32,
    alloc: Option<Arc<Mutex<Alloc>>>,
}

fn answer(method: u16, class: Class, id: &[u8; 12], attrs: &[(u16, &[u8])]) -> Vec<u8> {
    stun::message(message_type(method, class), id, attrs)
}

fn error(
    method: u16,
    id: &[u8; 12],
    code: u16,
    more: &[(u16, &[u8])],
    key: Option<&[u8]>,
) -> Vec<u8> {
    let value = [
        [0, 0, (code / 100) as u8, (code % 100) as u8].as_slice(),
        b"mock".as_slice(),
    ]
    .concat();
    let mut attrs = vec![(stun::ERROR_CODE, value.as_slice())];
    attrs.extend_from_slice(more);
    let bytes = answer(method, Class::Error, id, &attrs);
    key.map_or(bytes.clone(), |key| stun::signed(&bytes, key))
}

/// Start the server on 127.0.0.1 with a port of the system.
pub async fn start() -> Mock {
    start_on("127.0.0.1:0".parse().unwrap()).await
}

/// Start the server on `bind` with the system's port when it is 0. The
/// address is the namespace of the caller: the test of two namespaces binds
/// the stand on the far side of its veth (GitHub #485).
pub async fn start_on(bind: SocketAddr) -> Mock {
    // The TCP face takes the same address: UDP and TCP are separate
    // spaces, so one `turn://HOST:PORT` names both faces. A chosen UDP
    // port can already be in use for TCP, so an address that asks for a
    // port of its own takes another one until both faces bind.
    let (socket, addr, listener) = loop {
        let socket = UdpSocket::bind(bind).await.unwrap();
        let addr = socket.local_addr().unwrap();
        match TcpListener::bind(addr).await {
            Ok(listener) => break (socket, addr, listener),
            Err(e) if bind.port() == 0 && e.kind() == std::io::ErrorKind::AddrInUse => continue,
            Err(e) => panic!("the TCP face of the TURN stand at {addr}: {e}"),
        }
    };
    let socket = Arc::new(socket);
    let control: Shared = Arc::default();
    tokio::spawn(serve(socket, control.clone()));
    tokio::spawn(serve_tcp(listener, control.clone()));
    // The TLS face takes a port of its own: `turns://127.0.0.1:PORT`.
    let tls_bind = if bind.ip().is_loopback() {
        "127.0.0.1:0".parse().unwrap()
    } else {
        SocketAddr::new(bind.ip(), 0)
    };
    let tls_listener = TcpListener::bind(tls_bind).await.unwrap();
    let tls = tls_listener.local_addr().unwrap();
    tokio::spawn(serve_tls(tls_listener, control.clone()));
    Mock {
        addr,
        tcp: addr,
        tls,
        control,
    }
}

async fn serve(socket: Arc<UdpSocket>, control: Shared) {
    let mut sessions: HashMap<SocketAddr, Session> = HashMap::new();
    let mut buf = vec![0u8; 65_536];
    while let Ok((n, client)) = socket.recv_from(&mut buf).await {
        let bytes = &buf[..n];
        let session = sessions.entry(client).or_default();
        if bytes.first().is_some_and(|b| b & 0xc0 == 0x40) {
            relay_channel_data(session, bytes, &control).await;
            continue;
        }
        if let Some(reply) = request(&socket, &control, session, client, bytes).await {
            let _ = socket.send_to(&reply, client).await;
        }
    }
}

/// ChannelData of the client: to the peer of the channel.
async fn relay_channel_data(session: &Session, bytes: &[u8], control: &Shared) {
    let (Some(alloc), true) = (&session.alloc, bytes.len() >= 4) else {
        return;
    };
    let number = u16::from_be_bytes([bytes[0], bytes[1]]);
    let len = usize::from(u16::from_be_bytes([bytes[2], bytes[3]]));
    let Some(data) = bytes.get(4..4 + len) else {
        return;
    };
    let (relay, peer) = {
        let alloc = alloc.lock().unwrap();
        (alloc.relay.clone(), alloc.channels.get(&number).copied())
    };
    if let Some(peer) = peer {
        control.lock().unwrap().channel_data += 1;
        let _ = relay.send_to(data, peer).await;
    }
}

/// What the peers send to `alloc`'s relay socket goes to the client.
fn spawn_relay(socket: Arc<UdpSocket>, alloc: Arc<Mutex<Alloc>>, client: SocketAddr) {
    tokio::spawn(async move {
        let relay = alloc.lock().unwrap().relay.clone();
        let mut buf = vec![0u8; 65_536];
        while let Ok((n, peer)) = relay.recv_from(&mut buf).await {
            let channel = {
                let alloc = alloc.lock().unwrap();
                if !alloc.permissions.contains(&peer.ip()) {
                    continue;
                }
                alloc
                    .channels
                    .iter()
                    .find(|(_, p)| **p == peer)
                    .map(|(c, _)| *c)
            };
            let Some(channel) = channel else { continue };
            let mut out = [channel.to_be_bytes(), (n as u16).to_be_bytes()].concat();
            out.extend_from_slice(&buf[..n]);
            let _ = socket.send_to(&out, client).await;
        }
    });
}

fn fresh_nonce(session: &mut Session) -> Vec<u8> {
    session.nonces += 1;
    session.nonce = format!("mock-nonce-{}", session.nonces).into_bytes();
    session.nonce.clone()
}

fn challenge(session: &mut Session, method: u16, id: &[u8; 12], code: u16) -> Vec<u8> {
    let nonce = fresh_nonce(session);
    error(
        method,
        id,
        code,
        &[(stun::REALM, REALM.as_bytes()), (stun::NONCE, &nonce)],
        None,
    )
}

async fn request(
    socket: &Arc<UdpSocket>,
    control: &Shared,
    session: &mut Session,
    client: SocketAddr,
    bytes: &[u8],
) -> Option<Vec<u8>> {
    let read = stun::parse(bytes).ok()?;
    let (method, id) = (stun::method_of(read.kind), read.id);
    if stun::class_of(read.kind) != Class::Request {
        return None;
    }
    let note = |code: u16| control.lock().unwrap().seen.push((method, code));
    let user = read.attribute(stun::USERNAME);
    if user.is_none() || read.attribute(stun::MESSAGE_INTEGRITY).is_none() {
        note(401);
        return Some(challenge(session, method, &id, 401));
    }
    if read.attribute(stun::NONCE) != Some(session.nonce.as_slice()) {
        note(438);
        return Some(challenge(session, method, &id, 438));
    }
    let key = stun::long_term_key(USER, REALM, PASSWORD);
    if user != Some(USER.as_bytes()) || stun::integrity(bytes, &key).is_err() {
        note(401);
        return Some(challenge(session, method, &id, 401));
    }
    let stale = {
        let mut control = control.lock().unwrap();
        let stale = control.stale_next > 0 && method != stun::ALLOCATE;
        control.stale_next -= u32::from(stale);
        stale
    };
    if stale {
        note(438);
        return Some(challenge(session, method, &id, 438));
    }
    let (code, reply) = method_reply(socket, control, session, client, &read, &key).await;
    note(code);
    Some(reply)
}

async fn method_reply(
    socket: &Arc<UdpSocket>,
    control: &Shared,
    session: &mut Session,
    client: SocketAddr,
    read: &stun::Message<'_>,
    key: &[u8],
) -> (u16, Vec<u8>) {
    let (method, id) = (stun::method_of(read.kind), read.id);
    let fail = |code: u16| (code, error(method, &id, code, &[], Some(key)));
    let ok = |attrs: &[(u16, &[u8])]| {
        (
            0,
            stun::signed(&answer(method, Class::Success, &id, attrs), key),
        )
    };
    let peer = read
        .attribute(stun::XOR_PEER_ADDRESS)
        .and_then(|v| stun::read_address(v, Some(&id)).ok());
    let asked = read
        .attribute(stun::LIFETIME)
        .map(|v| u32::from_be_bytes([v[0], v[1], v[2], v[3]]));
    match method {
        stun::ALLOCATE => {
            if session.alloc.is_some() {
                return fail(437);
            }
            let relay = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
            let relayed = relay.local_addr().unwrap();
            let alloc = Arc::new(Mutex::new(Alloc {
                relay,
                permissions: HashSet::new(),
                channels: HashMap::new(),
            }));
            spawn_relay(socket.clone(), alloc.clone(), client);
            session.alloc = Some(alloc);
            let (r, m) = (
                stun::write_address(relayed, Some(&id)),
                stun::write_address(client, Some(&id)),
            );
            let life = asked.unwrap_or(600).to_be_bytes();
            ok(&[
                (stun::XOR_RELAYED_ADDRESS, &r),
                (stun::XOR_MAPPED_ADDRESS, &m),
                (stun::LIFETIME, &life),
            ])
        }
        stun::REFRESH => {
            if session.alloc.is_none() {
                return fail(437);
            }
            let lifetime = asked.unwrap_or(600);
            if lifetime == 0 {
                session.alloc = None;
            } else if let Some(code) = control.lock().unwrap().refresh_code {
                return fail(code);
            }
            ok(&[(stun::LIFETIME, &lifetime.to_be_bytes())])
        }
        stun::CREATE_PERMISSION | stun::CHANNEL_BIND => {
            let (Some(alloc), Some(peer)) = (&session.alloc, peer) else {
                return fail(400);
            };
            let mut alloc = alloc.lock().unwrap();
            if method == stun::CHANNEL_BIND {
                if let Some(code) = control.lock().unwrap().bind_code {
                    return fail(code);
                }
                let Some(value) = read.attribute(stun::CHANNEL_NUMBER) else {
                    return fail(400);
                };
                let number = u16::from_be_bytes([value[0], value[1]]);
                if !(0x4000..=0x4fff).contains(&number) {
                    return fail(400);
                }
                alloc.channels.insert(number, peer);
            }
            alloc.permissions.insert(peer.ip());
            ok(&[])
        }
        _ => fail(400),
    }
}

/// A UDP peer on the loopback that answers each datagram with `echo:` and
/// the datagram.
pub async fn echo_peer() -> SocketAddr {
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = socket.local_addr().unwrap();
    tokio::spawn(async move {
        let mut buf = [0u8; 2048];
        while let Ok((n, from)) = socket.recv_from(&mut buf).await {
            let mut back = b"echo:".to_vec();
            back.extend_from_slice(&buf[..n]);
            let _ = socket.send_to(&back, from).await;
        }
    });
    addr
}

// ---------------------------------------------------------------------
// The plain-TCP face of RFC 6062.
// ---------------------------------------------------------------------

/// The state of one TCP control connection of the stand-in.
struct TcpSession {
    nonce: Vec<u8>,
    nonces: u32,
    /// The relayed address of a TCP allocation, and its listener.
    alloc: Option<(SocketAddr, TcpListener)>,
}

/// The relays that wait for a ConnectionBind, by CONNECTION-ID.
type Pending = Arc<Mutex<HashMap<u32, TcpStream>>>;

/// The next CONNECTION-ID.
static CONNECTION: AtomicU32 = AtomicU32::new(100);

async fn serve_tcp(listener: TcpListener, control: Shared) {
    let pending: Pending = Arc::default();
    while let Ok((stream, _)) = listener.accept().await {
        let control = control.clone();
        let pending = pending.clone();
        tokio::spawn(async move {
            serve_tcp_conn(stream, control, pending).await;
        });
    }
}

/// The TLS face of RFC 6062: each connection is wrapped before the codec.
async fn serve_tls(listener: TcpListener, control: Shared) {
    let pending: Pending = Arc::default();
    while let Ok((stream, _)) = listener.accept().await {
        let control = control.clone();
        let pending = pending.clone();
        tokio::spawn(async move {
            let Ok(tls) = acceptor().accept(Box::new(stream) as BoxIo).await else {
                return;
            };
            serve_tcp_conn(Box::new(tls) as BoxIo, control, pending).await;
        });
    }
}

/// One STUN message of a byte stream: the header gives its length. A
/// message that is not whole waits for the next read.
fn take_message(buf: &mut Vec<u8>) -> Option<Vec<u8>> {
    if buf.len() < 4 {
        return None;
    }
    let len = 20 + usize::from(u16::from_be_bytes([buf[2], buf[3]]));
    if buf.len() < len {
        return None;
    }
    let message = buf[..len].to_vec();
    buf.drain(..len);
    Some(message)
}

async fn serve_tcp_conn<S>(mut stream: S, control: Shared, pending: Pending)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut session = TcpSession {
        nonce: Vec::new(),
        nonces: 0,
        alloc: None,
    };
    let mut live = Live {
        control: control.clone(),
        alloc: false,
    };
    let mut buf = Vec::new();
    let mut chunk = vec![0u8; 8192];
    loop {
        while let Some(message) = take_message(&mut buf) {
            let done = tcp_request(
                &mut stream,
                &control,
                &pending,
                &mut session,
                &message,
                &mut live,
            )
            .await;
            if done {
                return;
            }
        }
        // A TCP allocation is a listener: a peer that connects to the
        // relayed address makes a CONNECTION-ATTEMPT (RFC 6062, 4.2).
        if let Some((_, listener)) = session.alloc.as_ref() {
            tokio::select! {
                read = stream.read(&mut chunk) => match read {
                    Ok(0) | Err(_) => return,
                    Ok(n) => buf.extend_from_slice(&chunk[..n]),
                },
                peer = listener.accept() => {
                    let Ok((peer_stream, addr)) = peer else { return };
                    let connection = CONNECTION.fetch_add(1, Ordering::SeqCst);
                    pending.lock().unwrap().insert(connection, peer_stream);
                    let txid = [0u8; 12];
                    let value = connection.to_be_bytes();
                    let peer_value = stun::write_address(addr, Some(&txid));
                    let indication = answer(
                        stun::CONNECTION_ATTEMPT,
                        Class::Indication,
                        &txid,
                        &[
                            (stun::CONNECTION_ID, &value),
                            (stun::XOR_PEER_ADDRESS, &peer_value),
                        ],
                    );
                    let _ = stream.write_all(&indication).await;
                }
            }
        } else {
            match stream.read(&mut chunk).await {
                Ok(0) | Err(_) => return,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
            }
        }
    }
}

/// Answer one request of the TCP face. `true` when this connection became
/// a data connection (RFC 6062, 4.3) and the caller must stop reading it.
async fn tcp_request<S>(
    stream: &mut S,
    control: &Shared,
    pending: &Pending,
    session: &mut TcpSession,
    bytes: &[u8],
    live: &mut Live,
) -> bool
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let Ok(read) = stun::parse(bytes) else {
        return false;
    };
    let (method, id) = (stun::method_of(read.kind), read.id);
    if stun::class_of(read.kind) != Class::Request {
        return false;
    }
    let seen = |code: u16| control.lock().unwrap().seen.push((method, code));
    let key = stun::long_term_key(USER, REALM, PASSWORD);
    let user = read.attribute(stun::USERNAME);
    let signed = read.attribute(stun::MESSAGE_INTEGRITY).is_some()
        && user == Some(USER.as_bytes())
        && stun::integrity(bytes, &key).is_ok();
    if !signed {
        seen(401);
        let nonce = fresh_tcp_nonce(session);
        let reply = error(
            method,
            &id,
            401,
            &[(stun::REALM, REALM.as_bytes()), (stun::NONCE, &nonce)],
            None,
        );
        let _ = stream.write_all(&reply).await;
        return false;
    }
    // A ConnectionBind arrives on a new connection with the nonce of the
    // allocation: the nonce of this connection is not its own.
    if method != stun::CONNECTION_BIND
        && read.attribute(stun::NONCE) != Some(session.nonce.as_slice())
    {
        seen(438);
        let nonce = fresh_tcp_nonce(session);
        let reply = error(
            method,
            &id,
            438,
            &[(stun::REALM, REALM.as_bytes()), (stun::NONCE, &nonce)],
            Some(&key),
        );
        let _ = stream.write_all(&reply).await;
        return false;
    }
    let reply =
        |attrs: &[(u16, &[u8])]| stun::signed(&answer(method, Class::Success, &id, attrs), &key);
    let refuse = |code: u16| error(method, &id, code, &[], Some(&key));
    match method {
        stun::ALLOCATE => {
            if session.alloc.is_some() {
                seen(437);
                let reply = refuse(437);
                let _ = stream.write_all(&reply).await;
                return false;
            }
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let relayed = listener.local_addr().unwrap();
            session.alloc = Some((relayed, listener));
            live.alloc = true;
            let asked = lifetime_of(&read).unwrap_or(600);
            let (r, m) = (
                stun::write_address(relayed, Some(&id)),
                stun::write_address("127.0.0.1:1".parse().unwrap(), Some(&id)),
            );
            let life = asked.to_be_bytes();
            let reply = reply(&[
                (stun::XOR_RELAYED_ADDRESS, &r),
                (stun::XOR_MAPPED_ADDRESS, &m),
                (stun::LIFETIME, &life),
            ]);
            seen(0);
            let _ = stream.write_all(&reply).await;
        }
        stun::REFRESH => {
            if session.alloc.is_none() {
                seen(437);
                let reply = refuse(437);
                let _ = stream.write_all(&reply).await;
                return false;
            }
            let asked = lifetime_of(&read).unwrap_or(600);
            let code = if asked == 0 {
                session.alloc = None;
                live.alloc = false;
                control.lock().unwrap().releases += 1;
                0
            } else {
                control.lock().unwrap().refresh_code.unwrap_or(0)
            };
            if code != 0 {
                seen(code);
                let reply = refuse(code);
                let _ = stream.write_all(&reply).await;
                return false;
            }
            let life = asked.to_be_bytes();
            let reply = reply(&[(stun::LIFETIME, &life)]);
            seen(0);
            let _ = stream.write_all(&reply).await;
        }
        stun::CONNECT => {
            let Some(peer) = read
                .attribute(stun::XOR_PEER_ADDRESS)
                .and_then(|v| stun::read_address(v, Some(&id)).ok())
            else {
                seen(400);
                let reply = refuse(400);
                let _ = stream.write_all(&reply).await;
                return false;
            };
            let Ok(peer_stream) = TcpStream::connect(peer).await else {
                seen(447);
                let reply = refuse(447);
                let _ = stream.write_all(&reply).await;
                return false;
            };
            let connection = CONNECTION.fetch_add(1, Ordering::SeqCst);
            pending.lock().unwrap().insert(connection, peer_stream);
            let value = connection.to_be_bytes();
            let reply = reply(&[(stun::CONNECTION_ID, &value)]);
            seen(0);
            let _ = stream.write_all(&reply).await;
        }
        stun::CONNECTION_BIND => {
            let Some(value) = read.attribute(stun::CONNECTION_ID) else {
                seen(400);
                let reply = refuse(400);
                let _ = stream.write_all(&reply).await;
                return false;
            };
            let Ok(bytes) = <[u8; 4]>::try_from(value) else {
                seen(400);
                let reply = refuse(400);
                let _ = stream.write_all(&reply).await;
                return false;
            };
            let id = u32::from_be_bytes(bytes);
            let Some(mut peer) = pending.lock().unwrap().remove(&id) else {
                seen(400);
                let reply = refuse(400);
                let _ = stream.write_all(&reply).await;
                return false;
            };
            let reply = reply(&[]);
            seen(0);
            let _ = stream.write_all(&reply).await;
            // The data connection: the bytes of the peer and of the client
            // go through unchanged (RFC 6062, 4.5).
            let _ = tokio::io::copy_bidirectional(stream, &mut peer).await;
            return true;
        }
        _ => {
            seen(400);
            let reply = refuse(400);
            let _ = stream.write_all(&reply).await;
        }
    }
    false
}

/// The end of a control connection: an allocation that it held ends.
struct Live {
    control: Shared,
    alloc: bool,
}

impl Drop for Live {
    fn drop(&mut self) {
        if self.alloc {
            self.control.lock().unwrap().closed += 1;
        }
    }
}

fn fresh_tcp_nonce(session: &mut TcpSession) -> Vec<u8> {
    session.nonces += 1;
    session.nonce = format!("mock-tcp-nonce-{}", session.nonces).into_bytes();
    session.nonce.clone()
}

fn lifetime_of(read: &stun::Message<'_>) -> Option<u32> {
    let value = read.attribute(stun::LIFETIME)?;
    let bytes = <[u8; 4]>::try_from(value).ok()?;
    Some(u32::from_be_bytes(bytes))
}

/// A TCP peer on the loopback that answers each line with `echo:` and the
/// line, and keeps the connection open.
pub async fn tcp_echo_peer() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 2048];
                while let Ok(n) = stream.read(&mut buf).await {
                    if n == 0 {
                        return;
                    }
                    let mut back = b"echo:".to_vec();
                    back.extend_from_slice(&buf[..n]);
                    if stream.write_all(&back).await.is_err() {
                        return;
                    }
                }
            });
        }
    });
    addr
}
