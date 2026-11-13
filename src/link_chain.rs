//! The chain of elements before the carrier of a link (T-303).
//!
//! `podnet link dial` and `podnet link connect` take each `--egress SPEC`
//! as one element. The elements apply in order: the first element is the
//! first hop from this host, and the last element reaches the carrier
//! address. There is no `direct` at the end unless the user gives it.
//!
//! The chain is a [`Chain`] of [`crate::network`]: each element is one
//! layer, and it comes up over the way of the layer below. An element is:
//!
//! * `direct`: only as the first element;
//! * a proxy (`socks5://`, `socks5h://`, `http://`): a SOCKS5 or HTTP
//!   CONNECT hop. It opens its connection over the layer below. A hop on
//!   a unix socket is only the first element;
//! * `peer:FINGERPRINT/SERVICE`: one stream to a service of a peer (T-266).
//!   Here the stream is the connection to the next element or to the
//!   carrier address, so the service of the peer must lead there. It is
//!   only the first element.
//!
//! Limits: [`MAX_ELEMENTS`] elements, [`ELEMENT_LIMIT`] for each one. A
//! duplicate element or a cycle exits 78 ([`Plan::parse`]). A chain that
//! cannot be built exits 69 and names the element ([`Plan::open`]), before
//! the carrier opens. [`Plan::check`] resolves and dials each element once.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use podnet_netns::upstream::{self, Handshake, Proxy as Hop, ProxyKind};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::exit::{self, Fault};
use crate::network::{
    BoxFuture, Caps, Chain, Connected, Ctx, Dial, Failure, Link, Network, Stage, Target, Way,
};
use crate::networks::{self, SpecNetwork};

/// The most elements of a chain.
pub const MAX_ELEMENTS: usize = 8;
/// The time for each element: to come up, to dial.
pub const ELEMENT_LIMIT: Duration = Duration::from_secs(10);

/// One element of a chain.
struct Element {
    /// The place in the chain, from 1.
    n: usize,
    /// The spec with no credentials.
    label: String,
    net: SpecNetwork,
    /// The hop, for a proxy that can ride over a way below it.
    hop: Option<Hop>,
    /// The place that the element names, for a cycle check and for the
    /// dial of the element above: `HOST:PORT`, `unix:PATH` or
    /// `peer:FINGERPRINT`. `direct` has none.
    endpoint: Option<String>,
}

/// The elements of a chain, checked and ready to come up.
pub struct Plan {
    elements: Vec<Element>,
    /// The `HOST:PORT` of the carrier.
    carrier: String,
}

/// The result of one element of [`Plan::check`].
#[derive(Clone, Debug)]
pub struct Checked {
    pub n: usize,
    pub element: String,
    pub resolved: String,
    pub dial_ms: u128,
}

impl Checked {
    /// The line of the element for a person.
    pub fn line(&self, of: usize) -> String {
        format!(
            "podnet: element {} of {of}  {}  resolved {}  dialed in {} ms",
            self.n, self.element, self.resolved, self.dial_ms
        )
    }

    /// The element for a JSON document.
    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "n": self.n,
            "element": self.element,
            "resolved": self.resolved,
            "dial_ms": self.dial_ms,
        })
    }
}

/// A chain that is up: dial the carrier address through it.
pub struct Opened {
    dial: Arc<dyn Dial>,
    _link: Link,
}

impl Opened {
    /// The way out of the carrier.
    pub fn dial(&self) -> Arc<dyn Dial> {
        self.dial.clone()
    }
}

fn usage(text: String) -> anyhow::Error {
    exit::fault(Fault::Usage, anyhow!(text))
}

/// The `HOST:PORT` of a carrier address (`KEY@HOST:PORT` or `HOST:PORT`).
fn carrier_place(address: &str) -> String {
    let place = address.rsplit_once('@').map_or(address, |(_, place)| place);
    place.trim().to_ascii_lowercase()
}

