//! The router of `podnet route` (T-100): a config file of networks, groups,
//! doors and a health probe, read with the YAML reader of `podnet-core`.
//!
//! ```yaml
//! networks:            # NAME: SPEC of the registry (crate::networks)
//!   home: direct
//!   proxy: socks5h://127.0.0.1:1080
//! groups:              # NAME: MEMBER*WEIGHT, ... (T-096)
//!   egress: home*2, proxy*1
//! health:              # the probe of the members of each group (T-097)
//!   target: 1.1.1.1:443
//!   interval: 10
//! doors:               # SOCKS5 and HTTP CONNECT doors (T-061)
//!   - listen: unix:/run/r.sock
//!     via: egress      # a network or a group
//!     rules: /etc/r.rules   # optional: the rules of T-098
//! ```

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use podnet_netns::listen::ProxyListen;
use podnet_vnet::door::Door;
use podnet_vnet::routes::RulesFile;
use serde::Deserialize;
use tokio::time::Instant;

use crate::network::{
    parse_group, probe, Balance, Choice, Ctx, Dial, Member, Network as _, Selection, Target,
    Thresholds, Way,
};

/// The config file.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub networks: BTreeMap<String, String>,
    #[serde(default)]
    pub groups: BTreeMap<String, String>,
    #[serde(default)]
    pub health: Option<Health>,
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
    if config.doors.is_empty() {
        bail!("doors: give one door or more");
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
    for (i, door) in config.doors.iter().enumerate() {
        if !config.networks.contains_key(&door.via) && !config.groups.contains_key(&door.via) {
            bail!(
                "doors[{i}].via: {:?} is not a network or a group of the file",
                door.via
            );
        }
        ProxyListen::parse(&door.listen).map_err(|e| anyhow::anyhow!("doors[{i}].listen: {e}"))?;
    }
    if let Some(health) = &config.health {
        health
            .target
            .parse::<Target>()
            .map_err(|_| anyhow::anyhow!("health.target: give HOST:PORT"))?;
    }
    Ok(config)
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
}

/// A router that is up: its ways, its groups and its doors.
pub struct Router {
    pub ways: HashMap<String, Way>,
    pub groups: Vec<(String, Arc<Balance>)>,
    pub doors: Vec<RouterDoor>,
    pub health: Option<(Target, Duration)>,
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
        for (name, spec) in &config.networks {
            let network = crate::networks::parse(spec)
                .map_err(|e| anyhow::anyhow!("networks.{name}: {}", e.text()))?;
            let link = network
                .up(&Ctx::default())
                .await
                .with_context(|| format!("networks.{name}"))?;
            let way = link
                .way
                .with_context(|| format!("networks.{name}: the network gives no way out"))?;
            ways.insert(name.clone(), way);
        }
        Router::build(config, ways)
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
            let group = Arc::new(
                Balance::new(name, built)
                    .map_err(|e| anyhow::anyhow!("groups.{name}: {}", e.text()))?,
            );
            ways.insert(name.clone(), Way::destination(group.clone()));
            groups.push((name.clone(), group));
        }
        let mut doors = Vec::new();
        for (i, door) in config.doors.iter().enumerate() {
            let way = ways[&door.via].clone();
            let rules = match &door.rules {
                Some(path) => Some(Arc::new(
                    RulesFile::open(path, ways.clone(), Some(way.clone()))
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
            doors.push(RouterDoor {
                listen,
                door: Arc::new(Door::new(dial.clone())),
                label: dial.label(),
                via: door.via.clone(),
                rules,
            });
        }
        let health = match &config.health {
            Some(h) => Some((
                h.target
                    .parse()
                    .map_err(|_| anyhow::anyhow!("health.target"))?,
                Duration::from_secs(h.interval.unwrap_or(10).max(1)),
            )),
            None => None,
        };
        Ok(Router {
            ways,
            groups,
            doors,
            health,
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
        })
    }

    /// Probe each member of each group, each interval, until the end. The
    /// control of an interval passes when one member or more reached the
    /// target: a failure of each member is an outage of the host.
    pub async fn probe_forever(&self) {
        let Some((target, interval)) = self.health.clone() else {
            return std::future::pending().await;
        };
        let thresholds = Thresholds {
            interval,
            ..Thresholds::default()
        };
        loop {
            for (_, group) in &self.groups {
                let mut results = Vec::new();
                for member in group.members() {
                    results.push(probe(&member.way, &target, thresholds.timeout).await);
                }
                let control = results
                    .iter()
                    .any(|r| matches!(r, crate::network::ProbeResult::Ok(_)));
                let now = Instant::now();
                for (i, result) in results.into_iter().enumerate() {
                    group.observe(i, result, control, now, &thresholds);
                }
            }
            tokio::time::sleep(interval).await;
        }
    }
}

#[cfg(test)]
mod tests;
