//! Third-party service roads, watch checks, front probes, and fallback class (T-368).
//!
//! Each third-party road uses one common descriptor (`ThirdPartyRoad`), is disabled by
//! default, carries a cost class, and records a UTC verification date (`YYYY-MM-DD`).
//! Users pass endpoints at runtime; this module contains no hardcoded third-party hostnames.

pub mod fallback;
pub mod front;
pub mod irc;
pub mod mqtt;
pub mod syncthing;
pub mod watch;

use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Result};
use pod_link::Cost;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::carriers::cost_name;

pub use fallback::{
    classify_road, select_road, AdapterEntry, AdapterKind, RoadCandidate, RoadClass,
    SelectionPolicy, ADAPTER_REGISTRY,
};
pub use front::{check_front, parse_front_param, FrontCheckError, MAX_FRONT_CHECK_TIMEOUT};
pub use watch::{probe_registry, probe_road, DEFAULT_PROBE_TIMEOUT};

/// Default staleness threshold in days for third-party road checks.
pub const DEFAULT_STALE_AFTER_DAYS: u32 = 30;

/// Supported third-party road kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ThirdPartyKind {
    Irc,
    Mqtt,
    Syncthing,
    Piping,
    WormholeTransit,
}

impl ThirdPartyKind {
    /// All registered third-party road kinds in fixed display order.
    pub const ALL: [ThirdPartyKind; 5] = [
        ThirdPartyKind::Irc,
        ThirdPartyKind::Mqtt,
        ThirdPartyKind::Syncthing,
        ThirdPartyKind::Piping,
        ThirdPartyKind::WormholeTransit,
    ];

    /// Canonical wire and CLI name of the road kind.
    pub fn as_str(self) -> &'static str {
        match self {
            ThirdPartyKind::Irc => "irc",
            ThirdPartyKind::Mqtt => "mqtt",
            ThirdPartyKind::Syncthing => "syncthing",
            ThirdPartyKind::Piping => "piping",
            ThirdPartyKind::WormholeTransit => "wormhole-transit",
        }
    }

    /// Parse a kind name.
    pub fn parse(name: &str) -> Option<Self> {
        match name.trim() {
            "irc" => Some(ThirdPartyKind::Irc),
            "mqtt" => Some(ThirdPartyKind::Mqtt),
            "syncthing" => Some(ThirdPartyKind::Syncthing),
            "piping" => Some(ThirdPartyKind::Piping),
            "wormhole-transit" => Some(ThirdPartyKind::WormholeTransit),
            _ => None,
        }
    }

    /// Default cost class for this third-party road.
    pub fn default_cost(self) -> Cost {
        match self {
            ThirdPartyKind::Irc => Cost::High,
            ThirdPartyKind::Mqtt => Cost::Medium,
            ThirdPartyKind::Syncthing => Cost::Medium,
            ThirdPartyKind::Piping => Cost::Medium,
            ThirdPartyKind::WormholeTransit => Cost::Medium,
        }
    }

    /// Address template shown in carrier listings.
    pub fn address_form(self) -> &'static str {
        match self {
            ThirdPartyKind::Irc => "irc://HOST:PORT/CHANNEL",
            ThirdPartyKind::Mqtt => "mqtt://HOST:PORT/TOPIC",
            ThirdPartyKind::Syncthing => "syncthing://HOST:PORT/?id=DEVICE_ID",
            ThirdPartyKind::Piping => "https://HOST[:PORT]/PATH",
            ThirdPartyKind::WormholeTransit => "tcp:HOST:PORT",
        }
    }

    /// One-line description of the road.
    pub fn about(self) -> &'static str {
        match self {
            ThirdPartyKind::Irc => "chunked base64 PRIVMSG stream over an IRC channel",
            ThirdPartyKind::Mqtt => "MQTT 3.1.1 publish/subscribe byte stream over TCP",
            ThirdPartyKind::Syncthing => "Syncthing Relay Protocol v1 binary session over TCP/TLS",
            ThirdPartyKind::Piping => "HTTP streaming relay through a piping server",
            ThirdPartyKind::WormholeTransit => "magic-wormhole transit relay over TCP",
        }
    }

    /// Matching `pod-services` kind when one exists in the built-in catalog.
    fn pod_services_kind(self) -> Option<&'static str> {
        match self {
            ThirdPartyKind::Irc => None,
            ThirdPartyKind::Mqtt => Some("mqtt-broker"),
            ThirdPartyKind::Syncthing => Some("syncthing-relay"),
            ThirdPartyKind::Piping => Some("piping"),
            ThirdPartyKind::WormholeTransit => Some("transit"),
        }
    }
}

impl fmt::Display for ThirdPartyKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// UTC calendar date (`YYYY-MM-DD`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VerifiedDate {
    pub year: u16,
    pub month: u8,
    pub day: u8,
}

