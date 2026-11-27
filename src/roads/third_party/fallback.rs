//! Adapter fallback class and road selection rules (T-368).
//!
//! Overlay adapters (`tor`, `iroh`, `zerotier`, `tailscale`) belong to
//! [`RoadClass::FallbackAdapter`]. Default road selection never picks a fallback adapter
//! without an explicit flag or policy rule naming it.

use serde::{Deserialize, Serialize};

/// Classification of a road for selection policy enforcement.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoadClass {
    Direct,
    Relay,
    ThirdParty,
    FallbackAdapter,
}

/// Overlay network adapters restricted to the fallback class.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AdapterKind {
    Tor,
    Iroh,
    ZeroTier,
    Tailscale,
}

impl AdapterKind {
    pub fn as_str(self) -> &'static str {
        match self {
            AdapterKind::Tor => "tor",
            AdapterKind::Iroh => "iroh",
            AdapterKind::ZeroTier => "zerotier",
            AdapterKind::Tailscale => "tailscale",
        }
    }
}

/// Metadata entry for a registered fallback adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdapterEntry {
    pub name: &'static str,
    pub kind: AdapterKind,
    pub class: RoadClass,
    pub description: &'static str,
}

/// Registered overlay adapters in the fallback class.
pub const ADAPTER_REGISTRY: [AdapterEntry; 4] = [
    AdapterEntry {
        name: "tor",
        kind: AdapterKind::Tor,
        class: RoadClass::FallbackAdapter,
        description: "Tor bridge fallback adapter; selected only when explicitly named",
    },
    AdapterEntry {
        name: "iroh",
        kind: AdapterKind::Iroh,
        class: RoadClass::FallbackAdapter,
        description: "iroh overlay fallback adapter; selected only when explicitly named",
    },
    AdapterEntry {
        name: "zerotier",
        kind: AdapterKind::ZeroTier,
        class: RoadClass::FallbackAdapter,
        description: "ZeroTier overlay fallback adapter; selected only when explicitly named",
    },
    AdapterEntry {
        name: "tailscale",
        kind: AdapterKind::Tailscale,
        class: RoadClass::FallbackAdapter,
        description: "Tailscale overlay fallback adapter; selected only when explicitly named",
    },
];

/// Classify a road name into its [`RoadClass`].
pub fn classify_road(name: &str) -> RoadClass {
    match name.trim().to_ascii_lowercase().as_str() {
        "tor" | "tor-bridge" | "iroh" | "zerotier" | "tailscale" => RoadClass::FallbackAdapter,
        "irc" | "mqtt" | "syncthing" | "piping" | "wormhole-transit" => RoadClass::ThirdParty,
        "relay" | "tunnel" | "derp" | "turn" => RoadClass::Relay,
        _ => RoadClass::Direct,
    }
}

/// One candidate road offered to [`select_road`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoadCandidate {
    pub name: String,
    pub class: RoadClass,
    /// Lower priority value is preferred first.
    pub priority: u32,
    pub available: bool,
}

impl RoadCandidate {
    /// Build a candidate using [`classify_road`] for its class.
    pub fn new(name: impl Into<String>, priority: u32, available: bool) -> Self {
        let name = name.into();
        let class = classify_road(&name);
        Self {
            name,
            class,
            priority,
            available,
        }
    }
}

/// Policy controlling road selection.
///
/// `SelectionPolicy::default()` prohibits all [`RoadClass::FallbackAdapter`] and
/// [`RoadClass::ThirdParty`] roads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SelectionPolicy {
    /// Explicit CLI flag naming one adapter (for example `--adapter iroh`).
    pub explicit_adapter: Option<String>,
    /// Policy rules explicitly naming allowed fallback adapters.
    pub allowed_adapters: Vec<String>,
    /// Permit third-party roads when explicitly enabled by the user.
    pub allow_third_party: bool,
}

impl SelectionPolicy {
    /// True when `name` is explicitly permitted as a fallback adapter.
    pub fn permits_adapter(&self, name: &str) -> bool {
        if let Some(explicit) = &self.explicit_adapter {
            if explicit.eq_ignore_ascii_case(name) {
                return true;
            }
        }
        self.allowed_adapters
            .iter()
            .any(|rule| rule.eq_ignore_ascii_case(name))
    }
}

/// Select the best eligible road candidate under `policy`.
///
/// Without an explicit flag or policy rule naming an adapter, no
/// [`RoadClass::FallbackAdapter`] road is ever selected, even when no other road
/// is available.
pub fn select_road<'a>(
    candidates: &'a [RoadCandidate],
    policy: &SelectionPolicy,
) -> Option<&'a RoadCandidate> {
    candidates
        .iter()
        .filter(|c| c.available)
        .filter(|c| match c.class {
            RoadClass::Direct | RoadClass::Relay => true,
            RoadClass::ThirdParty => policy.allow_third_party,
            RoadClass::FallbackAdapter => policy.permits_adapter(&c.name),
        })
        .min_by_key(|c| c.priority)
}
