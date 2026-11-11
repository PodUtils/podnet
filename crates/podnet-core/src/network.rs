//! The network types of podnet (T-055): a target, a dialer, datagrams, a
//! network and its link.
//!
//! A network gives egress (a way out for TCP and UDP flows), ingress, an
//! export (a configuration for another program), or a mix
//! (`docs/research/podnet/architecture.md`, section 4.3).
//!
//! The target, the dialer and the datagrams are the types of `pod-core`
//! (PodUtils/podcrates). This module gives them again, and adds no second
//! copy:
//!
//! * [`Target`]: a name and a port, or an address;
//! * [`Dial`]: a way out, with `label`, `dial` (TCP), `datagrams` (UDP) and
//!   `names_inside`;
//! * [`Datagrams`]: a poll to send and a poll to receive, in the shape of
//!   the UDP socket of quinn.
//!
//! This module adds:
//!
//! * [`Addressing`] and [`Way`]: the addressing mode of a dialer. A
//!   dialer dials a destination (a host and a port), or it meets a peer (a
//!   name or a shared secret, with no port; GitHub #377). A consumer that
//!   dials a destination refuses a peer dialer ([`Addressing::admit`]);
//! * [`Network`], [`Link`], [`Caps`], [`Ctx`], [`Export`] and [`Check`]:
//!   an adapter, and what it gives when it is up;
//! * the combinators (T-058): [`Chain`] (an outer network carries an inner
//!   one), [`Rules`] (a way for each target), [`Race`] (staggered
//!   attempts) and [`Failover`] (the next way after an error).

use std::fmt;
use std::ops::BitOr;
use std::sync::Arc;

pub mod chain;
pub mod failover;
pub mod race;
pub mod rules;

pub use chain::Chain;
pub use failover::Failover;
pub use race::Race;
pub use rules::{matcher, Choice, Matcher, Rule, Rules};

pub use pod_core::datagrams::{recv_from, send_to};
pub use pod_core::{
    BoxFuture, BoxedIo, Class, Connected, Datagrams, Dial, Failure, RecvMeta, Stage,
    SystemDatagrams, Target, Transmit,
};

/// The addressing mode of a dialer and of a consumer of dialers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Addressing {
    /// The host and the port of a [`Target`] name a destination that the
    /// dialer connects to. Direct, a proxy, WARP and the relay are of this
    /// mode.
    Destination,
    /// The host of a [`Target`] names a peer, or a secret that the two ends
    /// share. The port is 0 ([`peer`]). The dialer meets the peer that
    /// holds the same name.
    Peer,
}

impl Addressing {
    /// The name of the mode in a log line: `destination` or `peer`.
    pub fn name(self) -> &'static str {
        match self {
            Addressing::Destination => "destination",
            Addressing::Peer => "peer",
        }
    }

    /// Check each way of a chain for a consumer of this mode, before the
    /// first dial. A consumer that dials a destination refuses a peer
    /// way: the peer way reads the destination as a peer name, and the
    /// flow goes to a wrong place. A consumer that dials a peer takes each
    /// way. The error is one line that names the way.
    pub fn admit(self, ways: &[Way]) -> Result<(), Failure> {
        if self == Addressing::Peer {
            return Ok(());
        }
        match ways.iter().find(|w| w.addressing == Addressing::Peer) {
            None => Ok(()),
            Some(way) => Err(Failure::new(
                Stage::Connect,
                Class::Usage,
                format!(
                    "{} meets a peer by a name or a shared secret, but this consumer dials a \
                     destination (a host and a port): remove the way from this chain",
                    way.label()
                ),
            )),
        }
    }
}

impl fmt::Display for Addressing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// The target of a peer dialer: the peer name in the host, and port 0.
/// Do not put a secret in it: the target goes into log lines. A peer
/// dialer that meets by a secret holds the secret in its own settings.
pub fn peer(name: &str) -> Target {
    Target::Name {
        host: name.to_string(),
        port: 0,
    }
}

/// A dialer and its addressing mode.
#[derive(Clone)]
pub struct Way {
    pub dial: Arc<dyn Dial>,
    pub addressing: Addressing,
}

impl Way {
    /// A dialer that dials a destination.
    pub fn destination(dial: Arc<dyn Dial>) -> Way {
        Way {
            dial,
            addressing: Addressing::Destination,
        }
    }

    /// A dialer that meets a peer.
    pub fn peer(dial: Arc<dyn Dial>) -> Way {
        Way {
            dial,
            addressing: Addressing::Peer,
        }
    }

    /// The label of the dialer.
    pub fn label(&self) -> String {
        self.dial.label()
    }
}

impl fmt::Debug for Way {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Way")
            .field("dial", &self.dial.label())
            .field("addressing", &self.addressing)
            .finish()
    }
}

/// What a network gives: a set of bits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Caps(u8);

