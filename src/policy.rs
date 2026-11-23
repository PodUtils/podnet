//! The egress policy of podnet#500, EGRESS-1 to EGRESS-5.
//!
//! A destination is classified before any connection: `direct`, `proxy`,
//! `relay`, `refuse` or `queue`. The inputs are the destination, its
//! resolved address family, the port and the rules of the deployment. The
//! classification lives here, with the rules; a socket asks this module and
//! does not decide for itself.
//!
//! A refusal carries its reason. It is a policy decision, so it is not a
//! timeout and it reaches the caller before `connect(2)`.
//!
//! A rule that an earlier rule covers can never match: [`Policy::shadowed`]
//! names it, and `podnet rules check` exits 78 for it.

use std::net::IpAddr;

use serde::Deserialize;

/// What the policy does with a destination.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Direct,
    Proxy,
    Relay,
    Refuse,
    Queue,
}

impl Action {
    pub fn as_str(self) -> &'static str {
        match self {
            Action::Direct => "direct",
            Action::Proxy => "proxy",
            Action::Relay => "relay",
            Action::Refuse => "refuse",
            Action::Queue => "queue",
        }
    }

    fn refusal_is_an_error(self) -> bool {
        matches!(self, Action::Refuse | Action::Queue)
    }
}

/// The address family of a destination. A name takes `Any` until it is
/// resolved.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Family {
    #[default]
    Any,
    Ipv4,
    Ipv6,
}

impl Family {
    pub fn of_ip(ip: IpAddr) -> Family {
        match ip {
            IpAddr::V4(_) => Family::Ipv4,
            IpAddr::V6(_) => Family::Ipv6,
        }
    }

    /// The family of `host`, when it is an address.
    pub fn of_host(host: &str) -> Family {
        host.parse::<IpAddr>()
            .map(Family::of_ip)
            .unwrap_or_default()
    }

    fn covers(self, other: Family) -> bool {
        self == Family::Any || self == other
    }
}

/// A host pattern: `*`, an exact name or `*.suffix`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostPattern {
    Any,
    Exact(String),
    Suffix(String),
}

impl HostPattern {
    fn parse(text: &str) -> HostPattern {
        let text = text.trim().to_ascii_lowercase();
        match text.as_str() {
            "*" | "" => HostPattern::Any,
            _ => match text.strip_prefix("*.") {
                Some(suffix) => HostPattern::Suffix(suffix.to_string()),
                None => HostPattern::Exact(text),
            },
        }
    }

    fn matches(&self, host: &str) -> bool {
        let host = host.to_ascii_lowercase();
        match self {
            HostPattern::Any => true,
            HostPattern::Exact(name) => *name == host,
            HostPattern::Suffix(suffix) => {
                host == *suffix
                    || host
                        .strip_suffix(suffix)
                        .is_some_and(|head| head.ends_with('.'))
            }
        }
    }

    fn covers(&self, other: &HostPattern) -> bool {
        match (self, other) {
            (HostPattern::Any, _) => true,
            (HostPattern::Exact(a), HostPattern::Exact(b)) => a == b,
            (HostPattern::Suffix(a), HostPattern::Suffix(b)) => {
                a == b || b.strip_suffix(a).is_some_and(|head| head.ends_with('.'))
            }
            (HostPattern::Suffix(a), HostPattern::Exact(b)) => {
                b == a || b.strip_suffix(a).is_some_and(|head| head.ends_with('.'))
            }
            _ => false,
        }
    }
}

/// The ports of a rule: all, one, or an inclusive range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ports {
    Any,
    One(u16),
    Range(u16, u16),
}

impl Ports {
    fn parse(text: &str) -> anyhow::Result<Ports> {
        let text = text.trim();
        if text == "*" || text.is_empty() {
            return Ok(Ports::Any);
        }
        if let Some((low, high)) = text.split_once('-') {
            let low: u16 = low
                .trim()
                .parse()
                .map_err(|_| anyhow::anyhow!("the port {low:?} is not 0 to 65535"))?;
            let high: u16 = high
                .trim()
                .parse()
                .map_err(|_| anyhow::anyhow!("the port {high:?} is not 0 to 65535"))?;
            if low > high {
                anyhow::bail!("the port range {low}-{high} is empty");
            }
            return Ok(Ports::Range(low, high));
        }
        let port: u16 = text
            .parse()
            .map_err(|_| anyhow::anyhow!("the port {text:?} is not 0 to 65535"))?;
        Ok(Ports::One(port))
    }

    fn matches(self, port: u16) -> bool {
        match self {
            Ports::Any => true,
            Ports::One(one) => one == port,
            Ports::Range(low, high) => (low..=high).contains(&port),
        }
    }

