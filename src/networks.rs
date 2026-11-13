//! The registry of networks and the parser of network specs (T-059).
//!
//! A spec names one network in the form of `podnet pod -x`: `direct`, a
//! proxy URL (`socks5h://HOST:PORT`, `http://HOST:PORT`,
//! `SCHEME+unix:/PATH`, `SCHEME+exec:COMMAND`), a hop of the pod
//! (`unix:/PATH`, `HOST:PORT`, `socks://`, `https://`), `tailscale:/PATH`,
//! `netbird:[PORT]`, `wg:FILE` (WireGuard), `devtunnel://TUNNEL` (TCP through
//! a Dev Tunnels relay), `turn://HOST[:PORT]` and
//! `turns://HOST[:PORT]` (UDP through a TURN server), or the relay (`relay`,
//! `relay:URL`). [`networks`] is the table of the
//! networks of this build: a static table in the binary, with no
//! dependency (`docs/research/podnet/architecture.md`, section 4.3).
//!
//! Each form of the two old parsers stays: the proxy of the process
//! ([`crate::egress::Proxy`], with Digest, `socks5h` and the commands) and
//! the hop of the pod (`podnet_netns::upstream::Proxy`). A spec that both
//! take goes to the proxy of the process. A spec with no scheme
//! (`HOST:PORT`) is a SOCKS5 hop, as `pod -x` reads it.

use std::sync::Arc;

use podnet_vnet::dialer::UpstreamChain;

use crate::dialers::{Direct, RelayDialer};
use crate::network::{
    BoxFuture, Caps, Check, Class, Ctx, Dial, Failure, Link, Network, Stage, Target, Way,
};
use crate::ws_relay::Relay;

mod devtunnel;
mod netbird;
mod peer;
pub use netbird::{greeting as netbird_greeting, port_from_env as netbird_port};
mod tailscale;
mod turn;
mod turn_socket;
mod wg;
pub use tailscale::{rows as tailscale_rows, state as tailscale_state};

/// One row of the registry: a scheme, its forms and what it gives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    /// The scheme of [`Network::scheme`].
    pub scheme: &'static str,
    /// The forms of a spec of this scheme.
    pub forms: &'static [&'static str],
    /// What the network is, in one line.
    pub about: &'static str,
    /// What the network gives.
    pub caps: Caps,
}

const ROWS: [Row; 9] = [
    Row {
        scheme: "direct",
        forms: &["direct"],
        about: "TCP and UDP from this host, with no proxy",
        caps: Caps::EGRESS_TCP.with(Caps::EGRESS_UDP),
    },
    Row {
        scheme: "proxy",
        forms: &[
            "http://[USER:PASS@]HOST[:PORT]",
            "socks5://[USER:PASS@]HOST[:PORT]",
            "socks5h://[USER:PASS@]HOST[:PORT]",
            "SCHEME+unix:/PATH",
            "SCHEME://unix:/PATH",
            "SCHEME+exec:COMMAND",
            "unix:/PATH",
            "HOST:PORT",
        ],
        about: "TCP through an HTTP CONNECT or a SOCKS5 proxy",
        caps: Caps::EGRESS_TCP,
    },
    Row {
        scheme: "relay",
        forms: &["relay", "relay:URL", "relay:HOST"],
        about: "TCP to an IPv4 address through the WebSocket relay",
        caps: Caps::EGRESS_TCP,
    },
    Row {
        scheme: "tailscale",
        forms: &["tailscale:/PATH/tailscaled.sock"],
        about: "TCP into a tailnet through the LocalAPI of tailscaled",
        caps: Caps::EGRESS_TCP,
    },
    Row {
        scheme: "netbird",
        forms: &["netbird", "netbird:[PORT]"],
        about: "TCP to a peer address through the SOCKS5 listener of NetBird netstack mode",
        caps: Caps::EGRESS_TCP,
    },
    Row {
        scheme: "wg",
        forms: &["wg:FILE"],
        about: "TCP and UDP through a WireGuard peer of a wg-quick file, on a userspace stack",
        caps: Caps::EGRESS_TCP.with(Caps::EGRESS_UDP),
    },
    Row {
        scheme: "turn",
        forms: &["turn://HOST[:PORT]", "turns://HOST[:PORT]"],
        about: "UDP through an allocation on a TURN server (RFC 8656), over UDP (turn) or over TLS on TCP (turns); TCP flows are not carried",
        caps: Caps::EGRESS_UDP,
    },
    Row {
        scheme: "devtunnel",
        forms: &["devtunnel://NAME", "devtunnel://ID.CLUSTER"],
        about: "TCP to a port of the host of a Microsoft Dev Tunnels tunnel, through the relay (WebSocket and SSH); the target is 127.0.0.1:PORT of the tunnel host",
        caps: Caps::EGRESS_TCP,
    },
    Row {
        scheme: "peer",
        forms: &["peer:FINGERPRINT/SERVICE"],
        about: "one stream to a service of a peer, over a link that a running podnet link instance holds (a peer way: it meets a peer, it dials no destination)",
        caps: Caps::EGRESS_TCP,
    },
];