impl Caps {
    /// No capability.
    pub const NONE: Caps = Caps(0);
    /// TCP flows out ([`Dial::dial`]).
    pub const EGRESS_TCP: Caps = Caps(1);
    /// UDP flows out ([`Dial::datagrams`]).
    pub const EGRESS_UDP: Caps = Caps(1 << 1);
    /// Flows in to local services.
    pub const INGRESS: Caps = Caps(1 << 2);
    /// A configuration for another program ([`Export`]).
    pub const EXPORT: Caps = Caps(1 << 3);
    /// A kernel interface (a TUN device). It needs `CAP_NET_ADMIN`.
    pub const KERNEL_IFACE: Caps = Caps(1 << 4);

    const NAMES: [(Caps, &'static str); 5] = [
        (Caps::EGRESS_TCP, "egress-tcp"),
        (Caps::EGRESS_UDP, "egress-udp"),
        (Caps::INGRESS, "ingress"),
        (Caps::EXPORT, "export"),
        (Caps::KERNEL_IFACE, "kernel-iface"),
    ];

    /// The bits of `self` and of `other` (the `|` of a constant).
    pub const fn with(self, other: Caps) -> Caps {
        Caps(self.0 | other.0)
    }

    /// True when each bit of `other` is in `self`.
    pub fn contains(self, other: Caps) -> bool {
        self.0 & other.0 == other.0
    }

    /// The name of each bit, in a fixed order (for a status document).
    pub fn names(self) -> Vec<&'static str> {
        Caps::NAMES
            .iter()
            .filter(|(cap, _)| self.contains(*cap))
            .map(|(_, name)| *name)
            .collect()
    }
}

impl BitOr for Caps {
    type Output = Caps;

    fn bitor(self, other: Caps) -> Caps {
        Caps(self.0 | other.0)
    }
}

/// What a network needs to come up.
#[derive(Clone, Default)]
pub struct Ctx {
    /// The way under this network: the network sends its own connections
    /// through it (the relay through a proxy, WARP through the relay).
    /// With `None`, the network uses the egress of the process.
    pub below: Option<Arc<dyn Dial>>,
}

impl fmt::Debug for Ctx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Ctx")
            .field("below", &self.below.as_ref().map(|d| d.label()))
            .finish()
    }
}

/// A configuration that a network gives to another program.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Export {
    /// The form: `wg-quick`, `env`, `ssh-config` or `mwan-json`.
    pub kind: &'static str,
    /// The name of the file, or of the export in a list.
    pub name: String,
    /// The text of the export.
    pub text: String,
}

/// One row of `podnet doctor` that a network asks for: a target that the
/// network must reach.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check {
    /// The name of the row.
    pub name: String,
    /// The target to reach.
    pub target: Target,
    /// True: after the TCP connect, the row checks TLS with the name of
    /// the target.
    pub tls: bool,
}

/// The function that ends a link.
pub type Close = Box<dyn FnOnce() -> BoxFuture<'static, ()> + Send>;

/// A network that is up: its way out, its exports, its part of the ready
/// document, and the function that ends it.
pub struct Link {
    /// The way out, for a network with egress.
    pub way: Option<Way>,
    /// The exports, for a network with [`Caps::EXPORT`].
    pub exports: Vec<Export>,
    /// The part of the network in the ready document (`--json`).
    pub status: serde_json::Value,
    close: Option<Close>,
}

impl Link {
    /// A link with `way`, no export, a null status and no close function.
    pub fn new(way: Option<Way>) -> Link {
        Link {
            way,
            exports: Vec::new(),
            status: serde_json::Value::Null,
            close: None,
        }
    }

    /// Add an export.
    pub fn with_export(mut self, export: Export) -> Link {
        self.exports.push(export);
        self
    }

    /// Set the status document.
    pub fn with_status(mut self, status: serde_json::Value) -> Link {
        self.status = status;
        self
    }

    /// Set the function that [`Link::close`] runs.
    pub fn on_close(mut self, close: Close) -> Link {
        self.close = Some(close);
        self
    }

    /// End the link: run its close function one time.
    pub async fn close(mut self) {
        if let Some(close) = self.close.take() {
            close().await;
        }
    }
}

impl fmt::Debug for Link {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Link")
            .field("way", &self.way)
            .field("exports", &self.exports)
            .field("status", &self.status)
            .field("close", &self.close.is_some())
            .finish()
    }
}

/// An adapter: a network that a spec names (`direct`, `socks5h://...`,
/// `relay:HOST`, `warp`).
pub trait Network: Send + Sync {
    /// The scheme of the spec: `direct`, `proxy`, `relay`, `warp`, `wg`,
    /// `vnet` or `tailscale`.
    fn scheme(&self) -> &'static str;

    /// What the network gives.
    fn caps(&self) -> Caps;

    /// Make the network ready (register, do the handshake, bind), and give
    /// its link.
    fn up<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, anyhow::Result<Link>>;

    /// The rows of `podnet doctor` for this network. The default: no row.
    fn checks(&self) -> Vec<Check> {
        Vec::new()
    }
}

#[cfg(test)]
mod fakes;
#[cfg(test)]
mod tests;
