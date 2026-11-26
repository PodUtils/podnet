#![cfg(test)]
//! Stand-ins on the loopback for the tests of `vless://`, `ss://` and
//! `wisp:` (T-127): a VLESS server over a WebSocket with TLS, a Shadowsocks
//! server of the AEAD form on TCP, and a Wisp server of version 2 that
//! confirms each stream, over a WebSocket with TLS. They read the bytes of a
//! client as the documents of each protocol give them, and they use the CA
//! of the stands of `road_stand.rs`. No public server and no account: the ID
//! and the password are made-up values.
//!
//! The file is used by the unit tests (`networks::servers`) and by the test of
//! the command (`tests/road_network.rs`, by `#[path]`), each beside
//! `road_stand.rs`.
#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use pod_egress::shadowsocks::{self, codec, Method};
use pod_egress::wisp::packet::{self, ext, Extension, Info, Packet, Reason};
use pod_ws::frame::{self, Frame, Role};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Notify};

use super::road_stand::{self, Seen, Stand};

/// The ID of the VLESS stand: a word, which Xray-core maps to a UUID (a
/// made-up value).
pub const VLESS_ID: &str = "vless-id-for-tests";
/// The path of the WebSocket of the VLESS stand.
pub const VLESS_PATH: &str = "/vless";
/// The method of the Shadowsocks stand.
pub const SS_METHOD: &str = "aes-256-gcm";
/// The password of the Shadowsocks stand (a made-up value).
pub const SS_PASSWORD: &str = "ss-secret-for-tests";
/// The path of the WebSocket of the Wisp stand.
pub const WISP_PATH: &str = "/wisp/";
/// The DATA packets that the Wisp stand buffers for each stream.
const WISP_BUFFER: u32 = 64;

/// A frame of the server with `payload`.
fn server_frame(opcode: u8, payload: Vec<u8>) -> Vec<u8> {
    let frame = Frame {
        fin: true,
        opcode,
        payload,
    };
    frame::encode(&frame, Role::Server, [0; 4])
}

/// The payload of the next data frame of the client; `None` at its end, its
/// Close or a frame that does not decode. `raw` keeps the bytes that were
/// read and not decoded yet.
async fn next_payload<S: AsyncRead + Unpin>(stream: &mut S, raw: &mut Vec<u8>) -> Option<Vec<u8>> {
    let mut chunk = [0u8; 16 * 1024];
    loop {
        match frame::decode(raw, Role::Client) {
            Ok(Some((frame, used))) => {
                raw.drain(..used);
                match frame.opcode {
                    0..=2 => return Some(frame.payload),
                    8 => return None,
                    _ => continue,
                }
            }
            Ok(None) => {}
            Err(_) => return None,
        }
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        raw.extend_from_slice(&chunk[..n]);
    }
}

/// The upgrade of a client at `path`, with the protocol that it offers; a
/// request of another path gets a 404. `None` when it did not upgrade.
async fn upgrade<S: AsyncRead + AsyncWrite + Unpin>(tls: &mut S, path: &str) -> Option<()> {
    let head = road_stand::read_head(tls).await?;
    let asked = head.split_whitespace().nth(1).unwrap_or_default();
    let key = road_stand::header(&head, "sec-websocket-key");
    let (true, Some(key)) = (asked == path, key) else {
        let answer = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n";
        let _ = tls.write_all(answer.as_bytes()).await;
        return None;
    };
    let protocol = road_stand::header(&head, "sec-websocket-protocol")
        .map(|p| {
            format!(
                "Sec-WebSocket-Protocol: {}\r\n",
                p.split(',').next().unwrap_or(p).trim()
            )
        })
        .unwrap_or_default();
    let answer = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n{protocol}\r\n",
        pod_ws::handshake::accept_key(key)
    );
    tls.write_all(answer.as_bytes()).await.ok()
}

/// The length of a whole VLESS request at the start of `bytes`, its ID and
/// its target; `None` until it is whole.
fn vless_request(bytes: &[u8]) -> Option<(usize, [u8; 16], String)> {
    let addons = usize::from(*bytes.get(17)?);
    let at = 18 + addons;
    let port = u16::from_be_bytes([*bytes.get(at + 1)?, *bytes.get(at + 2)?]);
    let (host, end) = match *bytes.get(at + 3)? {
        1 => {
            let b = bytes.get(at + 4..at + 8)?;
            (format!("{}.{}.{}.{}", b[0], b[1], b[2], b[3]), at + 8)
        }
        2 => {
            let len = usize::from(*bytes.get(at + 4)?);
            let name = bytes.get(at + 5..at + 5 + len)?;
            (String::from_utf8_lossy(name).to_string(), at + 5 + len)
        }
        _ => return None,
    };
    let mut id = [0u8; 16];
    id.copy_from_slice(&bytes[1..17]);
    Some((end, id, format!("{host}:{port}")))
}

