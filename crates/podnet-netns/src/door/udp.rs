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
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite};
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
