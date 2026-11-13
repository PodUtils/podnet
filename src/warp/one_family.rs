//! A UDP socket of the WARP stack on one address of the device, and a way
//! out whose datagrams take it.
//!
//! A fix (PodUtils/podnet#523, T-332): the datagrams of `pod_warp::WarpNet`
//! bind `[::]:port` (`bind_udp_any`). On such a socket noq sends an IPv4
//! destination as an IPv4-mapped IPv6 address, which the stack does not route,
//! and a datagram that comes back can carry a source that does not equal the
//! IPv4 address that the caller sent to. A socket on one address of the device
//! sends and receives in one family. podcrates holds the fix of `pod-warp`
//! (PodUtils/podnet#524).

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use anyhow::{Context as _, Result};
use pod_core::{BoxFuture, Connected, Datagrams, Dial, Failure, Target};
use tokio::time::Instant;

use super::ip::WarpStack;

/// The first port of the ephemeral range (RFC 6335).
const EPHEMERAL_BASE: u16 = 49152;

/// A UDP socket of the stack on one address of the device: of IPv6 when
/// `v6`, else of IPv4; the other family when the device has none of it.
pub async fn socket(stack: &WarpStack, v6: bool) -> Result<Arc<dyn Datagrams>> {
    let addresses = stack.addresses();
    let ip = addresses
        .iter()
        .find(|ip| ip.is_ipv6() == v6)
        .or_else(|| addresses.first())
        .copied()
        .context("the WARP device has no address")?;
    let mut last = None;
    for step in 0..64u16 {
        let port = EPHEMERAL_BASE + (std::process::id() as u16).wrapping_add(step * 97) % 16384;
        match stack.stack().bind_udp(SocketAddr::new(ip, port)).await {
            Ok(socket) => return Ok(Arc::new(socket.into_datagrams())),
            Err(err) if err.kind() == std::io::ErrorKind::AddrInUse => last = Some(err),
            Err(err) => return Err(err).context("binding a UDP socket inside the WARP session"),
        }
    }
    Err(last
        .map(anyhow::Error::from)
        .unwrap_or_else(|| anyhow::anyhow!("no free port")))
    .context("binding a UDP socket inside the WARP session")
}

/// The WARP session as a way out whose datagrams bind one family: the TCP
/// dials and the names go to the session.
pub struct OneFamily {
    pub stack: Arc<WarpStack>,
    /// The family of the far end of the datagrams.
    pub v6: bool,
}

impl OneFamily {
    /// The way for datagrams to `ip`.
    pub fn to(stack: Arc<WarpStack>, ip: IpAddr) -> OneFamily {
        OneFamily {
            stack,
            v6: ip.is_ipv6(),
        }
    }
}

impl Dial for OneFamily {
    fn label(&self) -> String {
        self.stack.label()
    }

    fn dial<'a>(
        &'a self,
        target: &'a Target,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        self.stack.dial(target, deadline)
    }

    fn dial_http<'a>(
        &'a self,
        target: &'a Target,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        self.stack.dial_http(target, deadline)
    }

    fn dial_addrs<'a>(
        &'a self,
        host: &'a str,
        addrs: &'a [SocketAddr],
        deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        self.stack.dial_addrs(host, addrs, deadline)
    }

    fn names_inside(&self) -> bool {
        self.stack.names_inside()
    }

    fn datagrams<'a>(&'a self) -> BoxFuture<'a, Result<Arc<dyn Datagrams>, Failure>> {
        Box::pin(async move {
            socket(&self.stack, self.v6).await.map_err(|err| {
                Failure::new(
                    pod_core::Stage::Warp,
                    pod_core::Class::Unavailable,
                    format!("{err:#}"),
                )
            })
        })
    }
}
