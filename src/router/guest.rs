//! The guest wires of the router (T-248). A guest (a VM, a container, a host
//! behind one wire) uses the router as its gateway. The wire carries IPv4
//! packets. A stack of `pod-netstack` ends the TCP and the UDP of the guest,
//! and the router sends each flow to a network or a group, with the rules and
//! the groups of a door.
//!
//! ```yaml
//! guests:
//!   - wire: unix:/run/g.sock    # unix:/PATH (SOCK_SEQPACKET) or tun:NAME
//!     net: 10.66.0.0/24         # the guest net; the router is its first host
//!     via: egress               # a network or a group of the file
//!     rules: /etc/r.rules       # optional: rules that go before `via`
//!     allow_shadowed_rules: false
//!     mtu: 1500                 # optional (576 to 9000)
//! ```
//!
//! * TCP: the pump of the wire holds each new SYN, dials the target, and gives
//!   the SYN to the stack when the dial succeeds. A failed dial answers the
//!   SYN with a RST ([`tcp`]).
//! * UDP: a datagram goes over `Dial::datagrams` of the way that the rules
//!   choose. Each flow has a bounded queue and counters ([`udp`]).

use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use podnet_vnet::routes::RulesFile;
use serde::Deserialize;

use crate::network::{Choice, Class, Dial, Failure, Stage, Target, Way};

pub mod pump;
pub mod tcp;
pub mod udp;
pub mod wire;

#[cfg(test)]
mod kit;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod udp_tests;

pub use wire::{Opened, WireSpec};

/// The flows of a guest at one time: the held SYNs and the open TCP flows.
pub const MAX_FLOWS: usize = 2048;
/// The time that a dial for a held SYN may take. A Linux guest gives up
/// its connect after about 127 s.
pub const DIAL_TIMEOUT: Duration = Duration::from_secs(30);
/// The MTU of a guest wire when the file gives none.
pub const DEFAULT_MTU: usize = 1500;

/// One `guests` entry of the file.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuestConfig {
    /// `unix:/PATH` or `tun:NAME`.
    pub wire: String,
    /// The guest net, `A.B.C.D/N` (8 to 30). The router has the first host
    /// address; a source address outside the net is dropped.
    pub net: String,
    /// A network or a group of the file.
    pub via: String,
    /// A rules file: its rules go first; `via` is the default.
    #[serde(default)]
    pub rules: Option<PathBuf>,
    /// Load a rules file in which an earlier rule shadows a later `reject`.
    #[serde(default)]
    pub allow_shadowed_rules: bool,
    /// The MTU of the wire (576 to 9000; default 1500).
    #[serde(default)]
    pub mtu: Option<usize>,
}

/// The net of a guest: its network address and its prefix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GuestNet {
    pub network: Ipv4Addr,
    pub prefix: u8,
}

impl GuestNet {
    fn mask(&self) -> u32 {
        u32::MAX
            .checked_shl(32 - u32::from(self.prefix))
            .unwrap_or(0)
    }

    /// The address of the router: the first host of the net.
    pub fn gateway(&self) -> Ipv4Addr {
        Ipv4Addr::from(u32::from(self.network) | 1)
    }

    pub fn contains(&self, ip: Ipv4Addr) -> bool {
        u32::from(ip) & self.mask() == u32::from(self.network)
    }
}

impl FromStr for GuestNet {
    type Err = anyhow::Error;

    fn from_str(text: &str) -> Result<Self> {
        let bad = || anyhow::anyhow!("{text:?}: give A.B.C.D/N with N from 8 to 30");
        let (ip, prefix) = text.split_once('/').ok_or_else(bad)?;
        let ip: Ipv4Addr = ip.parse().map_err(|_| bad())?;
        let prefix: u8 = prefix.parse().map_err(|_| bad())?;
        if !(8..=30).contains(&prefix) {
            return Err(bad());
        }
        let mut net = GuestNet {
            network: ip,
            prefix,
        };
        net.network = Ipv4Addr::from(u32::from(ip) & net.mask());
        Ok(net)
    }
}

