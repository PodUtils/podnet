//! UDP ASSOCIATE for the door (T-098): one UDP socket of the door relays the
//! SOCKS5 datagrams (RFC 1928, section 7) of one client over the
//! [`Dial::datagrams`] of the door. The first valid datagram fixes the source
//! of the client; a datagram of another source is dropped. A fragment is
//! dropped. An answer is relayed only when it comes from an address that the
//! client sent to. A name in a datagram resolves with the resolver of this
//! host (the datagrams of a way take an address). The association ends
//! when the client closes the control stream or when it is idle.

use std::collections::HashSet;
use std::net::SocketAddr;
use std::time::Duration;

use crate::socks::{self, ProxyHost};
use anyhow::{Context as _, Result};
use pod_core::datagrams::{recv_from, send_to};
use pod_core::Class;
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};
use tokio::net::UdpSocket;
use tokio::time::Instant;

use super::Door;

/// The idle time after which an association ends (the control stream of
/// the client may stay open with no traffic).
pub const IDLE: Duration = Duration::from_secs(300);
/// The most peers that one association may send to.
pub const MAX_PEERS: usize = 1024;
/// The time to resolve the name of a datagram.
const RESOLVE_LIMIT: Duration = Duration::from_secs(5);
/// The largest datagram.
const MAX_DATAGRAM: usize = 65535;

/// Serve one UDP ASSOCIATE on the control stream `stream`: the reply with
/// the address of the relay socket, then the relay until the client closes
/// the control stream, or the association is idle.
pub(super) async fn associate<S>(door: &Door, stream: &mut S) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    let datagrams = match door.dial.datagrams().await {
        Ok(datagrams) => datagrams,
        Err(e) => {
            // The kill switch of a group says "not allowed"; a way with no
            // datagrams says "command not supported".
            let code = if e.class == Class::Policy { 0x02 } else { 0x07 };
            socks::socks5_reply(stream, code, None).await?;
            return Ok(());
        }
    };
    let relay = UdpSocket::bind((door.udp_bind, 0))
        .await
        .context("the UDP socket of the door")?;
    socks::socks5_reply(stream, 0, Some(relay.local_addr()?)).await?;
    let mut client: Option<SocketAddr> = None;
    let mut peers: HashSet<SocketAddr> = HashSet::new();
    let (mut up, mut down) = (vec![0u8; MAX_DATAGRAM], vec![0u8; MAX_DATAGRAM]);
    let mut hold = [0u8; 64];
    let mut idle_at = Instant::now() + IDLE;
    loop {
        tokio::select! {
            got = relay.recv_from(&mut up) => {
                let (n, from) = got.context("the UDP socket of the door")?;
                if client.is_some_and(|c| c != from) {
                    continue;
                }
                let Some((host, port, payload)) = socks::socks5_udp_unwrap(&up[..n]) else {
                    continue;
                };
                let Some(to) = resolve(&host, port).await else {
                    continue;
                };
                if !peers.contains(&to) && peers.len() >= MAX_PEERS {
                    continue;
                }
                client.get_or_insert(from);
                peers.insert(to);
                idle_at = Instant::now() + IDLE;
                if let Err(e) = send_to(&*datagrams, payload, to).await {
                    tracing::debug!("a datagram of the door to {to}: {e}");
                }
            }
            back = recv_from(&*datagrams, &mut down) => {
                let (n, from) = back.context("the datagrams of the door")?;
                let Some(to) = client.filter(|_| peers.contains(&from)) else {
                    continue;
                };
                idle_at = Instant::now() + IDLE;
                let _ = relay.send_to(&wrap(from, &down[..n]), to).await;
            }
            closed = stream.read(&mut hold) => {
                // The client sends nothing on the control stream; its end
                // ends the association.
                if matches!(closed, Ok(0) | Err(_)) {
                    return Ok(());
                }
            }
            _ = tokio::time::sleep_until(idle_at) => return Ok(()),
        }
    }
}

/// The command 0x83 of `pod-proxy`: UDP in the stream.
pub const CMD_UDP_IN_STREAM: u8 = 0x83;

