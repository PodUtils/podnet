//! The router of `podnet route` (T-100): a config file of networks, groups,
//! doors and a health probe, read with the YAML reader of `podnet-core`.
//!
//! ```yaml
//! networks:            # NAME: SPEC of the registry (crate::networks)
//!   home: direct
//!   proxy: socks5h://127.0.0.1:1080
//! groups:              # NAME: MEMBER*WEIGHT, ... (T-096)
//!   egress: home*2, proxy*1
//! policies:           # optional: NAME: POLICY of a group (T-178)
//!   egress: minrtt     # weighted (default), rr, minrtt, leastload, protect
//! splits:             # optional: NAME: up=MEMBER down=MEMBER of a group (T-179)
//!   egress: up=home down=proxy  # UDP only; off (default) keeps one member
//! health:              # the probe of the members of each group (T-097)
//!   target: 1.1.1.1:443
//!   interval: 10
//!   down_after: 4      # K misses take a member down (K > R > 1, R > K/2)
//!   up_after: 3        # R passes take it up
//!   cooldown: 30       # seconds in cooldown before up
//!   timeout: 5         # seconds of one probe
//!   window: 8          # passing probe times for the floor (0 to 64; minrtt)
//!   egress:            # optional: NETWORK: ECHO_HOST:PORT=IP (WrongEgress, T-097)
//!     proxy: 203.0.113.9:7=198.51.100.4
//! resolvers:           # optional: NETWORK: HOST:PORT, a DNS server at the far end (T-098)
//!   proxy: 10.0.0.53:53
//! kernel:              # optional: tables, rules and a multipath route (T-119)
//!   wans:              # see `kernel` for the keys; needs CAP_NET_ADMIN
//!     - {name: a, dev: tun0}
//!     - {name: b, dev: tun1, weight: 2}
//! control: unix:/run/r.ctl  # optional: `podnet route --status` reads it
//! doors:               # SOCKS5 (CONNECT and UDP ASSOCIATE) and HTTP CONNECT doors (T-061)
//!   - listen: unix:/run/r.sock
//!     via: egress      # a network or a group
//!     rules: /etc/r.rules   # optional: the rules of T-098
//!     allow_shadowed_rules: false  # true loads a file with a shadowed reject
//!     dns: 127.0.0.1:5353   # optional: a DNS door (UDP and TCP) over the same way
//!     dns_upstream: 10.0.0.53:53  # the resolver at the far end (needed with dns)
//! ```

use std::collections::{BTreeMap, HashMap};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use podnet_netns::listen::ProxyListen;
use podnet_vnet::door::Door;
use podnet_vnet::routes::RulesFile;
use serde::Deserialize;
use tokio::time::Instant;

pub mod control;
pub mod kernel;
pub mod metrics;
pub mod resolving;

pub use control::{read_metrics, read_status};

use crate::network::{
    parse_group, probe, probe_egress, Balance, Choice, Ctx, Dial, Member, Network as _, Policy,
    Selection, Target, Thresholds, Way,
};

/// The config file.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub networks: BTreeMap<String, String>,
    #[serde(default)]
    pub groups: BTreeMap<String, String>,
    /// The policy of a group (T-178); a group with no entry is `weighted`.
    #[serde(default)]
    pub policies: BTreeMap<String, String>,
    /// The split of the directions of the datagrams of a group (T-179):
    /// `up=MEMBER down=MEMBER`, or `off`. A group with no entry has no split.
    #[serde(default)]
    pub splits: BTreeMap<String, String>,
    /// The resolver of a network (T-098): `NETWORK: HOST:PORT`, a DNS server
    /// (DNS over TCP) at the far end of the network. A name that a flow
    /// brings to a member of this network resolves there, never with the
    /// resolver of this host. A network with no entry resolves as its dial
    /// does (a proxy that takes names does it at the far end).
    #[serde(default)]
    pub resolvers: BTreeMap<String, String>,
    #[serde(default)]
    pub health: Option<Health>,
    /// `unix:/PATH`: the control socket (mode 0600) of `route --status`.
    #[serde(default)]
    pub control: Option<String>,
    /// A directory for the identity of each network, `DIR/NETWORK` (T-180);
    /// a reconnect and a rotate give it to the network again.
    #[serde(default)]
    pub state_dir: Option<PathBuf>,
    /// The kernel mode (T-119): tables, rules and a multipath route.
    #[serde(default)]
    pub kernel: Option<kernel::KernelConfig>,
    /// The doors; none are allowed when `kernel` is set.
    #[serde(default)]
    pub doors: Vec<DoorConfig>,
}