/// A VLESS server: TLS, the upgrade at [`VLESS_PATH`], then VLESS's request
/// in the first frames. A request of another ID ends the session with no
/// answer, as Xray-core does. The answer (version 0, no add-ons) goes
/// before the bytes of the target.
pub async fn vless() -> Stand {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let seen: Arc<Mutex<Seen>> = Arc::default();
    let (acceptor, record) = (road_stand::acceptor(), seen.clone());
    let id = *pod_egress::vless::Id::parse(VLESS_ID).unwrap().bytes();
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let (acceptor, record) = (acceptor.clone(), record.clone());
            tokio::spawn(async move {
                let Ok(mut tls) = acceptor.accept(tcp).await else {
                    return;
                };
                if upgrade(&mut tls, VLESS_PATH).await.is_none() {
                    return;
                }
                let (mut raw, mut bytes) = (Vec::new(), Vec::new());
                let (used, given, target) = loop {
                    match vless_request(&bytes) {
                        Some(request) => break request,
                        None => match next_payload(&mut tls, &mut raw).await {
                            Some(more) => bytes.extend_from_slice(&more),
                            None => return,
                        },
                    }
                };
                let far = match given == id {
                    true => TcpStream::connect(&target).await.ok(),
                    false => None,
                };
                let Some(mut far) = far else {
                    record.lock().unwrap().refused += 1;
                    let _ = tls
                        .write_all(&server_frame(8, 1000u16.to_be_bytes().to_vec()))
                        .await;
                    return;
                };
                record.lock().unwrap().targets.push(target);
                let answered = tls.write_all(&server_frame(2, vec![0, 0])).await.is_ok();
                if answered && far.write_all(&bytes[used..]).await.is_ok() {
                    road_stand::pump(tls, far, raw).await;
                }
            });
        }
    });
    Stand { addr, seen }
}

/// A Shadowsocks server of the AEAD form ([`SS_METHOD`], [`SS_PASSWORD`])
/// on TCP: the salt, the address in the first chunk, then the bytes in
/// chunks each way. A stream of another key is closed with no answer.
pub async fn shadowsocks() -> Stand {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let seen: Arc<Mutex<Seen>> = Arc::default();
    let record = seen.clone();
    tokio::spawn(async move {
        while let Ok((client, _)) = listener.accept().await {
            tokio::spawn(ss_session(client, record.clone()));
        }
    });
    Stand { addr, seen }
}

async fn ss_session(client: TcpStream, record: Arc<Mutex<Seen>>) {
    let method = Method::parse(SS_METHOD).unwrap();
    let master = shadowsocks::master_key(SS_PASSWORD.as_bytes(), method.key_len());
    let mut decoder = codec::Decoder::new(method, &master);
    let (mut read, mut write) = client.into_split();
    let mut chunk = vec![0u8; 16 * 1024];
    let first = loop {
        match decoder.chunk() {
            Ok(Some(plain)) => break plain,
            Ok(None) => {}
            Err(_) => {
                record.lock().unwrap().refused += 1;
                return;
            }
        }
        match read.read(&mut chunk).await {
            Ok(0) | Err(_) => return,
            Ok(n) => decoder.feed(&chunk[..n]),
        }
    };
    let Some((host, port, used)) = codec::parse_address(&first) else {
        return;
    };
    let Ok(far) = TcpStream::connect((host.as_str(), port)).await else {
        return;
    };
    record
        .lock()
        .unwrap()
        .targets
        .push(format!("{host}:{port}"));
    let (mut far_read, mut far_write) = far.into_split();
    if far_write.write_all(&first[used..]).await.is_err() {
        return;
    }
    let up = tokio::spawn(async move {
        let mut chunk = vec![0u8; 16 * 1024];
        loop {
            while let Ok(Some(plain)) = decoder.chunk() {
                if far_write.write_all(&plain).await.is_err() {
                    return;
                }
            }
            match read.read(&mut chunk).await {
                Ok(0) | Err(_) => break,
                Ok(n) => decoder.feed(&chunk[..n]),
            }
        }
        let _ = far_write.shutdown().await;
    });
    let mut encoder = codec::Encoder::new(method, &master).unwrap();
    loop {
        let n = far_read.read(&mut chunk).await.unwrap_or(0);
        if n == 0 {
            break;
        }
        let mut sealed = Vec::new();
        encoder.seal(&chunk[..n], &mut sealed).unwrap();
        if write.write_all(&sealed).await.is_err() {
            break;
        }
    }
    let _ = write.shutdown().await;
    let _ = up.await;
}

