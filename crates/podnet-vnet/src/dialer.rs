//! The dialers of the virtual network, of the pod upstream chains and of
//! the tailnet front door (T-056). Each one is a [`Dial`] of
//! `podnet_core::network` over a function that exists, with no change of
//! its behaviour:
//!
//! * [`VnetDialer`]: [`NetStack::connect`] and [`NetStack::bind_udp`];
//! * [`UpstreamChain`]: `upstream::connect_upstream_async` (the `-x`
//!   chain of `podnet pod`: SOCKS5, HTTP CONNECT, a Unix socket, direct, or
//!   one tailnet hop);
//! * [`Tailnet`]: `tailscale::Dialer::dial` (`ts-dial` over the LocalAPI
//!   of `tailscaled`).
//!
//! [`PodRule`] gives a rule of `podnet pod -r` (`RuleMatch`) to the
//! combinator `Rules` of `podnet_core::network` (T-058).

use std::future::Future;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;

use podnet_core::network::{
    BoxFuture, Class, Connected, Datagrams, Dial, Failure, Matcher, Stage, Target,
};
use podnet_netns::tailscale;
use podnet_netns::upstream::{self, Proxy, ProxyKind};
use podnet_netns::RuleMatch;
use tokio::time::Instant;

use crate::addr::VirtAddr;
use crate::dns::Resolver;
use crate::stack::NetStack;

/// Run `dial` until `deadline`. After the deadline, the failure has the
/// class `Timeout`.
async fn before<T>(
    deadline: Instant,
    what: &str,
    dial: impl Future<Output = Result<T, Failure>>,
) -> Result<T, Failure> {
    match tokio::time::timeout_at(deadline, dial).await {
        Ok(done) => done,
        Err(_) => Err(Failure::new(
            Stage::Connect,
            Class::Timeout,
            format!("{what}: no connection before the time limit"),
        )),
    }
}

/// The first port of the UDP sockets of [`VnetDialer::datagrams`].
const FIRST_UDP_PORT: u16 = 49152;

/// The ports that [`VnetDialer::datagrams`] tries before it stops.
const UDP_PORT_TRIES: usize = 64;

/// A way out through a userspace [`NetStack`]: TCP and UDP to the
/// addresses of the virtual network.
pub struct VnetDialer {
    stack: Arc<NetStack>,
    local: IpAddr,
    names: Option<Arc<Resolver>>,
    next_port: AtomicU16,
}

impl VnetDialer {
    /// A dialer through `stack`. `local` is the address of the stack (the
    /// `local_v4` or the `local_v6` of its [`crate::StackConfig`]); a UDP
    /// socket binds to it.
    pub fn new(stack: Arc<NetStack>, local: IpAddr) -> VnetDialer {
        VnetDialer {
            stack,
            local,
            names: None,
            next_port: AtomicU16::new(FIRST_UDP_PORT),
        }
    }

    /// Resolve the name of a target with the table `names` (the names of
    /// the virtual network). With no table, a name is an error.
    pub fn with_names(mut self, names: Arc<Resolver>) -> VnetDialer {
        self.names = Some(names);
        self
    }

    /// The virtual address of `target`.
    fn addr_of(&self, target: &Target) -> Result<VirtAddr, Failure> {
        let (host, port) = match target {
            Target::Addr(a) => return Ok(VirtAddr::new(a.ip(), a.port())),
            Target::Name { host, port } => (host, *port),
        };
        let ips = self.names.as_ref().and_then(|n| n.get(host)).unwrap_or(&[]);
        let same = ips.iter().find(|ip| ip.is_ipv4() == self.local.is_ipv4());
        match same.or(ips.first()) {
            Some(ip) => Ok(VirtAddr::new(*ip, port)),
            None => Err(Failure::new(
                Stage::Dns,
                Class::NotFound,
                format!("{host} is not a name of the virtual network"),
            )),
        }
    }
}

impl Dial for VnetDialer {
    fn label(&self) -> String {
        format!("vnet:{}", self.local)
    }

    fn dial<'a>(
        &'a self,
        target: &'a Target,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(async move {
            let to = self.addr_of(target)?;
            let what = format!("vnet TCP to {to}");
            let stream = before(deadline, &what, async {
                let mut stream = self.stack.connect(to).await.map_err(connect_failure)?;
                stream.established().await.map_err(connect_failure)?;
                Ok(stream)
            })
            .await?;
            let local = stream.local_addr();
            Ok(Connected {
                io: Box::new(stream),
                peer: Some(SocketAddr::new(to.ip, to.port)),
                local: Some(local),
            })
        })
    }

    /// True with a table of names: the dialer resolves the name, so the
    /// caller sends it.
    fn names_inside(&self) -> bool {
        self.names.is_some()
    }

    /// A UDP socket on the address of the stack, on the next free port
    /// from 49152.
    fn datagrams<'a>(&'a self) -> BoxFuture<'a, Result<Arc<dyn Datagrams>, Failure>> {
        Box::pin(async move {
            let mut last = None;
            for _ in 0..UDP_PORT_TRIES {
                let port = self.next_port.fetch_add(1, Ordering::Relaxed);
                let port = if port < FIRST_UDP_PORT {
                    self.next_port.store(FIRST_UDP_PORT + 1, Ordering::Relaxed);
                    FIRST_UDP_PORT
                } else {
                    port
                };
                match self.stack.bind_udp(VirtAddr::new(self.local, port)).await {
                    Ok(socket) => {
                        return Ok(Arc::new(socket.into_datagrams()) as Arc<dyn Datagrams>)
                    }
                    Err(e) if e.kind() == io::ErrorKind::AddrInUse => last = Some(e),
                    Err(e) => return Err(Failure::io(Stage::Connect, e)),
                }
            }
            let e = last.unwrap_or_else(|| io::Error::from(io::ErrorKind::AddrInUse));
            Err(Failure::io(Stage::Connect, e))
        })
    }
}