/// The health probe of the members of the groups.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Health {
    /// HOST:PORT that each member must reach.
    pub target: String,
    /// Seconds between two probes (default 10).
    #[serde(default)]
    pub interval: Option<u64>,
    /// K: the misses in a row that take a member down (default 4).
    #[serde(default)]
    pub down_after: Option<u32>,
    /// R: the passes in a row that take a member up (default 3).
    #[serde(default)]
    pub up_after: Option<u32>,
    /// The least seconds in cooldown before up (default 30).
    #[serde(default)]
    pub cooldown: Option<u64>,
    /// The seconds of one probe (default 5).
    #[serde(default)]
    pub timeout: Option<u64>,
    /// How many passing probe times the floor of `minrtt` reads (default 8).
    #[serde(default)]
    pub window: Option<usize>,
    /// The egress check of a network (T-097): `NETWORK: HOST:PORT=IP`. The
    /// probe also connects through the member to the echo endpoint
    /// `HOST:PORT` of the operator, which writes the address it sees for
    /// the peer. Another address than `IP` is `WrongEgress`. A network with
    /// no entry has no egress check.
    #[serde(default)]
    pub egress: BTreeMap<String, String>,
}

impl Health {
    /// The thresholds of the file over the defaults. They must give K > R >
    /// 1 and R > K/2, and an interval and a timeout of 1 s or more.
    pub fn thresholds(&self) -> Result<Thresholds> {
        let d = Thresholds::default();
        let secs = |key: &str, value: Option<u64>, default: Duration| -> Result<Duration> {
            match value {
                Some(0) => bail!("health.{key}: give 1 or more seconds"),
                Some(n) => Ok(Duration::from_secs(n)),
                None => Ok(default),
            }
        };
        let t = Thresholds {
            down_after: self.down_after.unwrap_or(d.down_after),
            up_after: self.up_after.unwrap_or(d.up_after),
            cooldown: Duration::from_secs(self.cooldown.unwrap_or(d.cooldown.as_secs())),
            interval: secs("interval", self.interval, d.interval)?,
            timeout: secs("timeout", self.timeout, d.timeout)?,
            rtt_window: self.window.unwrap_or(d.rtt_window),
        };
        t.check()
            .map_err(|e| anyhow::anyhow!("health: {}", e.text()))?;
        Ok(t)
    }
}

impl Health {
    /// The egress checks of the file, by network name: the echo endpoint
    /// and the address that it must name.
    pub fn egress_checks(&self, config: &Config) -> Result<HashMap<String, (Target, IpAddr)>> {
        let mut checks = HashMap::new();
        for (name, value) in &self.egress {
            if !config.networks.contains_key(name) {
                bail!("health.egress.{name}: not a network of the file");
            }
            let bad = || anyhow::anyhow!("health.egress.{name}: give HOST:PORT=IP");
            let (echo, expect) = value.rsplit_once('=').ok_or_else(bad)?;
            let echo: Target = echo.trim().parse().map_err(|_| bad())?;
            let expect: IpAddr = expect.trim().parse().map_err(|_| bad())?;
            if echo.port() == 0 {
                bail!("health.egress.{name}: the port must be 1 to 65535");
            }
            checks.insert(name.clone(), (echo, expect));
        }
        Ok(checks)
    }
}