impl Plan {
    /// Check `specs` (one for each element) for a carrier at `address`.
    /// A spec that cannot be parsed, a network that is not an element, or
    /// more than [`MAX_ELEMENTS`] elements exit 64. A duplicate element,
    /// two elements at one place, or an element at the place of the
    /// carrier (a cycle) exit 78. Nothing is dialed.
    pub fn parse(specs: &[String], address: &str) -> Result<Plan> {
        if specs.len() > MAX_ELEMENTS {
            return Err(usage(format!(
                "--egress: a link chain has at most {MAX_ELEMENTS} elements, and {} are given",
                specs.len()
            )));
        }
        let mut elements = Vec::new();
        for (i, spec) in specs.iter().enumerate() {
            elements.push(element(i + 1, spec.trim())?);
        }
        let carrier = carrier_place(address);
        for (i, e) in elements.iter().enumerate() {
            if let Some(first) = elements[..i].iter().find(|o| o.label == e.label) {
                return Err(exit::fault(
                    Fault::Config,
                    anyhow!(
                        "--egress: element {} ({}) repeats element {}: a chain has no duplicate element",
                        e.n, e.label, first.n
                    ),
                ));
            }
            let Some(place) = &e.endpoint else { continue };
            let cycle = |why: String| exit::fault(Fault::Config, anyhow!("--egress: {why}"));
            if let Some(first) = elements[..i]
                .iter()
                .find(|o| o.endpoint.as_ref() == Some(place))
            {
                return Err(cycle(format!(
                    "a cycle: element {} ({}) and element {} ({}) name the same place",
                    first.n, first.label, e.n, e.label
                )));
            }
            if *place == carrier {
                return Err(cycle(format!(
                    "a cycle: element {} ({}) is the carrier address",
                    e.n, e.label
                )));
            }
        }
        Ok(Plan { elements, carrier })
    }

    /// The count of the elements.
    pub fn len(&self) -> usize {
        self.elements.len()
    }

    /// True when the chain has no element.
    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    /// The labels of the elements, in order.
    pub fn labels(&self) -> Vec<String> {
        self.elements.iter().map(|e| e.label.clone()).collect()
    }

    fn chain(&self, upto: usize) -> Result<Chain> {
        let count = self.elements.len();
        let layers = self.elements[..upto]
            .iter()
            .map(|e| {
                Arc::new(Layer {
                    n: e.n,
                    count,
                    label: e.label.clone(),
                    net: e.net.clone(),
                    hop: e.hop.clone(),
                }) as Arc<dyn Network>
            })
            .collect();
        Chain::new(layers).map_err(|f| anyhow!("{}", f.text()))
    }

    /// Resolve each element, then bring the chain up. An element that
    /// cannot be built exits 69 and is named, before any carrier opens.
    pub async fn open(&self) -> Result<Opened> {
        for e in &self.elements {
            e.resolve().await?;
        }
        let link = self
            .chain(self.elements.len())?
            .up(&Ctx::default())
            .await
            .map_err(unavailable)?;
        let way = link.way.clone().expect("a chain gives a way");
        Ok(Opened {
            dial: way.dial,
            _link: link,
        })
    }

    /// Resolve each element and dial each one once: the dial of element
    /// `n` goes through elements 1 to `n` to the place of element `n + 1`
    /// (the last one: to the carrier address) and ends at once. A failure
    /// exits 69 and names the element.
    pub async fn check(&self) -> Result<Vec<Checked>> {
        let mut out = Vec::new();
        for (i, e) in self.elements.iter().enumerate() {
            let resolved = e.resolve().await?;
            let place = match self.elements.get(i + 1) {
                Some(next) => next.endpoint.clone().unwrap_or_default(),
                None => self.carrier.clone(),
            };
            let target = target_of(&place).ok_or_else(|| {
                exit::fault(
                    Fault::Unavailable,
                    anyhow!("element {} ({}): no place to dial: {place}", e.n, e.label),
                )
            })?;
            let started = Instant::now();
            let dial = async {
                let link = self.chain(i + 1)?.up(&Ctx::default()).await?;
                let way = link.way.clone().expect("a chain gives a way");
                let end = tokio::time::Instant::now() + ELEMENT_LIMIT;
                let done = way.dial.dial(&target, end).await;
                link.close().await;
                done.map(drop).map_err(|f| anyhow!("{}", f.text()))
            };
            match tokio::time::timeout(ELEMENT_LIMIT, dial).await {
                Ok(Ok(())) => {}
                Ok(Err(err)) => {
                    return Err(unavailable(err.context(format!(
                        "element {} of {} ({}): the dial of {place}",
                        e.n,
                        self.elements.len(),
                        e.label
                    ))))
                }
                Err(_) => {
                    return Err(exit::fault(
                        Fault::Unavailable,
                        anyhow!(
                            "element {} of {} ({}): no dial of {place} in {} s",
                            e.n,
                            self.elements.len(),
                            e.label,
                            ELEMENT_LIMIT.as_secs()
                        ),
                    ))
                }
            }
            out.push(Checked {
                n: e.n,
                element: e.label.clone(),
                resolved,
                dial_ms: started.elapsed().as_millis(),
            });
        }
        Ok(out)
    }
}