/// The counters of a guest. A counter only grows, except `tcp_open`.
#[derive(Debug, Default)]
pub struct GuestStats {
    /// Dials started for a held SYN.
    pub tcp_dialed: AtomicU64,
    /// SYNs answered with a RST: the dial failed or a rule refused it.
    pub tcp_refused: AtomicU64,
    /// SYNs dropped at `MAX_FLOWS` (the guest sends them again).
    pub tcp_syn_dropped: AtomicU64,
    /// TCP flows open now.
    pub tcp_open: AtomicU64,
    /// UDP flows made.
    pub udp_flows: AtomicU64,
    /// Datagrams sent out, and datagrams sent to the guest.
    pub udp_up: AtomicU64,
    pub udp_down: AtomicU64,
    /// Datagrams lost at a full queue (a flow is slow), up and down.
    pub udp_queue_dropped: AtomicU64,
    /// Datagrams over the size that the wire carries (down).
    pub udp_oversize: AtomicU64,
    /// Datagrams refused: a rule, a way with no UDP, or the limit of flows.
    pub udp_refused: AtomicU64,
    /// Packets dropped by the screen of the wire: a source outside the net,
    /// a destination that the router does not route, a protocol other than
    /// TCP and UDP, or IPv6.
    pub dropped: AtomicU64,
    /// The counters of the stack (`NetStats`), copied at each pass of the
    /// housekeeping: the datagrams it lost at the full backlog of a port,
    /// the datagrams to the guest it lost, and the datagrams over its size.
    pub stack_udp_rx_dropped: AtomicU64,
    pub stack_udp_tx_dropped: AtomicU64,
    pub stack_udp_oversize: AtomicU64,
}

/// The counters that the status document and the metrics give: the key of
/// the status, the name of the metric, the type and the help.
pub const COUNTERS: [(&str, &str, &str, &str); 14] = [
    (
        "tcp_dialed",
        "podnet_route_guest_tcp_dialed_total",
        "counter",
        "Dials for the SYNs of the guest.",
    ),
    (
        "tcp_refused",
        "podnet_route_guest_tcp_refused_total",
        "counter",
        "SYNs of the guest answered with a RST.",
    ),
    (
        "tcp_syn_dropped",
        "podnet_route_guest_tcp_syn_dropped_total",
        "counter",
        "SYNs of the guest dropped at the limit of flows.",
    ),
    (
        "tcp_open",
        "podnet_route_guest_tcp_open",
        "gauge",
        "TCP flows of the guest that are open.",
    ),
    (
        "udp_flows",
        "podnet_route_guest_udp_flows_total",
        "counter",
        "UDP flows of the guest.",
    ),
    (
        "udp_up",
        "podnet_route_guest_udp_up_total",
        "counter",
        "Datagrams sent out for the guest.",
    ),
    (
        "udp_down",
        "podnet_route_guest_udp_down_total",
        "counter",
        "Datagrams sent to the guest.",
    ),
    (
        "udp_queue_dropped",
        "podnet_route_guest_udp_queue_dropped_total",
        "counter",
        "Datagrams lost at the full queue of a flow.",
    ),
    (
        "udp_oversize",
        "podnet_route_guest_udp_oversize_total",
        "counter",
        "Datagrams to the guest over the size of the wire.",
    ),
    (
        "udp_refused",
        "podnet_route_guest_udp_refused_total",
        "counter",
        "Datagrams refused by a rule, a way with no UDP or the limit of flows.",
    ),
    (
        "dropped",
        "podnet_route_guest_dropped_total",
        "counter",
        "Packets of the guest dropped by the screen of the wire.",
    ),
    (
        "stack_udp_rx_dropped",
        "podnet_route_guest_stack_udp_rx_dropped_total",
        "counter",
        "Datagrams of the guest that the stack lost at the full backlog of a port.",
    ),
    (
        "stack_udp_tx_dropped",
        "podnet_route_guest_stack_udp_tx_dropped_total",
        "counter",
        "Datagrams to the guest that the stack lost.",
    ),
    (
        "stack_udp_oversize",
        "podnet_route_guest_stack_udp_oversize_total",
        "counter",
        "Datagrams over what the stack carries (a refused send, or fragments it cannot join).",
    ),
];

impl GuestStats {
    fn counter(&self, key: &str) -> &AtomicU64 {
        match key {
            "tcp_dialed" => &self.tcp_dialed,
            "tcp_refused" => &self.tcp_refused,
            "tcp_syn_dropped" => &self.tcp_syn_dropped,
            "tcp_open" => &self.tcp_open,
            "udp_flows" => &self.udp_flows,
            "udp_up" => &self.udp_up,
            "udp_down" => &self.udp_down,
            "udp_queue_dropped" => &self.udp_queue_dropped,
            "udp_oversize" => &self.udp_oversize,
            "udp_refused" => &self.udp_refused,
            "stack_udp_rx_dropped" => &self.stack_udp_rx_dropped,
            "stack_udp_tx_dropped" => &self.stack_udp_tx_dropped,
            "stack_udp_oversize" => &self.stack_udp_oversize,
            _ => &self.dropped,
        }
    }

