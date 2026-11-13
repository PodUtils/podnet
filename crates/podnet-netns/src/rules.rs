//! The rules of `-r` (T-156): a matcher of a name or of prefixes, and an
//! action: a proxy, `direct`, or a refusal (`reject`, `block`). The WARP
//! rules (`podnet warp proxy`) use the same matcher and the same two words
//! for a refusal.

use std::net::IpAddr;

use anyhow::{Context, Result};

use crate::upstream::Proxy;

pub mod shadow;

// One reader of the matcher for the doors, the pods and WARP: the text of a
// rule is read by `pod_warp::rules::matcher` (T-328; the freeze of this
// file ended with the take of podcrates v0.38.0).
pub use pod_warp::rules::matcher::{
    domain_matches, in_cidr, load_nets, Net, RuleMatch, TcpHost, MAX_NETS,
};

/// What a rule does with a destination that it matches.
#[derive(Clone, Debug)]
pub enum RuleAction {
    /// Send the flow through this proxy (`direct`: no proxy).
    Proxy(Proxy),
    /// Refuse the flow (`reject` or `block`, the words of the WARP rules).
    Reject,
}

impl RuleAction {
    /// `reject` or `block` (any case), else a proxy spec or `direct`.
    pub fn parse(text: &str) -> Result<Self> {
        let text = text.trim();
        match text.to_ascii_lowercase().as_str() {
            "reject" | "block" => Ok(RuleAction::Reject),
            _ => Ok(RuleAction::Proxy(Proxy::parse(text)?)),
        }
    }
}

/// One rule of `-r`: `KIND:VALUE=ACTION`.
#[derive(Clone, Debug)]
pub struct Rule {
    /// `None`: each target (the rule of `--default-action reject`).
    pub matcher: Option<RuleMatch>,
    pub action: RuleAction,
    /// The text of the rule, for the log and the flow record.
    pub spec: String,
}

impl Rule {
    /// `KIND:VALUE=ACTION`: ACTION is a proxy, `direct`, `reject` or
    /// `block`.
    pub fn parse(spec: &str) -> Result<Self> {
        let (lhs, rhs) = spec
            .split_once('=')
            .context("--rule needs KIND:VALUE=PROXY")?;
        Ok(Rule {
            matcher: Some(RuleMatch::parse(lhs)?),
            action: RuleAction::parse(rhs)?,
            spec: spec.to_string(),
        })
    }

    /// The last rule of `--default-action reject`: it refuses each target.
    /// Its name in the log and in the record is `default`.
    pub fn default_reject() -> Self {
        Rule {
            matcher: None,
            action: RuleAction::Reject,
            spec: "default".into(),
        }
    }
}

/// The decision of the rules for one target.
#[derive(Clone, Copy, Debug)]
pub enum Choice<'a> {
    /// Dial through this chain (empty: no proxy).
    Chain(&'a [Proxy]),
    /// Refuse the flow.
    Reject,
}

/// The decision for one target, and the rule that made it (`None`: the
/// default).
#[derive(Clone, Copy, Debug)]
pub struct Selected<'a> {
    pub choice: Choice<'a>,
    pub rule: Option<&'a Rule>,
}

impl<'a> Selected<'a> {
    /// The chain, or `None` for a refusal.
    pub fn chain(&self) -> Option<&'a [Proxy]> {
        match self.choice {
            Choice::Chain(chain) => Some(chain),
            Choice::Reject => None,
        }
    }

    /// The text of the rule, or `default`.
    pub fn rule_name(&self) -> &'a str {
        self.rule.map_or("default", |r| r.spec.as_str())
    }
}

/// First matching rule wins; otherwise the default `-x` chain. A rule is a
/// single hop or a refusal; the default is a chain. `target` is `HOST:PORT`
/// of a TCP connection: the rules of a port (`port:`, `tcp:`) match its port.
pub fn select_chain<'a>(rules: &'a [Rule], default: &'a [Proxy], target: &str) -> Selected<'a> {
    select(rules, default, target, true)
}

/// [`select_chain`] for a datagram flow: no rule of a port matches it.
pub fn select_chain_udp<'a>(rules: &'a [Rule], default: &'a [Proxy], target: &str) -> Selected<'a> {
    select(rules, default, target, false)
}