/// The split of group `name` with the members `list` (the value of
/// `groups.NAME`): the names of `up` and `down`, or `None` for `off`.
fn split_of(name: &str, value: &str, list: &str) -> Result<Option<(String, String)>> {
    let (_, members) = parse_group(&format!("{name}={list}"))
        .map_err(|e| anyhow::anyhow!("groups.{name}: {}", e.text()))?;
    let names: Vec<String> = members.into_iter().map(|(m, _)| m).collect();
    podnet_core::network::split::parse(value, &names)
        .map_err(|e| anyhow::anyhow!("splits.{name}: {e}"))
}

/// One door.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DoorConfig {
    /// `HOST:PORT`, `tcp://HOST:PORT` or `unix:/PATH`.
    pub listen: String,
    /// A network or a group of the file.
    pub via: String,
    /// A rules file (T-098): its rules go first; `via` is the default.
    #[serde(default)]
    pub rules: Option<PathBuf>,
    /// Load a rules file in which an earlier rule shadows a later `reject`
    /// (T-242; as `pod --allow-shadowed-rules`). The lint still warns.
    #[serde(default)]
    pub allow_shadowed_rules: bool,
    /// `ADDR:PORT` of a DNS door (UDP and TCP) over the dial of the door.
    #[serde(default)]
    pub dns: Option<String>,
    /// `HOST:PORT` of the resolver at the far end of the way (needed with
    /// `dns`: the router names no resolver of its own).
    #[serde(default)]
    pub dns_upstream: Option<String>,
}

impl DoorConfig {
    /// The DNS door: its address and its upstream, or `None`.
    pub fn dns(&self, i: usize) -> Result<Option<(std::net::SocketAddr, Target)>> {
        match (&self.dns, &self.dns_upstream) {
            (None, None) => Ok(None),
            (None, Some(_)) => bail!("doors[{i}].dns_upstream: give doors[{i}].dns too"),
            (Some(_), None) => bail!("doors[{i}].dns: give doors[{i}].dns_upstream (HOST:PORT)"),
            (Some(dns), Some(upstream)) => {
                let addr = dns
                    .parse()
                    .map_err(|_| anyhow::anyhow!("doors[{i}].dns: give ADDR:PORT"))?;
                let upstream: Target = upstream
                    .parse()
                    .map_err(|_| anyhow::anyhow!("doors[{i}].dns_upstream: give HOST:PORT"))?;
                if upstream.port() == 0 {
                    bail!("doors[{i}].dns_upstream: the port must be 1 to 65535");
                }
                Ok(Some((addr, upstream)))
            }
        }
    }
}

/// Read the config file `path`. An error names the key or the line.
pub fn load(path: &Path) -> Result<Config> {
    let text = std::fs::read_to_string(path).with_context(|| path.display().to_string())?;
    parse(&text).with_context(|| path.display().to_string())
}

/// Read a config of `text`.
pub fn parse(text: &str) -> Result<Config> {
    let value = podnet_core::yaml::parse(text)?;
    let config: Config = serde_json::from_value(value).map_err(|e| anyhow::anyhow!("{e}"))?;
    match &config.kernel {
        Some(kernel) => {
            kernel::Plan::new(kernel)?;
        }
        None if config.doors.is_empty() => bail!("doors: give one door or more"),
        None => {}
    }
    for (name, _) in config.networks.iter().chain(config.groups.iter()) {
        if name.is_empty()
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            bail!("the name {name:?}: letters, digits, - and _");
        }
    }
    if let Some(name) = config
        .groups
        .keys()
        .find(|g| config.networks.contains_key(*g))
    {
        bail!("groups.{name}: a network has the same name");
    }
    for (name, policy) in &config.policies {
        if !config.groups.contains_key(name) {
            bail!("policies.{name}: not a group of the file");
        }
        if Policy::parse(policy).is_none() {
            bail!(
                "policies.{name}: {policy:?} is not a policy (weighted, rr, minrtt, leastload, protect)"
            );
        }
    }
    for (name, value) in &config.splits {
        let Some(list) = config.groups.get(name) else {
            bail!("splits.{name}: not a group of the file");
        };
        split_of(name, value, list)?;
    }
    for name in config.resolvers.keys() {
        resolver_of(&config, name)?;
    }
    for (i, door) in config.doors.iter().enumerate() {
        if !config.networks.contains_key(&door.via) && !config.groups.contains_key(&door.via) {
            bail!(
                "doors[{i}].via: {:?} is not a network or a group of the file",
                door.via
            );
        }
        ProxyListen::parse(&door.listen).map_err(|e| anyhow::anyhow!("doors[{i}].listen: {e}"))?;
        door.dns(i)?;
    }
    if let Some(health) = &config.health {
        health
            .target
            .parse::<Target>()
            .map_err(|_| anyhow::anyhow!("health.target: give HOST:PORT"))?;
        health.thresholds()?;
        health.egress_checks(&config)?;
    }
    if let Some(control) = &config.control {
        match ProxyListen::parse(control) {
            Ok(ProxyListen::Unix(_)) => {}
            _ => bail!("control: give unix:/PATH"),
        }
    }
    Ok(config)
}