fn connect_failure(e: io::Error) -> Failure {
    Failure::io(Stage::Connect, e)
}

/// The chain of `-x` hops of `podnet pod` as a way out: TCP only.
pub struct UpstreamChain {
    chain: Vec<Proxy>,
}

impl UpstreamChain {
    /// A chain of the hops `chain`, in their order. An empty chain is an
    /// error.
    pub fn new(chain: Vec<Proxy>) -> Result<UpstreamChain, Failure> {
        if chain.is_empty() {
            return Err(Failure::new(
                Stage::Proxy,
                Class::Usage,
                "an upstream chain needs one hop or more",
            ));
        }
        Ok(UpstreamChain { chain })
    }

    /// A chain of the specs of `-x` (`socks5://HOST:PORT`, `unix:/PATH`,
    /// `http://...`, `tailscale:/PATH`, `direct`).
    pub fn parse(specs: &[&str]) -> anyhow::Result<UpstreamChain> {
        let chain = specs
            .iter()
            .map(|s| Proxy::parse(s))
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(UpstreamChain::new(chain)?)
    }
}

/// The word of one hop in a label, with no credentials.
fn hop_label(p: &Proxy) -> String {
    match p.kind {
        ProxyKind::Direct => "direct".into(),
        ProxyKind::Tailscale => format!("tailscale:{}", p.unix_path.as_deref().unwrap_or("")),
        ProxyKind::Socks5 => format!("socks5://{}", p.endpoint()),
        ProxyKind::Http => format!("http://{}", p.endpoint()),
    }
}

impl Dial for UpstreamChain {
    fn label(&self) -> String {
        self.chain
            .iter()
            .map(hop_label)
            .collect::<Vec<_>>()
            .join(" -> ")
    }

    fn dial<'a>(
        &'a self,
        target: &'a Target,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(async move {
            let to = target.to_string();
            let what = format!("{to} through {}", self.label());
            let io = before(deadline, &what, async {
                upstream::connect_upstream_async(&self.chain, &to)
                    .await
                    .map_err(|e| {
                        Failure::caused(Stage::Proxy, Class::Unavailable, format!("{e:#}"), e)
                    })
            })
            .await?;
            Ok(Connected::new(Box::new(io)))
        })
    }

    /// True when the first hop is a proxy or the tailnet: the name of the
    /// target goes to it.
    fn names_inside(&self) -> bool {
        self.chain[0].kind != ProxyKind::Direct
    }
}

/// A way out through a running `tailscaled`: TCP to the tailnet. The
/// daemon resolves MagicDNS names.
pub struct Tailnet {
    dialer: tailscale::Dialer,
}

impl Tailnet {
    /// A dialer that uses the LocalAPI socket at `socket`.
    pub fn new(socket: impl Into<PathBuf>) -> Tailnet {
        Tailnet {
            dialer: tailscale::Dialer::new(socket),
        }
    }
}

impl Dial for Tailnet {
    fn label(&self) -> String {
        format!("tailscale:{}", self.dialer.socket().display())
    }

    fn dial<'a>(
        &'a self,
        target: &'a Target,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(async move {
            let what = format!("tailnet TCP to {target}");
            let conn = before(deadline, &what, async {
                self.dialer
                    .dial(&target.host(), target.port())
                    .await
                    .map_err(|e| Failure::io(Stage::Connect, e))
            })
            .await?;
            Ok(Connected::new(Box::new(conn)))
        })
    }

    fn names_inside(&self) -> bool {
        true
    }
}

/// A rule of `podnet pod -r` (`domain:NAME`, `domain:.NAME`, `cidr:NET`,
/// `cidr:@FILE`) as a [`Matcher`] of `Rules` (T-058). A `domain:` rule
/// matches a name only; a `cidr:` rule matches an address only.
pub struct PodRule {
    spec: String,
    rule: RuleMatch,
}

impl PodRule {
    /// Parse `spec` as `podnet pod -r` does.
    pub fn parse(spec: &str) -> anyhow::Result<PodRule> {
        Ok(PodRule {
            spec: spec.to_string(),
            rule: RuleMatch::parse(spec)?,
        })
    }
}

impl Matcher for PodRule {
    fn label(&self) -> String {
        self.spec.clone()
    }

    fn matches(&self, target: &Target) -> bool {
        match target {
            Target::Name { host, .. } => self.rule.matches(host, None),
            Target::Addr(addr) => self.rule.matches(&addr.ip().to_string(), Some(addr.ip())),
        }
    }
}

#[cfg(test)]
mod tests;