fn select<'a>(rules: &'a [Rule], default: &'a [Proxy], target: &str, tcp: bool) -> Selected<'a> {
    let (host, port) = match target.rsplit_once(':') {
        Some((h, p)) => (h, p.parse::<u16>().ok()),
        None => (target, None),
    };
    let port = port.filter(|_| tcp);
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    let ip: Option<IpAddr> = bare.parse().ok();
    for rule in rules {
        if rule
            .matcher
            .as_ref()
            .is_none_or(|m| m.matches(host, ip, port))
        {
            let choice = match &rule.action {
                RuleAction::Proxy(proxy) => Choice::Chain(std::slice::from_ref(proxy)),
                RuleAction::Reject => Choice::Reject,
            };
            return Selected {
                choice,
                rule: Some(rule),
            };
        }
    }
    Selected {
        choice: Choice::Chain(default),
        rule: None,
    }
}

/// The count of the flows that a rule refused.
static REFUSALS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Count a refusal of `target` by `rule`. With `verbose`, print one line
/// with the count.
pub(crate) fn note_refusal(what: &str, target: &str, rule: &str, verbose: bool) {
    let count = REFUSALS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    if verbose {
        eprintln!(
            "{}: {what} {target} refused by the rule {rule} ({count} refused)",
            crate::cli::program_name()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // T-156: `reject` and `block` refuse; the decision names the rule.
    #[test]
    fn a_reject_rule_refuses_and_names_the_rule() {
        let default = vec![Proxy::parse("socks5://127.0.0.1:1").unwrap()];
        let rules: Vec<Rule> = ["domain:.ads.example=reject", "cidr:10.0.0.0/8=BLOCK"]
            .iter()
            .map(|spec| Rule::parse(spec).unwrap())
            .collect();
        for target in ["x.ads.example:443", "10.1.2.3:80"] {
            let picked = select_chain(&rules, &default, target);
            assert!(matches!(picked.choice, Choice::Reject), "{target}");
            assert!(picked.chain().is_none());
        }
        assert_eq!(
            select_chain(&rules, &default, "10.1.2.3:80").rule_name(),
            "cidr:10.0.0.0/8=BLOCK"
        );
        let other = select_chain(&rules, &default, "example.com:443");
        assert_eq!(other.rule_name(), "default");
        assert_eq!(other.chain().unwrap()[0].port, 1);
        assert!(Rule::parse("domain:a=rejected").is_err());
    }

    // T-328: a rule of a port or of a host and a port; a datagram flow has
    // no port for them.
    #[test]
    fn rules_port_rules_match_the_port_of_the_target() {
        let default = vec![Proxy::parse("socks5://127.0.0.1:1").unwrap()];
        let rules: Vec<Rule> = [
            "tcp:127.0.0.1:5432=socks5://127.0.0.1:2",
            "tcp:db.example:5432=socks5://127.0.0.1:3",
            "port:8000-8010=reject",
            "port:22=direct",
        ]
        .iter()
        .map(|spec| Rule::parse(spec).unwrap())
        .collect();
        let hop = |target: &str| {
            select_chain(&rules, &default, target)
                .chain()
                .map(|c| c.first().map(|p| p.port))
        };
        assert_eq!(hop("127.0.0.1:5432"), Some(Some(2)));
        assert_eq!(hop("127.0.0.1:5433"), Some(Some(1)));
        assert_eq!(hop("db.example:5432"), Some(Some(3)));
        assert_eq!(hop("other.example:5432"), Some(Some(1)));
        assert_eq!(hop("[::1]:8005"), None);
        assert_eq!(hop("10.0.0.1:22").unwrap().unwrap(), 0);
        let udp = select_chain_udp(&rules, &default, "127.0.0.1:5432");
        assert_eq!(udp.rule_name(), "default");
        assert!(select_chain_udp(&rules, &default, "10.0.0.1:8005")
            .chain()
            .is_some());
        for bad in ["port:0=direct", "port:9-1=direct", "tcp:127.0.0.1=direct"] {
            assert!(Rule::parse(bad).is_err(), "{bad}");
        }
    }
}