/// The networks of this build, in the order of their parse.
pub fn networks() -> &'static [Row] {
    &ROWS
}

/// What a spec names.
#[derive(Clone, Debug)]
enum Kind {
    Direct,
    /// The proxy of the process.
    Proxy(crate::egress::Proxy),
    /// One hop of the pod (`UpstreamChain`): the forms that only `pod -x`
    /// takes, and `tailscale:`.
    Hop {
        tailscale: bool,
    },
    /// The relay of this URL; `None`: of `PODNET_RELAY`, else the default.
    Relay(Option<String>),
    /// The SOCKS5 listener of NetBird netstack mode on 127.0.0.1:PORT.
    Netbird(u16),
    /// The WireGuard peer of this wg-quick file (T-117).
    Wg(std::path::PathBuf),
    /// The TURN server of this spec (T-321).
    Turn(turn::Spec),
    /// The Dev Tunnels tunnel of this spec (T-320).
    Devtunnel(devtunnel::Spec),
    /// A service of the peer with this fingerprint (T-266).
    Peer {
        fingerprint: String,
        service: String,
    },
}

/// A network that a spec names.
#[derive(Clone, Debug)]
pub struct SpecNetwork {
    spec: String,
    kind: Kind,
}

fn usage(text: String) -> Failure {
    Failure::new(Stage::Connect, Class::Usage, text)
}

/// Parse `spec`. The error is one line that names the spec.
pub fn parse(spec: &str) -> Result<SpecNetwork, Failure> {
    let spec = spec.trim();
    let kind = kind(spec)?;
    Ok(SpecNetwork {
        spec: spec.to_string(),
        kind,
    })
}