/// An error with the class 69, unless it has a class.
fn unavailable(err: anyhow::Error) -> anyhow::Error {
    if exit::code_of(&err) == exit::FAILURE {
        exit::fault(Fault::Unavailable, err)
    } else {
        err
    }
}

/// `HOST:PORT` as a target. A unix place has none.
fn target_of(place: &str) -> Option<Target> {
    let (host, port) = place.rsplit_once(':')?;
    let host = host.trim_start_matches('[').trim_end_matches(']');
    Some(Target::new(host, port.parse().ok()?))
}

fn element(n: usize, spec: &str) -> Result<Element> {
    let net = networks::parse(spec).map_err(|f| {
        usage(format!(
            "--egress element {n}: {}",
            f.text().trim_start_matches("--egress: ")
        ))
    })?;
    let label = net.label();
    let scheme = net.scheme();
    if !matches!(scheme, "direct" | "proxy" | "peer") {
        return Err(usage(format!(
            "--egress element {n} ({label}): a {scheme} network is not an element of a link chain; give direct, a proxy (socks5://, http://) or peer:FINGERPRINT/SERVICE"
        )));
    }
    let mut hop = None;
    let mut endpoint = None;
    if scheme == "proxy" {
        let parsed = Hop::parse(spec)
            .map_err(|e| usage(format!("--egress element {n} ({label}): {e:#}")))?;
        endpoint = Some(parsed.endpoint().to_ascii_lowercase());
        let rides = matches!(parsed.kind, ProxyKind::Socks5 | ProxyKind::Http)
            && parsed.unix_path.is_none();
        hop = rides.then_some(parsed);
    } else if let Some((fingerprint, _)) = net.peer() {
        endpoint = Some(format!("peer:{fingerprint}"));
    }
    Ok(Element {
        n,
        label,
        net,
        hop,
        endpoint,
    })
}

impl Element {
    /// Resolve the place of the element: a name gives an address, a
    /// unix socket exists, a peer runs a link. Within [`ELEMENT_LIMIT`].
    /// The text is the result. A failure exits 69 and names the element.
    async fn resolve(&self) -> Result<String> {
        let name = format!("element {} ({})", self.n, self.label);
        let fail = |why: String| exit::fault(Fault::Unavailable, anyhow!("{name}: {why}"));
        let Some(place) = &self.endpoint else {
            return Ok("this host".into());
        };
        if let Some(path) = place.strip_prefix("unix:") {
            return match std::fs::metadata(path) {
                Ok(_) => Ok(place.clone()),
                Err(e) => Err(fail(format!("the socket {path} is not there: {e}"))),
            };
        }
        if let Some(fingerprint) = place.strip_prefix("peer:") {
            let probe = crate::link_ctl::probe_peer(fingerprint);
            return match tokio::time::timeout(ELEMENT_LIMIT, probe).await {
                Ok(Ok(())) => Ok(format!("a link to {fingerprint}")),
                Ok(Err(e)) => Err(unavailable(e.context(name))),
                Err(_) => Err(fail(format!("no answer in {} s", ELEMENT_LIMIT.as_secs()))),
            };
        }
        let found = tokio::time::timeout(ELEMENT_LIMIT, tokio::net::lookup_host(place.as_str()));
        match found.await {
            Ok(Ok(mut addrs)) => match addrs.next() {
                Some(addr) => Ok(addr.to_string()),
                None => Err(fail(format!("{place} gives no address"))),
            },
            Ok(Err(e)) => Err(fail(format!("{place} does not resolve: {e}"))),
            Err(_) => Err(fail(format!(
                "{place} does not resolve in {} s",
                ELEMENT_LIMIT.as_secs()
            ))),
        }
    }
}

/// One element as a network: a layer of the chain.
struct Layer {
    n: usize,
    count: usize,
    label: String,
    net: SpecNetwork,
    hop: Option<Hop>,
}