    fn covers(self, other: Ports) -> bool {
        match (self, other) {
            (Ports::Any, _) => true,
            (Ports::One(a), Ports::One(b)) => a == b,
            (Ports::Range(low, high), Ports::One(b)) => (low..=high).contains(&b),
            (Ports::Range(low, high), Ports::Range(a, b)) => low <= a && high >= b,
            _ => false,
        }
    }
}

/// One rule of the policy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    pub name: String,
    pub host: HostPattern,
    pub ports: Ports,
    pub family: Family,
    pub action: Action,
}

impl Rule {
    fn matches(&self, destination: &Destination) -> bool {
        self.host.matches(&destination.host)
            && self.ports.matches(destination.port)
            && self.family.covers(destination.family)
    }

    fn covers(&self, later: &Rule) -> bool {
        self.host.covers(&later.host)
            && self.ports.covers(later.ports)
            && self.family.covers(later.family)
    }
}

/// A destination for the classification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Destination {
    pub host: String,
    pub port: u16,
    pub family: Family,
}

impl Destination {
    pub fn new(host: impl Into<String>, port: u16) -> Destination {
        let host = host.into();
        let family = Family::of_host(&host);
        Destination { host, port, family }
    }

    pub fn with_family(host: impl Into<String>, port: u16, family: Family) -> Destination {
        Destination {
            host: host.into(),
            port,
            family,
        }
    }
}

/// The result of the classification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verdict {
    pub action: Action,
    pub rule: Option<String>,
    /// The reason of a refusal or of a queue.
    pub reason: Option<String>,
}

impl Verdict {
    fn allow(action: Action, rule: Option<&str>) -> Verdict {
        Verdict {
            action,
            rule: rule.map(str::to_string),
            reason: None,
        }
    }

    fn refuse(action: Action, reason: String) -> Verdict {
        Verdict {
            action,
            rule: None,
            reason: Some(reason),
        }
    }
}

/// A rule that an earlier rule covers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shadow {
    pub earlier: String,
    pub later: String,
    pub reason: String,
}

impl std::fmt::Display for Shadow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "the rule {:?} is shadowed by the earlier rule {:?} and is unreachable: {}",
            self.later, self.earlier, self.reason
        )
    }
}

/// The rules of one deployment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Policy {
    pub rules: Vec<Rule>,
    pub default: Action,
}

#[derive(Deserialize)]
struct RawPolicy {
    #[serde(default = "default_action")]
    default: Action,
    rules: Vec<RawRule>,
}

fn default_action() -> Action {
    Action::Refuse
}

#[derive(Deserialize)]
struct RawRule {
    name: String,
    host: String,
    #[serde(default = "any_port")]
    port: String,
    #[serde(default)]
    family: Family,
    action: Action,
}

fn any_port() -> String {
    "*".into()
}

impl Policy {
    /// The policy of a JSON document: `{"default": "refuse", "rules": [...]}`.
    pub fn parse(text: &str) -> anyhow::Result<Policy> {
        let raw: RawPolicy = serde_json::from_str(text)
            .map_err(|e| anyhow::anyhow!("the rules are not JSON of the policy: {e}"))?;
        Policy::from_raw(raw)
    }

    /// The policy of a file.
    pub fn load(path: &std::path::Path) -> anyhow::Result<Policy> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
        Policy::parse(&text).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))
    }

    fn from_raw(raw: RawPolicy) -> anyhow::Result<Policy> {
        let mut names = std::collections::BTreeSet::new();
        let mut rules = Vec::with_capacity(raw.rules.len());
        for (index, rule) in raw.rules.into_iter().enumerate() {
            if rule.name.trim().is_empty() {
                anyhow::bail!("rule {}: the name is empty", index + 1);
            }
            if !names.insert(rule.name.clone()) {
                anyhow::bail!("rule {}: the name {:?} is twice", index + 1, rule.name);
            }
            rules.push(Rule {
                name: rule.name,
                host: HostPattern::parse(&rule.host),
                ports: Ports::parse(&rule.port)?,
                family: rule.family,
                action: rule.action,
            });
        }
        Ok(Policy {
            rules,
            default: raw.default,
        })
    }

    /// The action for `destination`, with the rule that gave it.
    pub fn classify(&self, destination: &Destination) -> Verdict {
        if let Some(rule) = self.rules.iter().find(|r| r.matches(destination)) {
            if rule.action.refusal_is_an_error() {
                return Verdict {
                    action: rule.action,
                    rule: Some(rule.name.clone()),
                    reason: Some(format!(
                        "the rule {:?} {} the destination {}:{} ({}), before any connect",
                        rule.name,
                        rule.action.as_str(),
                        destination.host,
                        destination.port,
                        destination.family_name()
                    )),
                };
            }
            return Verdict::allow(rule.action, Some(&rule.name));
        }
        if self.default.refusal_is_an_error() {
            return Verdict::refuse(
                self.default,
                format!(
                    "no rule matches {}:{} ({}), so the policy {} it before any connect (EGRESS-3)",
                    destination.host,
                    destination.port,
                    destination.family_name(),
                    self.default.as_str()
                ),
            );
        }
        Verdict::allow(self.default, None)
    }

    /// Each later rule that an earlier rule covers.
    pub fn shadowed(&self) -> Vec<Shadow> {
        let mut out = Vec::new();
        for (index, later) in self.rules.iter().enumerate() {
            for earlier in &self.rules[..index] {
                if earlier.covers(later) {
                    out.push(Shadow {
                        earlier: earlier.name.clone(),
                        later: later.name.clone(),
                        reason: format!(
                            "the earlier rule matches each host, port and family of the later rule ({}, {}, {})",
                            earlier.host_text(),
                            earlier.port_text(),
                            earlier.family_name()
                        ),
                    });
                }
            }
        }
        out
    }
}

