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

/// A name of `forward --name` (T-404): the address on 127.0.0.0/8 that
/// stands for it, and the target of its forward.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alias {
    pub name: String,
    pub ip: std::net::Ipv4Addr,
    pub target: String,
}

/// The most names: the addresses 127.0.0.2 to 127.0.0.255.
pub const MAX_ALIASES: usize = 254;

/// Parse each `NAME=HOST:PORT` of `forward --name`. The names get the
/// addresses 127.0.0.2, 127.0.0.3 and on, in their order. A name is
/// lowercase letters, digits, `-` and `.`, 253 characters at most; each name
/// once.
pub fn aliases(specs: &[String]) -> Result<Vec<Alias>> {
    if specs.len() > MAX_ALIASES {
        anyhow::bail!("--name: {} names; the most is {MAX_ALIASES}", specs.len());
    }
    let mut out: Vec<Alias> = Vec::new();
    for (i, spec) in specs.iter().enumerate() {
        let (name, target) = spec
            .split_once('=')
            .with_context(|| format!("--name {spec:?}: give NAME=HOST:PORT"))?;
        let name = name.trim_end_matches('.').to_ascii_lowercase();
        let good = !name.is_empty()
            && name.len() <= 253
            && name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.');
        if !good {
            anyhow::bail!("--name {spec:?}: a name is letters, digits, - and .");
        }
        if out.iter().any(|a| a.name == name) {
            anyhow::bail!("--name: {name} is given twice");
        }
        target
            .parse::<Target>()
            .map_err(|_| anyhow::anyhow!("--name {spec:?}: give NAME=HOST:PORT"))?;
        let ip = std::net::Ipv4Addr::new(127, 0, 0, u8::try_from(i + 2).expect("at most 254"));
        out.push(Alias {
            name,
            ip,
            target: target.to_string(),
        });
    }
    Ok(out)
}

/// The table of the DNS door for `aliases`: each name has its address;
/// each other name goes to the upstream of the door.
pub fn alias_table(aliases: &[Alias]) -> podnet_vnet::dns::Resolver {
    let mut table = podnet_vnet::dns::Resolver::new();
    for a in aliases {
        table.insert(&a.name, std::net::IpAddr::V4(a.ip));
    }
    // A marker: the door has an upstream (the network), so an unknown name
    // is not answered here.
    table.upstream = Some(podnet_vnet::addr::VirtAddr::v4(
        std::net::Ipv4Addr::UNSPECIFIED,
        53,
    ));
    table
}

mod udp;
pub use udp::{bind_dns, serve_dns, serve_dns_on, serve_dns_with, serve_udp};

#[cfg(test)]
mod tests;
