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
    parse_group, probe, Balance, Ctx, Dial, Member, Network as _, Target, Thresholds, Way,
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

/// A router that is up: its ways, its groups and its doors.
pub struct Router {
    pub ways: HashMap<String, Way>,
    pub groups: Vec<(String, Arc<Balance>)>,
    pub doors: Vec<(ProxyListen, Arc<Door>, String)>,
    pub health: Option<(Target, Duration)>,
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
            let dial: Arc<dyn Dial> = match &door.rules {
                Some(path) => Arc::new(
                    RulesFile::open(path, ways.clone(), Some(way))
                        .with_context(|| format!("doors[{i}].rules"))?,
                ),
                None => way.dial,
            };
            let listen = ProxyListen::parse(&door.listen)
                .map_err(|e| anyhow::anyhow!("doors[{i}].listen: {e}"))?;
            doors.push((listen, Arc::new(Door::new(dial.clone())), dial.label()));
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

    /// The ready document: each door and the status of each group.
    pub fn status(&self) -> serde_json::Value {
        serde_json::json!({
            "doors": self.doors.iter().map(|(listen, _, label)| serde_json::json!({"listen": listen.to_string(), "via": label})).collect::<Vec<_>>(),
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
