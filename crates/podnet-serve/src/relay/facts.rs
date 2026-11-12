//! The facts of the SSH forward relays: the endpoints, the user and the
//! rule of the URL of the tunnel. They are data of `pod-services`
//! (`forward::Relay`). This code knows no relay host.

use std::time::Duration;

use podnet_core::services::{self, forward};

pub use forward::{Banner, UrlRule};

/// The relay that a command uses when it names none.
pub const DEFAULT: &str = "pinggy";

/// The facts of one relay, ready for a [`super::RelayConfig`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Facts {
    /// The id of the service in `pod-services`.
    pub name: String,
    /// `host:port` of each plain SSH endpoint, in the order to try.
    pub addresses: Vec<String>,
    /// The SSH user of a free tunnel. `None`: the relay takes any user.
    pub user: Option<String>,
    /// Which URL of the banner is the tunnel URL. It also skips each known
    /// dashboard host (see [`dashboards`]).
    pub rule: UrlRule,
    /// The interval of the keepalive that the relay states.
    pub keepalive: Option<Duration>,
}

/// The ids of the known forward relays.
pub fn names() -> Vec<&'static str> {
    services::forward_relays()
        .iter()
        .map(|relay| relay.service().id())
        .collect()
}

/// The texts of `url-skip` of every known relay: the dashboard hosts. A
/// banner can name the dashboard of any relay, so a rule skips all of them.
pub fn dashboards() -> Vec<String> {
    let mut all: Vec<String> = Vec::new();
    for relay in services::forward_relays() {
        for skip in relay.url_rule().skips {
            if !all.contains(&skip) {
                all.push(skip);
            }
        }
    }
    all
}

/// The facts of the relay `name`, or the list of the known names.
pub fn lookup(name: &str) -> Result<Facts, String> {
    let relays = services::forward_relays();
    let Some(relay) = relays.iter().find(|r| r.service().id() == name) else {
        let known: Vec<&str> = relays.iter().map(|r| r.service().id()).collect();
        return Err(format!(
            "no SSH forward relay {name:?}; known: {}",
            known.join(", ")
        ));
    };
    let mut rule = relay.url_rule();
    for dashboard in dashboards() {
        if !rule.skips.contains(&dashboard) {
            rule.skips.push(dashboard);
        }
    }
    Ok(Facts {
        name: name.to_string(),
        addresses: relay.endpoints().map(|e| e.authority()).collect(),
        user: relay.user().map(str::to_string),
        rule,
        keepalive: relay.keepalive(),
    })
}

/// The facts of [`DEFAULT`].
pub fn default_facts() -> Facts {
    lookup(DEFAULT).unwrap_or_else(|e| panic!("pod-services: {e}"))
}

/// The URL rule of [`DEFAULT`].
pub fn default_rule() -> UrlRule {
    default_facts().rule
}