impl VerifiedDate {
    /// Construct a validated calendar date.
    pub fn new(year: u16, month: u8, day: u8) -> Result<Self> {
        let leap =
            year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
        let max_day = match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 if leap => 29,
            2 => 28,
            _ => bail!("invalid calendar month {month}"),
        };
        if !(1..=max_day).contains(&day) {
            bail!("invalid calendar day {day} for {year:04}-{month:02}");
        }
        Ok(Self { year, month, day })
    }

    /// Parse a strict `YYYY-MM-DD` UTC date string.
    pub fn parse(text: &str) -> Result<Self> {
        let trimmed = text.trim();
        let parts: Vec<&str> = trimmed.split('-').collect();
        if parts.len() != 3 || parts[0].len() != 4 || parts[1].len() != 2 || parts[2].len() != 2 {
            bail!("expected YYYY-MM-DD date, got {text:?}");
        }
        let year: u16 = parts[0]
            .parse()
            .map_err(|_| anyhow!("invalid year in {text:?}"))?;
        let month: u8 = parts[1]
            .parse()
            .map_err(|_| anyhow!("invalid month in {text:?}"))?;
        let day: u8 = parts[2]
            .parse()
            .map_err(|_| anyhow!("invalid day in {text:?}"))?;
        Self::new(year, month, day)
    }

    /// Convert a `pod-services` date.
    pub fn from_pod_date(d: pod_services::Date) -> Self {
        Self {
            year: d.year(),
            month: d.month(),
            day: d.day(),
        }
    }

    /// Current UTC calendar date from the system clock.
    pub fn today_utc() -> Self {
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let days = (secs / 86_400) as i64;
        civil_from_days(days)
    }

    /// Signed day difference `self - earlier`.
    pub fn days_since(self, earlier: VerifiedDate) -> i64 {
        days_from_civil(self) - days_from_civil(earlier)
    }
}

impl fmt::Display for VerifiedDate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

impl Serialize for VerifiedDate {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for VerifiedDate {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        VerifiedDate::parse(&s).map_err(serde::de::Error::custom)
    }
}

/// Convert Gregorian date to days since 1970-01-01 (Howard Hinnant's algorithm).
fn days_from_civil(date: VerifiedDate) -> i64 {
    let mut y = i64::from(date.year);
    let m = i64::from(date.month);
    let d = i64::from(date.day);
    if m <= 2 {
        y -= 1;
    }
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Convert days since 1970-01-01 to a Gregorian date.
fn civil_from_days(days: i64) -> VerifiedDate {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    VerifiedDate {
        year: year.clamp(0, i64::from(u16::MAX)) as u16,
        month: m as u8,
        day: d as u8,
    }
}

/// Watch health state of a third-party road.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum WatchState {
    Ok,
    Degraded {
        reason: String,
    },
    Stale {
        verified: VerifiedDate,
        max_age_days: u32,
    },
}

impl WatchState {
    /// Construct a `Degraded` state with a reason.
    pub fn degraded(reason: impl Into<String>) -> Self {
        WatchState::Degraded {
            reason: reason.into(),
        }
    }

    pub fn is_ok(&self) -> bool {
        matches!(self, WatchState::Ok)
    }

    pub fn is_degraded(&self) -> bool {
        matches!(self, WatchState::Degraded { .. })
    }

    pub fn is_stale(&self) -> bool {
        matches!(self, WatchState::Stale { .. })
    }

    /// Short state label for text output.
    pub fn label(&self) -> &'static str {
        match self {
            WatchState::Ok => "ok",
            WatchState::Degraded { .. } => "degraded",
            WatchState::Stale { .. } => "stale",
        }
    }
}

impl fmt::Display for WatchState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WatchState::Ok => f.write_str("ok"),
            WatchState::Degraded { reason } => write!(f, "degraded: {reason}"),
            WatchState::Stale {
                verified,
                max_age_days,
            } => write!(f, "stale: verified {verified}, max age {max_age_days} d"),
        }
    }
}

/// Unified descriptor for a registered third-party road.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThirdPartyRoad {
    pub kind: ThirdPartyKind,
    pub name: String,
    pub endpoint: Option<String>,
    pub cost_class: Cost,
    pub enabled: bool,
    pub last_verified: VerifiedDate,
    pub stale_after_days: u32,
    pub state: WatchState,
    pub address_form: &'static str,
    pub about: &'static str,
}

impl ThirdPartyRoad {
    /// Create a third-party road entry. Third-party roads start disabled by default.
    pub fn new(
        kind: ThirdPartyKind,
        cost_class: Cost,
        last_verified: VerifiedDate,
        stale_after_days: u32,
    ) -> Self {
        Self {
            kind,
            name: kind.as_str().to_string(),
            endpoint: None,
            cost_class,
            enabled: false,
            last_verified,
            stale_after_days,
            state: WatchState::Ok,
            address_form: kind.address_form(),
            about: kind.about(),
        }
    }

