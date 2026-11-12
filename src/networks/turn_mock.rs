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

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};

use pod_turn::stun::{self, message_type, Class};
use tokio::net::UdpSocket;

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
}

pub type Shared = Arc<Mutex<Control>>;

pub struct Mock {
    pub addr: SocketAddr,
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
    let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
    let addr = socket.local_addr().unwrap();
    let control: Shared = Arc::default();
    tokio::spawn(serve(socket, control.clone()));
    Mock { addr, control }
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