/// The DNS server of the network `name` (`resolvers.NAME`), or `None`.
fn resolver_of(config: &Config, name: &str) -> Result<Option<Target>> {
    let Some(value) = config.resolvers.get(name) else {
        return Ok(None);
    };
    if !config.networks.contains_key(name) {
        bail!("resolvers.{name}: not a network of the file");
    }
    let upstream: Target = value
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("resolvers.{name}: give HOST:PORT"))?;
    if upstream.port() == 0 {
        bail!("resolvers.{name}: the port must be 1 to 65535");
    }
    Ok(Some(upstream))
}

/// One door of a router.
pub struct RouterDoor {
    pub listen: ProxyListen,
    pub door: Arc<Door>,
    /// The label of the dial of the door.
    pub label: String,
    /// The network or the group of `via`.
    pub via: String,
    /// The rules file of the door, if any.
    pub rules: Option<Arc<RulesFile>>,
    /// The DNS door over the same dial: its address and its upstream.
    pub dns: Option<(std::net::SocketAddr, Target)>,
    /// The dial of the door (its rules, or the way of `via`).
    pub dial: Arc<dyn Dial>,
}

/// A router that is up: its ways, its groups and its doors.
pub struct Router {
    pub ways: HashMap<String, Way>,
    pub groups: Vec<(String, Arc<Balance>)>,
    pub doors: Vec<RouterDoor>,
    pub health: Option<(Target, Thresholds)>,
    /// The egress checks of the networks (`health.egress`, T-097).
    pub egress: HashMap<String, (Target, IpAddr)>,
    /// The write side of the control socket (T-180).
    pub control: control::ControlState,
}

/// The choice of a door for one target (T-177): the rule, the way and the
/// member of a group. The router dials with the same choice.
#[derive(Debug, PartialEq, Eq)]
pub struct Decision {
    /// The label of the rule that matched; `None` for the default (`via`).
    pub rule: Option<String>,
    /// The network or the group; `None` when the rules refuse the target.
    pub way: Option<String>,
    /// The member that the pick of the group gives, and its health; `None`
    /// for a network, or when no member of the group takes flows.
    pub member: Option<(String, String)>,
}

impl Decision {
    /// True when a flow to the target can go: a network, or a member that
    /// takes flows.
    pub fn passes(&self, groups: &[(String, Arc<Balance>)]) -> bool {
        match &self.way {
            None => false,
            Some(way) => !groups.iter().any(|(g, _)| g == way) || self.member.is_some(),
        }
    }
}

/// A way that names a network and does not dial: the ways of a dry run.
struct Named(String);

impl Dial for Named {
    fn label(&self) -> String {
        self.0.clone()
    }

    fn dial<'a>(
        &'a self,
        target: &'a Target,
        _deadline: Instant,
    ) -> podnet_core::network::BoxFuture<
        'a,
        Result<podnet_core::network::Connected, podnet_core::network::Failure>,
    > {
        Box::pin(async move {
            Err(podnet_core::network::Failure::new(
                podnet_core::network::Stage::Connect,
                podnet_core::network::Class::Usage,
                format!("{}: a dry run does not dial {target}", self.0),
            ))
        })
    }
}