/// A Wisp stand and the switch that closes each of its connections.
pub struct WispStand {
    pub stand: Stand,
    /// Each connection sends a Close of the WebSocket when it is notified.
    pub close_all: Arc<Notify>,
    /// The count of the connections that upgraded.
    pub connections: Arc<Mutex<u32>>,
}

/// A Wisp server of version 2: TLS, the upgrade at [`WISP_PATH`], its INFO
/// with stream open confirmation, the INFO of the client, then its CONTINUE.
/// It dials the target of each CONNECT and confirms the stream with a
/// CONTINUE, or closes it with the reason 0x44 (refused).
pub async fn wisp() -> WispStand {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let seen: Arc<Mutex<Seen>> = Arc::default();
    let close_all = Arc::new(Notify::new());
    let connections: Arc<Mutex<u32>> = Arc::default();
    let acceptor = road_stand::acceptor();
    let (record, close, count) = (seen.clone(), close_all.clone(), connections.clone());
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let (acceptor, record, close, count) = (
                acceptor.clone(),
                record.clone(),
                close.clone(),
                count.clone(),
            );
            tokio::spawn(async move {
                let Ok(mut tls) = acceptor.accept(tcp).await else {
                    return;
                };
                if upgrade(&mut tls, WISP_PATH).await.is_some() {
                    *count.lock().unwrap() += 1;
                    wisp_session(tls, record, close).await;
                }
            });
        }
    });
    WispStand {
        stand: Stand { addr, seen },
        close_all,
        connections,
    }
}

async fn wisp_session<S>(ws: S, record: Arc<Mutex<Seen>>, close: Arc<Notify>)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (mut ws_read, mut ws_write) = tokio::io::split(ws);
    let (send, mut out) = mpsc::unbounded_channel::<Vec<u8>>();
    let writer = tokio::spawn(async move {
        loop {
            tokio::select! {
                message = out.recv() => {
                    let Some(m) = message else { return };
                    if ws_write.write_all(&server_frame(2, m)).await.is_err() {
                        return;
                    }
                }
                () = close.notified() => {
                    let _ = ws_write.write_all(&server_frame(8, 1001u16.to_be_bytes().to_vec())).await;
                    let _ = ws_write.shutdown().await;
                    return;
                }
            }
        }
    });
    let confirm = Extension {
        id: ext::CONFIRM,
        payload: Vec::new(),
    };
    let info = Info {
        major: 2,
        minor: 1,
        extensions: vec![confirm],
    };
    let _ = send.send(packet::encode(0, &Packet::Info(info)));
    let mut raw = Vec::new();
    let opened = next_payload(&mut ws_read, &mut raw).await;
    if !matches!(
        opened.as_deref().map(packet::decode),
        Some(Ok((0, Packet::Info(_))))
    ) {
        writer.abort();
        return;
    }
    let _ = send.send(packet::encode(0, &Packet::Continue(WISP_BUFFER)));
    let mut streams: HashMap<u32, tokio::net::tcp::OwnedWriteHalf> = HashMap::new();
    while let Some(message) = next_payload(&mut ws_read, &mut raw).await {
        let Ok((id, packet)) = packet::decode(&message) else {
            break;
        };
        match packet {
            Packet::Connect { port, host, .. } => {
                let Ok(far) = TcpStream::connect((host.as_str(), port)).await else {
                    record.lock().unwrap().refused += 1;
                    let _ = send.send(packet::encode(id, &Packet::Close(Reason::REFUSED)));
                    continue;
                };
                record
                    .lock()
                    .unwrap()
                    .targets
                    .push(format!("{host}:{port}"));
                let (mut far_read, far_write) = far.into_split();
                streams.insert(id, far_write);
                let _ = send.send(packet::encode(id, &Packet::Continue(WISP_BUFFER)));
                let send = send.clone();
                tokio::spawn(async move {
                    let mut chunk = vec![0u8; 16 * 1024];
                    loop {
                        match far_read.read(&mut chunk).await {
                            Ok(0) | Err(_) => break,
                            Ok(n) => {
                                let _ = send.send(packet::data(id, &chunk[..n]));
                            }
                        }
                    }
                    let _ = send.send(packet::encode(id, &Packet::Close(Reason::VOLUNTARY)));
                });
            }
            Packet::Data(bytes) => {
                if let Some(far) = streams.get_mut(&id) {
                    let _ = far.write_all(&bytes).await;
                    let _ = send.send(packet::encode(id, &Packet::Continue(WISP_BUFFER)));
                }
            }
            Packet::Close(_) => {
                if let Some(mut far) = streams.remove(&id) {
                    let _ = far.shutdown().await;
                }
            }
            _ => {}
        }
    }
    drop(send);
    let _ = writer.await;
}