impl Destination {
    fn family_name(&self) -> &'static str {
        match self.family {
            Family::Any => "any family",
            Family::Ipv4 => "IPv4",
            Family::Ipv6 => "IPv6",
        }
    }
}

impl Rule {
    fn host_text(&self) -> String {
        match &self.host {
            HostPattern::Any => "*".into(),
            HostPattern::Exact(name) => name.clone(),
            HostPattern::Suffix(suffix) => format!("*.{suffix}"),
        }
    }

    fn port_text(&self) -> String {
        match self.ports {
            Ports::Any => "*".into(),
            Ports::One(one) => one.to_string(),
            Ports::Range(low, high) => format!("{low}-{high}"),
        }
    }

    fn family_name(&self) -> &'static str {
        match self.family {
            Family::Any => "any",
            Family::Ipv4 => "ipv4",
            Family::Ipv6 => "ipv6",
        }
    }
}

/// A refusal of the policy, with its reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refused {
    pub reason: String,
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.reason)
    }
}

impl std::error::Error for Refused {}

/// A connector that the policy checks before it runs. A test counts the
/// calls and proves that a refused destination never reaches one.
pub trait Connector {
    fn connect(&self, destination: &Destination) -> Result<(), String>;
}

/// Classify `destination`, and call `connector` only for an allowed action.
/// A refusal carries its reason and makes no call.
pub fn dial_with<C: Connector>(
    policy: &Policy,
    destination: &Destination,
    connector: &C,
) -> Result<Verdict, Refused> {
    let verdict = policy.classify(destination);
    if let Some(reason) = &verdict.reason {
        if verdict.action.refusal_is_an_error() {
            return Err(Refused {
                reason: reason.clone(),
            });
        }
    }
    connector.connect(destination).map_err(|e| Refused {
        reason: format!(
            "the rule {:?} sent {}:{} to the {} connector: {e}",
            verdict.rule.as_deref().unwrap_or("default"),
            destination.host,
            destination.port,
            verdict.action.as_str()
        ),
    })?;
    Ok(verdict)
}

/// The file that holds the egress policy of this process.
pub const POLICY_VARIABLE: &str = "PODNET_POLICY";
/// The node configuration whose `egress` object is the policy.
pub const NODE_VARIABLE: &str = "PODNET_NODE_CONFIG";

/// Install the policy that the environment names, before any dial. A file
/// that cannot be read or parsed is a named configuration error: the
/// process does not start with a policy that it did not read.
pub fn install_from_env() -> Result<(), String> {
    if let Some(path) = std::env::var_os(POLICY_VARIABLE) {
        let path = std::path::PathBuf::from(path);
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let policy = Policy::parse(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        install(policy);
        return Ok(());
    }
    if let Some(path) = std::env::var_os(NODE_VARIABLE) {
        let path = std::path::PathBuf::from(path);
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let document: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        let egress = document.get("egress").ok_or_else(|| {
            format!(
                "{}: the node configuration has no `egress` object",
                path.display()
            )
        })?;
        let policy =
            Policy::parse(&egress.to_string()).map_err(|e| format!("{}: {e}", path.display()))?;
        install(policy);
    }
    Ok(())
}

static INSTALLED: std::sync::OnceLock<Policy> = std::sync::OnceLock::new();

/// Install the policy of this process. A second call has no effect.
pub fn install(policy: Policy) {
    let _ = INSTALLED.set(policy);
}

/// The policy of this process, if one is installed.
pub fn installed() -> Option<&'static Policy> {
    INSTALLED.get()
}