    /// The counters as a JSON object.
    pub fn json(&self) -> serde_json::Value {
        let mut map = serde_json::Map::new();
        for (key, ..) in COUNTERS {
            map.insert(key.into(), self.counter(key).load(Ordering::Relaxed).into());
        }
        map.into()
    }

    pub(crate) fn add(&self, counter: &AtomicU64) {
        counter.fetch_add(1, Ordering::Relaxed);
    }
}

/// One guest wire of a router.
pub struct RouterGuest {
    pub wire: WireSpec,
    pub net: GuestNet,
    pub mtu: usize,
    /// The network or the group of `via`.
    pub via: String,
    /// The rules file, if any.
    pub rules: Option<Arc<RulesFile>>,
    /// The dial of TCP: the rules, or the way of `via`.
    pub dial: Arc<dyn Dial>,
    /// The way of a target that no rule matches.
    pub default: Way,
    pub stats: Arc<GuestStats>,
}

impl RouterGuest {
    /// The guest of `config`, over the `ways` of the router.
    pub(super) fn build(
        i: usize,
        config: &GuestConfig,
        ways: &std::collections::HashMap<String, Way>,
    ) -> Result<RouterGuest> {
        let wire: WireSpec = config
            .wire
            .parse()
            .map_err(|e| anyhow::anyhow!("guests[{i}].wire: {e}"))?;
        let net: GuestNet = config
            .net
            .parse()
            .map_err(|e| anyhow::anyhow!("guests[{i}].net: {e}"))?;
        let mtu = config.mtu.unwrap_or(DEFAULT_MTU);
        if !(576..=9000).contains(&mtu) {
            bail!("guests[{i}].mtu: give 576 to 9000");
        }
        let default = ways
            .get(&config.via)
            .with_context(|| {
                format!(
                    "guests[{i}].via: {:?} is not a network or a group of the file",
                    config.via
                )
            })?
            .clone();
        let rules = match &config.rules {
            Some(path) => Some(Arc::new(
                RulesFile::open_with(
                    path,
                    ways.clone(),
                    Some(default.clone()),
                    config.allow_shadowed_rules,
                )
                .with_context(|| format!("guests[{i}].rules"))?,
            )),
            None => None,
        };
        let dial: Arc<dyn Dial> = match &rules {
            Some(rules) => rules.clone(),
            None => default.dial.clone(),
        };
        Ok(RouterGuest {
            wire,
            net,
            mtu,
            via: config.via.clone(),
            rules,
            dial,
            default,
            stats: Arc::new(GuestStats::default()),
        })
    }

    /// The way for the datagrams of `target`: the rule that matches, else
    /// `via`. A rule that rejects the target is an error.
    pub fn way_for(&self, target: &Target) -> Result<Way, Failure> {
        let Some(rules) = &self.rules else {
            return Ok(self.default.clone());
        };
        let rules = rules.rules();
        match rules.choose_datagram(target) {
            Choice::Way { way, .. } => Ok(way.clone()),
            Choice::Refuse { rule } => Err(Failure::new(
                Stage::Connect,
                Class::Policy,
                format!(
                    "{target} is refused by {}",
                    rule.as_deref().unwrap_or("the rules")
                ),
            )),
        }
    }

    /// The label of the way, for a log line.
    pub fn label(&self) -> String {
        self.dial.label()
    }

    /// The status document of the guest.
    pub fn status(&self) -> serde_json::Value {
        serde_json::json!({
            "wire": self.wire.to_string(),
            "net": format!("{}/{}", self.net.network, self.net.prefix),
            "via": self.label(),
            "counters": self.stats.json(),
        })
    }

    /// Open the wire.
    pub fn open(&self, replace: bool) -> Result<Opened> {
        self.wire.open(replace)
    }

    /// Serve the guest on the open wire until the wire fails.
    pub async fn serve(self: Arc<Self>, opened: Opened) -> Result<()> {
        pump::Pump::new(self)?.run(opened).await
    }
}