fn kind(spec: &str) -> Result<Kind, Failure> {
    if spec.is_empty() {
        return Err(usage("the network spec is empty".into()));
    }
    if spec == "direct" {
        return Ok(Kind::Direct);
    }
    if spec == "relay" {
        return Ok(Kind::Relay(None));
    }
    if let Some(url) = spec.strip_prefix("relay:") {
        let url = if url.contains("://") {
            url.to_string()
        } else {
            format!("https://{url}")
        };
        Relay::new(&url, None).map_err(|e| usage(format!("{spec}: {e:#}")))?;
        return Ok(Kind::Relay(Some(url)));
    }
    if spec == "netbird" || spec.starts_with("netbird:") {
        let port = spec["netbird".len()..].trim_start_matches(':');
        let port = if port.is_empty() {
            netbird::DEFAULT_PORT
        } else {
            port.parse::<u16>()
                .ok()
                .filter(|p| *p != 0)
                .ok_or_else(|| usage(format!("{spec}: the port must be 1 to 65535")))?
        };
        return Ok(Kind::Netbird(port));
    }
    if turn::is_turn(spec) {
        return turn::parse(spec).map(Kind::Turn);
    }
    if devtunnel::is_devtunnel(spec) {
        return devtunnel::parse(spec).map(Kind::Devtunnel);
    }
    if let Some(rest) = spec.strip_prefix("peer:") {
        // `peer:PORT` is the SOCKS hop of a host named peer, as it was.
        if rest.parse::<u16>().is_err() {
            return peer::parse(spec, rest);
        }
    }
    if let Some(path) = wg::file_of(spec) {
        if path.as_os_str().is_empty() {
            return Err(usage(format!("{spec}: give the path of a wg-quick file")));
        }
        wg::check(&path)?;
        return Ok(Kind::Wg(path));
    }
    if spec == "warp" || spec.starts_with("warp:") {
        return Err(Failure::new(
            Stage::Connect,
            Class::Unsupported,
            format!("{spec}: WARP is not a network of the registry yet; use podnet warp proxy or tunnel --transport warp"),
        ));
    }
    let hop = |spec: &str| podnet_netns::upstream::Proxy::parse(spec);
    if spec.starts_with("tailscale:") {
        hop(spec).map_err(|e| usage(format!("{spec}: {e:#}")))?;
        return Ok(Kind::Hop { tailscale: true });
    }
    let schemed = spec.contains("://") || spec.contains("+exec:") || spec.contains("+unix:");
    if spec.starts_with("unix:") || !schemed {
        hop(spec).map_err(|e| usage(format!("{spec}: {e:#}")))?;
        return Ok(Kind::Hop { tailscale: false });
    }
    match crate::egress::Proxy::parse_trusted(spec) {
        Ok(proxy) => Ok(Kind::Proxy(proxy)),
        Err(why) => match hop(spec) {
            Ok(_) => Ok(Kind::Hop { tailscale: false }),
            Err(_) => Err(usage(format!("{}: {why}", redacted(spec)))),
        },
    }
}

/// `spec` with no user and password: a spec can go into an error line.
fn redacted(spec: &str) -> String {
    let (scheme, rest) = match spec.split_once("://") {
        Some((scheme, rest)) => (format!("{scheme}://"), rest),
        None => (String::new(), spec),
    };
    match rest.rsplit_once('@') {
        Some((_, host)) => format!("{scheme}***@{host}"),
        None => spec.to_string(),
    }
}

impl SpecNetwork {
    /// The spec with no credentials.
    pub fn label(&self) -> String {
        match &self.kind {
            Kind::Proxy(proxy) => proxy.label(),
            Kind::Netbird(port) => format!("netbird:{port}"),
            Kind::Wg(path) => format!("wg:{}", path.display()),
            Kind::Turn(spec) => spec.label(),
            Kind::Devtunnel(spec) => spec.label(),
            _ => redacted(&self.spec),
        }
    }

    /// The URL of the relay network: `Some(None)` for `relay` (the relay of
    /// `PODNET_RELAY`, else the default), `Some(Some(URL))` for `relay:URL`,
    /// `None` for each other network.
    pub fn relay_url(&self) -> Option<Option<&str>> {
        match &self.kind {
            Kind::Relay(url) => Some(url.as_deref()),
            _ => None,
        }
    }

    /// The port of the NetBird listener, for `netbird:[PORT]`.
    pub fn netbird_port(&self) -> Option<u16> {
        match self.kind {
            Kind::Netbird(port) => Some(port),
            _ => None,
        }
    }

    /// The TURN server and the transport, for `turn://` and `turns://`:
    /// `(HOST:PORT, tls)`.
    pub fn turn(&self) -> Option<(String, bool)> {
        match &self.kind {
            Kind::Turn(spec) => Some((spec.server.to_string(), spec.tls)),
            _ => None,
        }
    }

