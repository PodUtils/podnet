//! Port forwards over any network (T-099): each connection to a local
//! listener goes through a [`Dial`] to one target. `podnet forward -L
//! LISTEN=HOST:PORT --network SPEC` uses it (`udp:LISTEN` for UDP); the network is a spec of the
//! registry (`crate::networks`), and with no spec the way out of the
//! process.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use podnet_netns::listen::ProxyListen;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::time::Instant;

use crate::network::{Dial, Target};

/// The limit of one dial to the target.
pub const DIAL_LIMIT: Duration = Duration::from_secs(30);

/// One rule of `-L LISTEN=HOST:PORT`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    pub listen: ProxyListen,
    pub target: Target,
    /// True for `udp:LISTEN`: a UDP listener, and datagrams to the target.
    pub udp: bool,
    /// The spec `peer:FINGERPRINT/SERVICE` when the rule is `-L
    /// LISTEN=peer:FINGERPRINT/SERVICE` (T-266). The rule then has its own
    /// network: one stream of a link for each accepted connection. The
    /// target is the peer, with port 0.
    pub via: Option<String>,
}

impl Rule {
    /// `LISTEN=HOST:PORT`: LISTEN is `PORT` (on 127.0.0.1), `ADDR:PORT`,
    /// `tcp://ADDR:PORT` or `unix:/PATH`; an IPv6 host is in brackets.
    /// `udp:` before LISTEN (`udp:PORT`, `udp:ADDR:PORT`) makes a UDP
    /// forward. The target `peer:FINGERPRINT/SERVICE` makes a TCP forward
    /// to a service of a peer, over a link that a running `podnet link`
    /// instance holds.
    pub fn parse(spec: &str) -> Result<Rule> {
        let (listen, target) = spec
            .rsplit_once('=')
            .with_context(|| format!("-L {spec:?}: give LISTEN=HOST:PORT"))?;
        let (udp, listen) = match listen.strip_prefix("udp:") {
            Some(rest) => (true, rest),
            None => (false, listen),
        };
        let listen = match listen.parse::<u16>() {
            Ok(0) => bail!("-L {spec:?}: the listen port must be 1 to 65535"),
            Ok(port) => ProxyListen::Tcp(std::net::SocketAddr::from(([127, 0, 0, 1], port))),
            Err(_) => {
                ProxyListen::parse(listen).map_err(|e| anyhow::anyhow!("-L {spec:?}: {e}"))?
            }
        };
        if target.starts_with("peer:") {
            if udp {
                bail!("-L {spec:?}: a peer carries streams, not datagrams");
            }
            let network =
                crate::networks::parse(target).map_err(|e| anyhow::anyhow!("-L {spec:?}: {e}"))?;
            let (fingerprint, _) = network
                .peer()
                .with_context(|| format!("-L {spec:?}: give peer:FINGERPRINT/SERVICE after ="))?;
            return Ok(Rule {
                listen,
                target: crate::network::peer(fingerprint),
                udp,
                via: Some(target.to_string()),
            });
        }
        let target: Target = target
            .parse()
            .map_err(|_| anyhow::anyhow!("-L {spec:?}: give HOST:PORT after ="))?;
        if target.port() == 0 {
            bail!("-L {spec:?}: the port must be 1 to 65535");
        }
        if udp && !matches!(listen, ProxyListen::Tcp(_)) {
            bail!("-L {spec:?}: a UDP forward listens on PORT or ADDR:PORT");
        }
        Ok(Rule {
            listen,
            target,
            udp,
            via: None,
        })
    }
}

/// Carry each connection of `listener` through `dial` to `target`, until
/// the listener fails.
pub async fn serve_unix(
    listener: tokio::net::UnixListener,
    dial: Arc<dyn Dial>,
    target: Target,
) -> Result<()> {
    loop {
        let (stream, _) = listener.accept().await.context("the forward listener")?;
        spawn(stream, dial.clone(), target.clone());
    }
}

/// [`serve_unix`] for a TCP listener.
pub async fn serve_tcp(
    listener: tokio::net::TcpListener,
    dial: Arc<dyn Dial>,
    target: Target,
) -> Result<()> {
    loop {
        let (stream, _) = listener.accept().await.context("the forward listener")?;
        spawn(stream, dial.clone(), target.clone());
    }
}

fn spawn<S>(mut stream: S, dial: Arc<dyn Dial>, target: Target)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        match dial.dial(&target, Instant::now() + DIAL_LIMIT).await {
            Ok(mut conn) => {
                let _ = tokio::io::copy_bidirectional(&mut stream, &mut conn.io).await;
            }
            Err(e) => {
                tracing::warn!(%target, way = %dial.label(), "a forward did not reach its target: {e}")
            }
        }
    });
}

mod udp;
pub use udp::{serve_dns, serve_udp};

#[cfg(test)]
mod tests;
