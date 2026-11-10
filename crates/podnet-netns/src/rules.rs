//! The rules of `-r` (T-156): a matcher of a name or of prefixes, and an
//! action: a proxy, `direct`, or a refusal (`reject`, `block`). The WARP
//! rules (`podnet warp proxy`) use the same matcher and the same two words
//! for a refusal.

use std::net::{IpAddr, Ipv4Addr};

use anyhow::{bail, Context, Result};

use crate::upstream::Proxy;

#[derive(Clone, Debug)]
pub enum RuleMatch {
    Domain(String),
    /// One or more IPv4 or IPv6 prefixes (`cidr:NET`, or `cidr:@FILE`).
    Cidr(std::sync::Arc<[Net]>),
}

impl RuleMatch {
    /// `domain:NAME`, `domain:.NAME`, `cidr:ADDR/PREFIX` (IPv4 or IPv6) or
    /// `cidr:@FILE` (one prefix on each line; `#` starts a comment).
    pub fn parse(spec: &str) -> Result<Self> {
        let (kind, value) = spec
            .split_once(':')
            .with_context(|| format!("{spec:?}: give KIND:VALUE (domain, cidr)"))?;
        match kind {
            "domain" if !value.is_empty() && !value.contains(['/', ' ', ':']) => {
                Ok(RuleMatch::Domain(value.to_ascii_lowercase()))
            }
            "domain" => bail!("{spec:?}: give domain:NAME or domain:.NAME"),
            "cidr" => match value.strip_prefix('@') {
                Some(path) => Ok(RuleMatch::Cidr(
                    load_nets(std::path::Path::new(path))?.into(),
                )),
                None => Ok(RuleMatch::Cidr(vec![Net::parse(value)?].into())),
            },
            other => bail!("unknown rule kind {other:?} (domain, cidr)"),
        }
    }

    /// True when `host` (a name, or an address in text) or `ip` matches.
    pub fn matches(&self, host: &str, ip: Option<IpAddr>) -> bool {
        match self {
            RuleMatch::Domain(d) => ip.is_none() && domain_matches(d, host),
            RuleMatch::Cidr(nets) => ip.is_some_and(|ip| nets.iter().any(|n| n.contains(ip))),
        }
    }
}

/// An IPv4 or an IPv6 prefix (issue 142).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Net {
    pub addr: IpAddr,
    pub prefix: u8,
}

impl Net {
    /// `ADDR/PREFIX`, or `ADDR` (one address).
    pub fn parse(text: &str) -> Result<Self> {
        let text = text.trim();
        let (addr, prefix) = match text.split_once('/') {
            Some((a, p)) => (a, Some(p)),
            None => (text, None),
        };
        let addr: IpAddr = addr
            .trim_start_matches('[')
            .trim_end_matches(']')
            .parse()
            .with_context(|| format!("{text:?} is not ADDR/PREFIX"))?;
        let max = if addr.is_ipv4() { 32 } else { 128 };
        let prefix = match prefix {
            Some(p) => p
                .parse::<u8>()
                .ok()
                .filter(|p| *p <= max)
                .with_context(|| format!("{text:?}: the prefix is 0 to {max}"))?,
            None => max,
        };
        Ok(Net { addr, prefix })
    }

    /// True when `ip` is in the prefix. An IPv4-mapped IPv6 address is
    /// taken as its IPv4 address.
    pub fn contains(&self, ip: IpAddr) -> bool {
        match (self.addr, ip.to_canonical()) {
            (IpAddr::V4(net), IpAddr::V4(ip)) => in_cidr(ip, net, self.prefix),
            (IpAddr::V6(net), IpAddr::V6(ip)) => {
                if self.prefix == 0 {
                    return true;
                }
                let mask = u128::MAX << (128 - self.prefix as u32);
                (u128::from(ip) & mask) == (u128::from(net) & mask)
            }
            _ => false,
        }
    }
}

impl std::fmt::Display for Net {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.addr, self.prefix)
    }
}

/// The most prefixes of one file.
pub const MAX_NETS: usize = 1_000_000;

/// The prefixes of a file: one on each line; a blank line, and the text
/// after `#`, are ignored. A bad line is an error with its number.
pub fn load_nets(path: &std::path::Path) -> Result<Vec<Net>> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading the prefix file {}", path.display()))?;
    let mut nets = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if nets.len() == MAX_NETS {
            bail!("{}: more than {MAX_NETS} prefixes", path.display());
        }
        nets.push(Net::parse(line).with_context(|| format!("{}:{}", path.display(), n + 1))?);
    }
    if nets.is_empty() {
        bail!("{}: no prefix", path.display());
    }
    Ok(nets)
}

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
    pub matcher: RuleMatch,
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
            matcher: RuleMatch::parse(lhs)?,
            action: RuleAction::parse(rhs)?,
            spec: spec.to_string(),
        })
    }
}

pub fn in_cidr(ip: Ipv4Addr, net: Ipv4Addr, prefix: u8) -> bool {
    if prefix == 0 {
        return true;
    }
    let mask = u32::MAX << (32 - prefix as u32);
    (u32::from(ip) & mask) == (u32::from(net) & mask)
}

/// A domain rule: `NAME` matches the name only; `.NAME` matches the name
/// and each name under it (`a.NAME`), not `NAMEx` or `NAME.evil`.
pub fn domain_matches(rule: &str, host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    let rule = rule.to_ascii_lowercase();
    match rule.strip_prefix('.') {
        Some(base) => host == base || host.ends_with(&format!(".{base}")),
        None => host == rule.trim_end_matches('.'),
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
/// single hop or a refusal; the default is a chain.
pub fn select_chain<'a>(rules: &'a [Rule], default: &'a [Proxy], target: &str) -> Selected<'a> {
    let host = target.rsplit_once(':').map_or(target, |(h, _)| h);
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    let ip: Option<IpAddr> = bare.parse().ok();
    for rule in rules {
        if rule.matcher.matches(host, ip) {
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
}