    /// The tunnel (`NAME` or `ID.CLUSTER`), for `devtunnel://TUNNEL`.
    pub fn devtunnel(&self) -> Option<String> {
        match &self.kind {
            Kind::Devtunnel(spec) => Some(spec.locator.to_string()),
            _ => None,
        }
    }

    /// The wg-quick file, for `wg:FILE`.
    pub fn wg_file(&self) -> Option<&std::path::Path> {
        match &self.kind {
            Kind::Wg(path) => Some(path),
            _ => None,
        }
    }

    /// The fingerprint and the service of `peer:FINGERPRINT/SERVICE`.
    pub fn peer(&self) -> Option<(&str, &str)> {
        match &self.kind {
            Kind::Peer {
                fingerprint,
                service,
            } => Some((fingerprint, service)),
            _ => None,
        }
    }

    /// The socket of `tailscaled`, for `tailscale:/PATH`.
    pub fn tailscale_socket(&self) -> Option<String> {
        match self.kind {
            Kind::Hop { tailscale: true } => podnet_netns::upstream::Proxy::parse(&self.spec)
                .ok()
                .and_then(|p| p.unix_path),
            _ => None,
        }
    }

    /// True when the spec is a proxy of the process (`--proxy`), not a hop
    /// that only `podnet pod -x` takes.
    pub fn is_process_proxy(&self) -> bool {
        matches!(self.kind, Kind::Proxy(_))
    }

    /// The row of this network in [`networks`].
    pub fn row(&self) -> &'static Row {
        let scheme = self.scheme();
        ROWS.iter()
            .find(|r| r.scheme == scheme)
            .expect("each kind has a row")
    }

    fn dial(&self) -> Result<Arc<dyn Dial>, Failure> {
        Ok(match &self.kind {
            Kind::Direct => Arc::new(Direct),
            Kind::Proxy(proxy) => crate::dialers::proxy(proxy.clone()),
            Kind::Hop { .. } => Arc::new(
                UpstreamChain::parse(&[self.spec.as_str()])
                    .map_err(|e| usage(format!("{}: {e:#}", self.label())))?,
            ),
            Kind::Wg(_) | Kind::Turn(_) | Kind::Devtunnel(_) => {
                unreachable!("wg:FILE, turn:// and devtunnel:// come up in `up`")
            }
            Kind::Peer {
                fingerprint,
                service,
            } => Arc::new(peer::PeerDial::new(fingerprint, service)),
            Kind::Netbird(port) => Arc::new(
                netbird::NetbirdDialer::new(*port)
                    .map_err(|e| usage(format!("{}: {e:#}", self.label())))?,
            ),
            Kind::Relay(url) => {
                let relay = Relay::from_env(url.as_deref())
                    .map_err(|e| usage(format!("{}: {e:#}", self.label())))?;
                Arc::new(RelayDialer::of_process(Arc::new(relay)))
            }
        })
    }
}