/// Serve UDP in the stream (T-329, the command 0x83 of `pod-proxy`) on
/// `stream`. The reply is the one of a UDP ASSOCIATE with the address
/// 0.0.0.0:0. Then each datagram in each direction is a frame: a length of
/// 2 bytes (big endian, at most 65535), then a SOCKS5 UDP datagram (RFC 1928,
/// section 7: RSV 2 bytes, FRAG 0, the address type, the address, the port,
/// the payload). No UDP socket is open for the client: a pipe carries the
/// frames. The rules of [`associate`] hold: no fragment, an answer only from
/// an address that the client sent to, at most [`MAX_PEERS`] peers, the end
/// when the client closes the stream or is idle for [`IDLE`]. An answer
/// that does not fit in a frame is dropped.
pub(super) async fn in_stream<S>(door: &Door, stream: &mut S) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    let datagrams = match door.dial.datagrams().await {
        Ok(datagrams) => datagrams,
        Err(e) => {
            let code = if e.class == Class::Policy { 0x02 } else { 0x07 };
            socks::socks5_reply(stream, code, None).await?;
            return Ok(());
        }
    };
    socks::socks5_reply(stream, 0, None).await?;
    let mut peers: HashSet<SocketAddr> = HashSet::new();
    let mut down = vec![0u8; MAX_DATAGRAM];
    // A read into a buffer is safe to cancel; a frame is cut out of it.
    let mut held: Vec<u8> = Vec::new();
    let mut idle_at = Instant::now() + IDLE;
    loop {
        while let Some(frame) = take_frame(&mut held) {
            let Some((host, port, payload)) = socks::socks5_udp_unwrap(&frame) else {
                continue;
            };
            let Some(to) = resolve(&host, port).await else {
                continue;
            };
            if !peers.contains(&to) && peers.len() >= MAX_PEERS {
                continue;
            }
            peers.insert(to);
            idle_at = Instant::now() + IDLE;
            if let Err(e) = send_to(&*datagrams, payload, to).await {
                tracing::debug!("a datagram of the door to {to}: {e}");
            }
        }
        tokio::select! {
            read = stream.read_buf(&mut held) => {
                if matches!(read, Ok(0) | Err(_)) {
                    return Ok(());
                }
            }
            back = recv_from(&*datagrams, &mut down) => {
                let (n, from) = back.context("the datagrams of the door")?;
                if !peers.contains(&from) {
                    continue;
                }
                let datagram = wrap(from, &down[..n]);
                let Ok(len) = u16::try_from(datagram.len()) else {
                    continue;
                };
                idle_at = Instant::now() + IDLE;
                let mut frame = Vec::with_capacity(2 + datagram.len());
                frame.extend_from_slice(&len.to_be_bytes());
                frame.extend_from_slice(&datagram);
                stream.write_all(&frame).await?;
                stream.flush().await?;
            }
            _ = tokio::time::sleep_until(idle_at) => return Ok(()),
        }
    }
}

/// Cut the first whole frame off `held`: the length, then the datagram.
fn take_frame(held: &mut Vec<u8>) -> Option<Vec<u8>> {
    let len = usize::from(u16::from_be_bytes([*held.first()?, *held.get(1)?]));
    if held.len() < 2 + len {
        return None;
    }
    let frame = held[2..2 + len].to_vec();
    held.drain(..2 + len);
    Some(frame)
}

/// The address of a datagram: a name resolves with the resolver of this host.
async fn resolve(host: &ProxyHost, port: u16) -> Option<SocketAddr> {
    match host {
        ProxyHost::Ip(ip) => Some(SocketAddr::new(*ip, port)),
        ProxyHost::Domain(name) => {
            let lookup = tokio::net::lookup_host((name.as_str(), port));
            tokio::time::timeout(RESOLVE_LIMIT, lookup)
                .await
                .ok()?
                .ok()?
                .next()
        }
    }
}

/// The SOCKS5 datagram for an answer of `from`.
fn wrap(from: SocketAddr, payload: &[u8]) -> Vec<u8> {
    let mut out = vec![0, 0, 0];
    match from {
        SocketAddr::V4(a) => {
            out.push(0x01);
            out.extend_from_slice(&a.ip().octets());
        }
        SocketAddr::V6(a) => {
            out.push(0x04);
            out.extend_from_slice(&a.ip().octets());
        }
    }
    out.extend_from_slice(&from.port().to_be_bytes());
    out.extend_from_slice(payload);
    out
}

#[cfg(test)]
#[path = "udp_tests.rs"]
mod tests;
