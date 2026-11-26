#![cfg(test)]
//! Stand-ins on the loopback for the tests of `https://` and `wstunnel:`
//! (T-126): an HTTPS forward proxy that takes CONNECT on HTTP/1.1 with
//! Basic, a wstunnel server that reads the path prefix and the token of a
//! client as wstunnel 11 does, and an echo target. Both servers use TLS with
//! the CA of the TURN stand (`turn_mock.rs`), for `localhost` and
//! `127.0.0.1`. No public server and no account: the user, the password and
//! the prefix are made-up values.
//!
//! The file is used by the unit tests (`networks::via_tests`) and by the
//! test of the command (`tests/road_network.rs`, by `#[path]`), each beside
//! `turn_mock.rs`.
#![allow(dead_code)]

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use pod_tls::server::TlsAcceptor;
use pod_ws::frame::{self, Frame, Role};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use super::turn_mock;

/// The user of the proxy stand.
pub const USER: &str = "proxy-user";
/// The password of the proxy stand (a made-up value).
pub const PASSWORD: &str = "proxy-secret-for-tests";
/// The path prefix of the wstunnel stand (a made-up value).
pub const PREFIX: &str = "prefix-secret-for-tests";

/// What a stand saw: the target of each request that it took, and the
/// count of the requests that it refused.
#[derive(Default, Debug)]
pub struct Seen {
    pub targets: Vec<String>,
    pub refused: u32,
}

/// A stand on the loopback.
pub struct Stand {
    pub addr: SocketAddr,
    pub seen: Arc<Mutex<Seen>>,
}

/// The CA of the stands as PEM: a test gives it to `--ca-file`.
pub fn ca_pem() -> &'static str {
    &turn_mock::certificates().ca
}

fn acceptor() -> TlsAcceptor {
    let made = turn_mock::certificates();
    pod_tls::server::acceptor(
        turn_mock::provider(),
        made.leaf.as_bytes(),
        made.key.as_bytes(),
        &[],
    )
    .expect("the certificate of the stand")
}

/// A target that sends back each byte that it reads, and ends its side at
/// the end of its input.
pub async fn echo() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let (mut read, mut write) = stream.split();
                let _ = tokio::io::copy(&mut read, &mut write).await;
                let _ = write.shutdown().await;
            });
        }
    });
    addr
}

/// The head of a request, up to the empty line; `None` at an early end.
async fn read_head<S: AsyncRead + Unpin>(stream: &mut S) -> Option<String> {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    while !buf.ends_with(b"\r\n\r\n") {
        if buf.len() > 16 * 1024 || stream.read(&mut byte).await.ok()? == 0 {
            return None;
        }
        buf.push(byte[0]);
    }
    String::from_utf8(buf).ok()
}

/// The value of the header `name` (lower case) of `head`.
fn header<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    head.lines().skip(1).find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.trim()
            .eq_ignore_ascii_case(name)
            .then_some(value.trim())
    })
}

/// An HTTPS proxy: TLS, then `CONNECT HOST:PORT` with Basic of
/// [`USER`]:[`PASSWORD`]. Other credentials get a 407.
pub async fn https_proxy() -> Stand {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let seen: Arc<Mutex<Seen>> = Arc::default();
    let (acceptor, record) = (acceptor(), seen.clone());
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let (acceptor, record) = (acceptor.clone(), record.clone());
            tokio::spawn(async move {
                let Ok(mut tls) = acceptor.accept(tcp).await else {
                    return;
                };
                let Some(head) = read_head(&mut tls).await else {
                    return;
                };
                let target = head
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or_default()
                    .to_string();
                let pair = format!("{USER}:{PASSWORD}");
                let basic = base64::engine::general_purpose::STANDARD.encode(pair);
                let expected = format!("Basic {basic}");
                if header(&head, "proxy-authorization") != Some(expected.as_str()) {
                    record.lock().unwrap().refused += 1;
                    let answer = "HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"stand\"\r\nContent-Length: 0\r\n\r\n";
                    let _ = tls.write_all(answer.as_bytes()).await;
                    return;
                }
                let Ok(mut far) = TcpStream::connect(&target).await else {
                    let _ = tls.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n").await;
                    return;
                };
                record.lock().unwrap().targets.push(target);
                let ok = b"HTTP/1.1 200 Connection established\r\n\r\n";
                if tls.write_all(ok).await.is_ok() {
                    let _ = tokio::io::copy_bidirectional(&mut tls, &mut far).await;
                }
            });
        }
    });
    Stand { addr, seen }
}