impl Router {
    /// Bring up each network, build the groups and the doors.
    pub async fn up(config: &Config) -> Result<Router> {
        let mut ways = HashMap::new();
        let mut links = HashMap::new();
        for (name, spec) in &config.networks {
            let network = crate::networks::parse(spec)
                .map_err(|e| anyhow::anyhow!("networks.{name}: {}", e.text()))?;
            let link = network
                .up(&Ctx::default())
                .await
                .with_context(|| format!("networks.{name}"))?;
            let way = link
                .way
                .clone()
                .with_context(|| format!("networks.{name}: the network gives no way out"))?;
            ways.insert(name.clone(), way);
            links.insert(name.clone(), link);
        }
        let router = Router::build(config, ways)?;
        router.control.keep(links).await;
        Ok(router)
    }

    /// The router of `config` with no network up and no dial: each network
    /// is a way that only names it (`route --check --to`, T-177). The specs
    /// are parsed.
    pub fn plan(config: &Config) -> Result<Router> {
        let mut ways = HashMap::new();
        for (name, spec) in &config.networks {
            crate::networks::parse(spec)
                .map_err(|e| anyhow::anyhow!("networks.{name}: {}", e.text()))?;
            ways.insert(
                name.clone(),
                Way::destination(Arc::new(Named(name.clone()))),
            );
        }
        Router::build(config, ways)
    }

    fn build(config: &Config, mut ways: HashMap<String, Way>) -> Result<Router> {
        // Each network sits behind a `Swap`: a reconnect or a rotate (T-180)
        // reaches each group, each rule and each door.
        let mut swaps = HashMap::new();
        for (name, way) in ways.iter_mut() {
            let swap = Arc::new(crate::network::Swap::new(way.dial.clone()));
            way.dial = swap.clone();
            // A name resolves through the network of its member (T-098).
            if let Some(upstream) = resolver_of(config, name)? {
                way.dial = Arc::new(resolving::Resolving::new(swap.clone(), upstream));
            }
            swaps.insert(name.clone(), swap);
        }
        let mut groups = Vec::new();
        for (name, list) in &config.groups {
            let (_, members) = parse_group(&format!("{name}={list}"))
                .map_err(|e| anyhow::anyhow!("groups.{name}: {}", e.text()))?;
            let mut built = Vec::new();
            for (member, weight) in members {
                let way = ways.get(&member).with_context(|| {
                    format!("groups.{name}: {member:?} is not a network of the file")
                })?;
                built.push(Member::new(&member, way.clone(), weight));
            }
            let policy = config
                .policies
                .get(name)
                .and_then(|p| Policy::parse(p))
                .unwrap_or_default();
            let mut group = Balance::new(name, built)
                .map_err(|e| anyhow::anyhow!("groups.{name}: {}", e.text()))?
                .with_policy(policy);
            if let Some((up, down)) = config
                .splits
                .get(name)
                .map(|v| split_of(name, v, list))
                .transpose()?
                .flatten()
            {
                group = group
                    .with_split(&up, &down)
                    .map_err(|e| anyhow::anyhow!("splits.{name}: {}", e.text()))?;
            }
            let group = Arc::new(group);
            ways.insert(name.clone(), Way::destination(group.clone()));
            groups.push((name.clone(), group));
        }
        let mut doors = Vec::new();
        for (i, door) in config.doors.iter().enumerate() {
            let way = ways[&door.via].clone();
            let rules = match &door.rules {
                Some(path) => Some(Arc::new(
                    RulesFile::open_with(
                        path,
                        ways.clone(),
                        Some(way.clone()),
                        door.allow_shadowed_rules,
                    )
                    .with_context(|| format!("doors[{i}].rules"))?,
                )),
                None => None,
            };
            let dial: Arc<dyn Dial> = match &rules {
                Some(rules) => rules.clone(),
                None => way.dial,
            };
            let listen = ProxyListen::parse(&door.listen)
                .map_err(|e| anyhow::anyhow!("doors[{i}].listen: {e}"))?;
            // A UDP ASSOCIATE answers with the address of the door (T-098).
            let mut udp_door = Door::new(dial.clone());
            if let ProxyListen::Tcp(addr) = &listen {
                udp_door = udp_door.with_udp_bind(addr.ip());
            }
            doors.push(RouterDoor {
                listen,
                door: Arc::new(udp_door),
                label: dial.label(),
                via: door.via.clone(),
                rules,
                dns: door.dns(i)?,
                dial,
            });
        }
        let health = match &config.health {
            Some(h) => Some((
                h.target
                    .parse()
                    .map_err(|_| anyhow::anyhow!("health.target"))?,
                h.thresholds()?,
            )),
            None => None,
        };
        let egress = match &config.health {
            Some(h) => h.egress_checks(config)?,
            None => HashMap::new(),
        };
        let control = control::ControlState::new(
            swaps,
            &config.networks,
            config.state_dir.clone(),
            health.as_ref().map(|(_, t)| t.interval),
        );
        Ok(Router {
            ways,
            groups,
            doors,
            health,
            egress,
            control,
        })
    }