    /// Attach a user-supplied endpoint URL.
    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = Some(endpoint.into());
        self
    }

    /// Enable or disable this road.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Cost class word (`low`, `medium`, `high`, `highest`).
    pub fn cost_name(&self) -> &'static str {
        cost_name(self.cost_class)
    }

    /// Evaluate staleness against `today`. A stale road reports `WatchState::Stale`.
    pub fn evaluate_state(&self, today: VerifiedDate) -> WatchState {
        if let WatchState::Degraded { .. } = &self.state {
            return self.state.clone();
        }
        if today.days_since(self.last_verified) > i64::from(self.stale_after_days) {
            return WatchState::Stale {
                verified: self.last_verified,
                max_age_days: self.stale_after_days,
            };
        }
        WatchState::Ok
    }

    /// Update `self.state` using `today` and return the resulting state.
    pub fn refresh_staleness(&mut self, today: VerifiedDate) -> WatchState {
        let next = self.evaluate_state(today);
        self.state = next.clone();
        next
    }

    /// True only when the road is neither degraded nor stale on `today`.
    pub fn is_working(&self, today: VerifiedDate) -> bool {
        self.evaluate_state(today).is_ok()
    }

    /// Single-line carrier summary with cost class and last-verified date.
    pub fn line(&self) -> String {
        let enabled = if self.enabled { "enabled" } else { "disabled" };
        format!(
            "{name}: third-party road, cost {cost}, last-verified {verified}, max-age {max_age}d, {enabled}, address {addr}: {state}",
            name = self.name,
            cost = self.cost_name(),
            verified = self.last_verified,
            max_age = self.stale_after_days,
            addr = self.address_form,
            state = self.state,
        )
    }

    /// JSON representation for `podnet link carriers --third-party --json`.
    pub fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "kind": self.kind.as_str(),
            "class": "third_party",
            "about": self.about,
            "address": self.address_form,
            "cost": self.cost_name(),
            "enabled": self.enabled,
            "default": false,
            "last_verified": self.last_verified.to_string(),
            "stale_after_days": self.stale_after_days,
            "state": self.state,
        })
    }
}

/// Registry of third-party roads with staleness and oldest-date reporting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThirdPartyRegistry {
    roads: Vec<ThirdPartyRoad>,
}

impl ThirdPartyRegistry {
    /// Build a registry from explicit road descriptors.
    pub fn new(roads: Vec<ThirdPartyRoad>) -> Self {
        Self { roads }
    }

    /// Default registry for all five third-party road kinds.
    ///
    /// Verification dates come from built-in `pod-services` metadata when present,
    /// without hardcoding any third-party hostnames. Every road starts disabled.
    pub fn default_registry() -> Self {
        let fallback_date = VerifiedDate {
            year: 2026,
            month: 11,
            day: 27,
        };
        let roads = ThirdPartyKind::ALL
            .iter()
            .map(|&kind| {
                let verified = verified_date_from_catalog(kind).unwrap_or(fallback_date);
                ThirdPartyRoad::new(
                    kind,
                    kind.default_cost(),
                    verified,
                    DEFAULT_STALE_AFTER_DAYS,
                )
            })
            .collect();
        Self { roads }
    }

    pub fn roads(&self) -> &[ThirdPartyRoad] {
        &self.roads
    }

    pub fn roads_mut(&mut self) -> &mut [ThirdPartyRoad] {
        &mut self.roads
    }

    pub fn into_roads(self) -> Vec<ThirdPartyRoad> {
        self.roads
    }

    pub fn get(&self, kind: ThirdPartyKind) -> Option<&ThirdPartyRoad> {
        self.roads.iter().find(|r| r.kind == kind)
    }

    pub fn get_mut(&mut self, kind: ThirdPartyKind) -> Option<&mut ThirdPartyRoad> {
        self.roads.iter_mut().find(|r| r.kind == kind)
    }

    /// Toggle a third-party road on or off.
    pub fn set_enabled(&mut self, kind: ThirdPartyKind, enabled: bool) -> bool {
        if let Some(road) = self.get_mut(kind) {
            road.set_enabled(enabled);
            true
        } else {
            false
        }
    }

    /// Oldest `last_verified` date across registered roads.
    pub fn oldest_verified(&self) -> Option<VerifiedDate> {
        self.roads.iter().map(|r| r.last_verified).min()
    }

    /// Roads that are stale as of `today`.
    pub fn stale_roads(&self, today: VerifiedDate) -> Vec<&ThirdPartyRoad> {
        self.roads
            .iter()
            .filter(|r| r.evaluate_state(today).is_stale())
            .collect()
    }
}

/// Convenience helper returning the default third-party road registry.
pub fn default_registry() -> ThirdPartyRegistry {
    ThirdPartyRegistry::default_registry()
}

/// Read the latest recorded check date for a road kind from `pod-services` metadata.
fn verified_date_from_catalog(kind: ThirdPartyKind) -> Option<VerifiedDate> {
    let svc_kind = kind.pod_services_kind()?;
    pod_services::of_kind(svc_kind)
        .flat_map(|svc| svc.endpoints().iter())
        .map(|ep| VerifiedDate::from_pod_date(ep.source().read()))
        .max()
}