impl Network for Layer {
    fn scheme(&self) -> &'static str {
        self.net.scheme()
    }

    fn caps(&self) -> Caps {
        Caps::EGRESS_TCP
    }

    /// The first element is the network of its spec. A later element
    /// is a proxy that opens its connection over the way below it. A
    /// peer stream is a way to a destination here: the layer above
    /// asks the service of the peer for its place.
    fn up<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Link>> {
        Box::pin(async move {
            let name = format!("element {} of {} ({})", self.n, self.count, self.label);
            let up = async {
                if let (Some(below), Some(hop)) = (&ctx.below, &self.hop) {
                    let dial = Over {
                        hop: hop.clone(),
                        label: self.label.clone(),
                        below: below.clone(),
                    };
                    return Ok::<Link, anyhow::Error>(Link::new(Some(Way::destination(Arc::new(
                        dial,
                    )))));
                }
                let link = self.net.up(ctx).await?;
                match (self.net.peer(), link.way.clone()) {
                    (Some(_), Some(way)) => {
                        Ok(Link::new(Some(Way::destination(way.dial))).with_status(link.status))
                    }
                    _ => Ok(link),
                }
            };
            match tokio::time::timeout(ELEMENT_LIMIT, up).await {
                Ok(Ok(link)) => Ok(link),
                Ok(Err(e)) => Err(unavailable(e.context(name))),
                Err(_) => Err(exit::fault(
                    Fault::Unavailable,
                    anyhow!("{name}: not up in {} s", ELEMENT_LIMIT.as_secs()),
                )),
            }
        })
    }
}

/// A proxy hop over the way below it.
struct Over {
    hop: Hop,
    label: String,
    below: Arc<dyn Dial>,
}

fn io_failure(label: &str, e: impl std::fmt::Display) -> Failure {
    Failure::new(
        Stage::Proxy,
        crate::network::Class::Unavailable,
        format!("{label}: {e:#}"),
    )
}

impl Dial for Over {
    fn label(&self) -> String {
        format!("{} over {}", self.label, self.below.label())
    }

    fn dial<'a>(
        &'a self,
        target: &'a Target,
        deadline: tokio::time::Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(async move {
            let own = Target::new(self.hop.host.as_str(), self.hop.port);
            let end = deadline.min(tokio::time::Instant::now() + ELEMENT_LIMIT);
            let connected = self.below.dial(&own, end).await?;
            let to = target.to_string();
            let io: upstream::Upstream = Box::new(connected.io);
            let shake = async {
                match self.hop.kind {
                    ProxyKind::Socks5 => {
                        upstream::socks5_hop(io, &self.hop, &to, Handshake::Serial).await
                    }
                    _ => http_connect(io, &self.hop, &to).await,
                }
            };
            match tokio::time::timeout_at(end, shake).await {
                Ok(Ok(io)) => Ok(Connected::new(Box::new(io))),
                Ok(Err(e)) => Err(io_failure(&self.label, e)),
                Err(_) => Err(io_failure(&self.label, "no answer before the limit")),
            }
        })
    }
}

/// HTTP CONNECT to `target` over `io`, with Basic credentials if the hop has
/// them. Only the status line and the head are read: no byte of the tunnel.
async fn http_connect<S>(mut io: S, hop: &Hop, target: &str) -> Result<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut head = format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n");
    if let (Some(user), Some(pass)) = (&hop.user, &hop.pass) {
        let creds = upstream::base64(format!("{user}:{pass}").as_bytes());
        head.push_str(&format!("Proxy-Authorization: Basic {creds}\r\n"));
    }
    head.push_str("\r\n");
    io.write_all(head.as_bytes()).await?;
    // Byte by byte: a read ahead would take bytes of the tunnel.
    let mut seen = Vec::new();
    let mut byte = [0u8; 1];
    while !seen.ends_with(b"\r\n\r\n") {
        if io.read(&mut byte).await? == 0 {
            return Err(anyhow!("the proxy closed during CONNECT"));
        }
        seen.push(byte[0]);
        if seen.len() > 8192 {
            return Err(anyhow!("the CONNECT answer is too large"));
        }
    }
    let text = String::from_utf8_lossy(&seen);
    let line = text.lines().next().unwrap_or("");
    if line.split_whitespace().nth(1) != Some("200") {
        return Err(anyhow!("CONNECT to {target} failed: {line}"));
    }
    Ok(io)
}