    /// The choice of door `index` for `target`, with the rules of now and the
    /// pick of the group (T-177). A pick counts as a flow of the member.
    pub fn decide(&self, index: usize, target: &Target) -> Decision {
        let door = &self.doors[index];
        let (rule, way) = match &door.rules {
            Some(rules) => match rules.rules().choose(target) {
                Choice::Way { way, rule } => (rule, Some(self.name_of(way))),
                Choice::Refuse { rule } => (rule, None),
            },
            None => (None, Some(door.via.clone())),
        };
        let member = way.as_ref().and_then(|way| {
            let (_, group) = self.groups.iter().find(|(g, _)| g == way)?;
            match group.pick() {
                Selection::One(m) => Some((m.name.clone(), m.health().name().to_string())),
                Selection::None => None,
            }
        });
        Decision { rule, way, member }
    }

    /// The name of `way` in the file: the way with the same dial.
    fn name_of(&self, way: &Way) -> String {
        let at = Arc::as_ptr(&way.dial) as *const ();
        self.ways
            .iter()
            .find(|(_, w)| Arc::as_ptr(&w.dial) as *const () == at)
            .map_or_else(|| way.label(), |(name, _)| name.clone())
    }

    /// The ready document: each door and the status of each group.
    pub fn status(&self) -> serde_json::Value {
        serde_json::json!({
            "doors": self.doors.iter().map(|d| serde_json::json!({"listen": d.listen.to_string(), "via": d.label})).collect::<Vec<_>>(),
            "groups": self.groups.iter().map(|(_, g)| g.status()).collect::<Vec<_>>(),
            "interval": self.control.interval().map(|d| d.as_secs()),
        })
    }

    /// Probe each member of each group, each interval, until the end.
    pub async fn probe_forever(&self) {
        if self.health.is_none() {
            return std::future::pending().await;
        }
        loop {
            self.probe_round().await;
            self.control.pause().await;
        }
    }

    /// One round: probe each member of each group. The control of a group
    /// passes when one member or more reached the target: a failure of each
    /// member is an outage of the host. A member with an egress check
    /// (`health.egress`) also asks the echo endpoint for its address.
    pub async fn probe_round(&self) {
        let Some((target, thresholds)) = self.health.clone() else {
            return;
        };
        for (_, group) in &self.groups {
            let mut results = Vec::new();
            for member in group.members() {
                let mut result = probe(&member.way, &target, thresholds.timeout).await;
                if let (crate::network::ProbeResult::Ok(_), Some((echo, expect))) =
                    (result, self.egress.get(&member.name))
                {
                    result = probe_egress(&member.way, echo, *expect, thresholds.timeout).await;
                }
                results.push(result);
            }
            let control = results
                .iter()
                .any(|r| matches!(r, crate::network::ProbeResult::Ok(_)));
            let now = Instant::now();
            for (i, result) in results.into_iter().enumerate() {
                group.observe(i, result, control, now, &thresholds);
            }
            group.end_round();
        }
    }
}

#[cfg(test)]
mod health_tests;
#[cfg(test)]
mod resolver_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod udp_tests;