/// [`permit`] as a [`pod_core::Failure`] of the connect stage. A refusal is
/// [`pod_core::Class::Unavailable`], the documented exit 69, and carries the
/// policy's reason. A dial helper that returns a failure uses this rather
/// than opening the socket and waiting for a timeout.
pub fn permit_failure(host: &str, port: u16) -> Result<(), pod_core::Failure> {
    match permit(host, port) {
        Ok(_) => Ok(()),
        Err(refused) => Err(pod_core::Failure::new(
            pod_core::Stage::Connect,
            pod_core::Class::Unavailable,
            refused.reason,
        )),
    }
}

/// [`permit`] as an `anyhow` error of [`crate::exit::Fault::Unavailable`],
/// the documented exit 69, with the policy's reason. A command path that
/// dials through `anyhow::Result` uses this.
pub fn permit_fault(host: &str, port: u16) -> anyhow::Result<()> {
    permit(host, port).map(|_| ()).map_err(|refused| {
        crate::exit::fault(
            crate::exit::Fault::Unavailable,
            anyhow::anyhow!(refused.reason),
        )
    })
}

/// [`permit`] as an I/O error of the permission kind, for a helper whose
/// signature is `io::Result`. The text is the policy's reason.
pub fn permit_io(host: &str, port: u16) -> std::io::Result<()> {
    permit(host, port).map(|_| ()).map_err(|refused| {
        std::io::Error::new(std::io::ErrorKind::PermissionDenied, refused.reason)
    })
}

/// Check `host:port` with the policy of the process, before a dial. With no
/// policy, each destination is allowed, as before #500.
pub fn permit(host: &str, port: u16) -> Result<Option<Verdict>, Refused> {
    let Some(policy) = installed() else {
        return Ok(None);
    };
    let destination = Destination::new(host, port);
    let verdict = policy.classify(&destination);
    if let Some(reason) = &verdict.reason {
        if verdict.action.refusal_is_an_error() {
            return Err(Refused {
                reason: reason.clone(),
            });
        }
    }
    Ok(Some(verdict))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Counting(AtomicUsize);

    impl Connector for Counting {
        fn connect(&self, _destination: &Destination) -> Result<(), String> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    fn policy(text: &str) -> Policy {
        Policy::parse(text).unwrap()
    }

    /// EGRESS-3 and RUN-1: a destination with no rule is refused with a
    /// reason, and the connector is not called.
    #[test]
    fn a_refused_destination_makes_no_attempt() {
        let policy = policy(r#"{"default":"refuse","rules":[]}"#);
        let counting = Counting(AtomicUsize::new(0));
        let err = dial_with(&policy, &Destination::new("example.net", 443), &counting).unwrap_err();
        assert!(err.reason.contains("no rule matches"), "{err}");
        assert_eq!(counting.0.load(Ordering::SeqCst), 0);
    }

    /// EGRESS-5: a later rule that an earlier rule covers is named.
    #[test]
    fn a_shadowed_rule_is_named() {
        let policy = policy(
            r#"{"default":"refuse","rules":[
                {"name":"all","host":"*","port":"*","action":"proxy"},
                {"name":"later","host":"api.example","port":"443","action":"direct"}
            ]}"#,
        );
        let shadows = policy.shadowed();
        assert_eq!(shadows.len(), 1, "{shadows:?}");
        assert_eq!(shadows[0].earlier, "all");
        assert_eq!(shadows[0].later, "later");
        assert!(shadows[0].to_string().contains("unreachable"));
    }

    /// EGRESS-2: the host, the port and the family each take part.
    #[test]
    fn the_classification_uses_host_port_and_family() {
        let policy = policy(
            r#"{"default":"refuse","rules":[
                {"name":"v4","host":"*.example","port":"443-8443","family":"ipv4","action":"direct"}
            ]}"#,
        );
        assert_eq!(
            policy
                .classify(&Destination::with_family("a.example", 443, Family::Ipv4))
                .action,
            Action::Direct
        );
        assert_eq!(
            policy
                .classify(&Destination::with_family("a.example", 443, Family::Ipv6))
                .action,
            Action::Refuse
        );
        assert_eq!(
            policy
                .classify(&Destination::with_family("a.example", 9443, Family::Ipv4))
                .action,
            Action::Refuse
        );
        // A name whose family is not known yet cannot match a rule that
        // names one.
        assert_eq!(
            policy.classify(&Destination::new("a.example", 443)).action,
            Action::Refuse
        );
    }
}