impl Network for SpecNetwork {
    fn scheme(&self) -> &'static str {
        match self.kind {
            Kind::Direct => "direct",
            Kind::Proxy(_) | Kind::Hop { tailscale: false } => "proxy",
            Kind::Hop { tailscale: true } => "tailscale",
            Kind::Relay(_) => "relay",
            Kind::Netbird(_) => "netbird",
            Kind::Wg(_) => "wg",
            Kind::Turn(_) => "turn",
            Kind::Devtunnel(_) => "devtunnel",
            Kind::Peer { .. } => "peer",
        }
    }

    fn caps(&self) -> Caps {
        self.row().caps
    }

    /// The way of the spec. These networks dial from this host: a way
    /// below them (`ctx.below`) is refused, not ignored. `wg:FILE` and
    /// `turn://` send their UDP over the datagrams of that way (`turns://`
    /// opens its TCP connection over it).
    fn up<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, anyhow::Result<Link>> {
        Box::pin(async move {
            if let Kind::Wg(path) = &self.kind {
                let (_, link) = wg::up(path, self.label(), ctx.below.as_ref()).await?;
                return Ok(link);
            }
            if let Kind::Turn(spec) = &self.kind {
                // The account comes from the environment, now and not at the
                // parse: a parse reads no variable.
                let account = turn::account(&|name| std::env::var(name).ok())?;
                return Ok(turn::up(spec, account, ctx.below.as_ref(), Default::default()).await?);
            }
            if let Kind::Devtunnel(spec) = &self.kind {
                // The token comes from the environment, now and not at the
                // parse: a parse reads no variable.
                let account = devtunnel::account(&|name| std::env::var(name).ok())?;
                return Ok(devtunnel::up(spec, account, ctx.below.as_ref()).await?);
            }
            if let Some(below) = &ctx.below {
                return Err(Failure::new(
                    Stage::Connect,
                    Class::Unsupported,
                    format!(
                        "{} cannot go over another way ({}) yet",
                        self.label(),
                        below.label()
                    ),
                )
                .into());
            }
            if let Kind::Peer { fingerprint, .. } = &self.kind {
                // Check the link before the first stream: a peer that is
                // not the named fingerprint exits 77, none that runs 69.
                crate::link_ctl::probe_peer(fingerprint).await?;
                let status = serde_json::json!({"scheme": "peer", "spec": self.label()});
                return Ok(Link::new(Some(Way::peer(self.dial()?))).with_status(status));
            }
            let way = Way::destination(self.dial()?);
            let mut status = serde_json::json!({
                "scheme": self.scheme(),
                "spec": self.label(),
            });
            if let Some(socket) = self.tailscale_socket() {
                // T-115: the login state. A tailnet that is not Running is
                // a warning, not a fault: the daemon can log in later.
                let state = tailscale::word(&socket).await;
                if state != "Running" {
                    eprintln!(
                        "podnet: {}: tailscaled is {state}, not Running: each dial fails until it is logged in (tailscale up; tailscale status)",
                        self.label()
                    );
                }
                status["state"] = state.into();
            }
            Ok(Link::new(Some(way)).with_status(status))
        })
    }

    /// The proxy on TCP, and the relay: the host that the network must
    /// reach first.
    fn checks(&self) -> Vec<Check> {
        match &self.kind {
            Kind::Proxy(proxy) if matches!(proxy.carrier(), crate::egress::Carrier::Tcp) => {
                let (host, port) = proxy.address();
                vec![Check {
                    name: "proxy".into(),
                    target: Target::new(host, port),
                    tls: false,
                }]
            }
            Kind::Turn(spec) if spec.tls => vec![Check {
                name: "turn".into(),
                target: spec.server.clone(),
                tls: true,
            }],
            Kind::Devtunnel(_) => devtunnel::checks(),
            Kind::Netbird(port) => vec![Check {
                name: "netbird".into(),
                target: Target::new("127.0.0.1", *port),
                tls: false,
            }],
            Kind::Relay(url) => match Relay::from_env(url.as_deref()) {
                Ok(relay) => vec![Check {
                    name: "relay".into(),
                    target: Target::new(relay.host(), relay.port()),
                    tls: relay.url().starts_with("https://"),
                }],
                Err(_) => Vec::new(),
            },
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod devtunnel_tests;
#[cfg(test)]
mod tailscale_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod turn_mock;
#[cfg(test)]
mod turn_tests;
#[cfg(test)]
mod wg_tests;

/// The peer way of `peer:FINGERPRINT/SERVICE`, with no link check: the
/// dial of each stream asks the running instance (T-288). A link group
/// builds its members from it, so that a member exists while its link is
/// down.
pub fn peer_way(fingerprint: &str, service: &str) -> Way {
    Way::peer(Arc::new(peer::PeerDial::new(fingerprint, service)))
}