/// The target of a wstunnel token: the claims `r` and `rp` of its JWT.
fn token_target(protocols: &str) -> Option<String> {
    let token = protocols
        .split(',')
        .map(str::trim)
        .find_map(|p| p.strip_prefix("authorization.bearer."))?;
    let claims = token.split('.').nth(1)?;
    let claims = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(claims)
        .ok()?;
    let claims: serde_json::Value = serde_json::from_slice(&claims).ok()?;
    Some(format!(
        "{}:{}",
        claims["r"].as_str()?,
        claims["rp"].as_u64()?
    ))
}

/// A wstunnel server: TLS, then `GET /PREFIX/events` with the subprotocol
/// `v1` and the token. It dials the target of the token before its 101, as
/// wstunnel does, and refuses each other request with 400 "Invalid
/// request". The frames carry the bytes in binary frames.
pub async fn wstunnel() -> Stand {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let seen: Arc<Mutex<Seen>> = Arc::default();
    let (acceptor, record) = (acceptor(), seen.clone());
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let (acceptor, record) = (acceptor.clone(), record.clone());
            tokio::spawn(async move {
                let Ok(mut tls) = acceptor.accept(tcp).await else {
                    return;
                };
                let Some(head) = read_head(&mut tls).await else {
                    return;
                };
                let path = head.split_whitespace().nth(1).unwrap_or_default();
                let key = header(&head, "sec-websocket-key");
                let target = header(&head, "sec-websocket-protocol").and_then(token_target);
                let far = match target.as_deref() {
                    Some(t) if path == format!("/{PREFIX}/events") => {
                        TcpStream::connect(t).await.ok()
                    }
                    _ => None,
                };
                let (Some(key), Some(target), Some(far)) = (key, target, far) else {
                    record.lock().unwrap().refused += 1;
                    let answer =
                        "HTTP/1.1 400 Bad Request\r\nContent-Length: 15\r\n\r\nInvalid request";
                    let _ = tls.write_all(answer.as_bytes()).await;
                    return;
                };
                record.lock().unwrap().targets.push(target);
                let answer = format!(
                    "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\nSec-WebSocket-Protocol: v1\r\n\r\n",
                    pod_ws::handshake::accept_key(key)
                );
                if tls.write_all(answer.as_bytes()).await.is_ok() {
                    pump(tls, far).await;
                }
            });
        }
    });
    Stand { addr, seen }
}

/// The bytes of a tunnel: each binary frame of the client to the target,
/// each read of the target to the client in a binary frame. A Close of the
/// client ends the input of the target; the end of the target sends a Close.
async fn pump<S>(ws: S, far: TcpStream)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (mut ws_read, mut ws_write) = tokio::io::split(ws);
    let (mut far_read, mut far_write) = far.into_split();
    let up = tokio::spawn(async move {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 16 * 1024];
        loop {
            while let Ok(Some((frame, used))) = frame::decode(&buf, Role::Client) {
                buf.drain(..used);
                match frame.opcode {
                    0 | 2 => {
                        if far_write.write_all(&frame.payload).await.is_err() {
                            return;
                        }
                    }
                    8 => {
                        let _ = far_write.shutdown().await;
                        return;
                    }
                    _ => {}
                }
            }
            match ws_read.read(&mut chunk).await {
                Ok(0) | Err(_) => {
                    let _ = far_write.shutdown().await;
                    return;
                }
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
            }
        }
    });
    let mut chunk = [0u8; 16 * 1024];
    loop {
        let n = far_read.read(&mut chunk).await.unwrap_or(0);
        let frame = match n {
            0 => Frame {
                fin: true,
                opcode: 8,
                payload: 1000u16.to_be_bytes().to_vec(),
            },
            n => Frame {
                fin: true,
                opcode: 2,
                payload: chunk[..n].to_vec(),
            },
        };
        let bytes = frame::encode(&frame, Role::Server, [0; 4]);
        if ws_write.write_all(&bytes).await.is_err() || n == 0 {
            break;
        }
    }
    let _ = ws_write.shutdown().await;
    let _ = up.await;
}
