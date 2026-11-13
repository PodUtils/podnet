//! The lint of shadowed rules (T-242). The rules are first match wins. A
//! rule is shadowed when an earlier rule of the same kind matches each
//! target that the rule matches, and the two actions differ. The lint
//! changes no order of evaluation.

use std::collections::HashMap;

use anyhow::{bail, Result};

use super::{Net, RuleMatch, TcpHost};

/// The most pairs of prefixes that the lint compares for two rules. Two
/// big prefix files over this size are not compared.
const MAX_PAIRS: usize = 4_000_000;

/// One rule, as the lint reads it.
pub struct LintItem<'a> {
    pub matcher: &'a RuleMatch,
    /// The action in a normal form: `reject` for `reject` and `block`.
    pub action: String,
    /// Where the rule is: `-r 2`, `line 7`.
    pub place: String,
    /// The text of the rule.
    pub spec: String,
}

/// The normal form of the text of an action.
pub fn action_key(text: &str) -> String {
    let text = text.trim().to_ascii_lowercase();
    if text == "block" {
        "reject".into()
    } else {
        text
    }
}

/// True when the domain rule `earlier` matches each name that `later`
/// matches. `NAME` covers only `NAME`; `.NAME` covers `NAME` and each name
/// under it.
fn domain_covers(earlier: &str, later: &str) -> bool {
    let earlier = earlier.trim_end_matches('.');
    let later = later.trim_end_matches('.');
    match earlier.strip_prefix('.') {
        Some(base) => {
            let later = later.strip_prefix('.').unwrap_or(later);
            later == base || later.ends_with(&format!(".{base}"))
        }
        None => earlier == later,
    }
}

fn tcp_host_equal(a: &TcpHost, b: &TcpHost) -> bool {
    match (a, b) {
        (TcpHost::Name(a), TcpHost::Name(b)) => a == b,
        (TcpHost::Ip(a), TcpHost::Ip(b)) => a.to_canonical() == b.to_canonical(),
        _ => false,
    }
}

fn net_covers(earlier: &Net, later: &Net) -> bool {
    earlier.addr.is_ipv4() == later.addr.is_ipv4()
        && earlier.prefix <= later.prefix
        && earlier.contains(later.addr)
}

/// True when `earlier` matches each target that `later` matches. A rule of a
/// name and a rule of prefixes never cover each other. A rule of a name
/// or of prefixes matches each port; a rule of a port matches each host; a
/// `tcp:` rule matches one host and one port (T-328).
pub fn covers(earlier: &RuleMatch, later: &RuleMatch) -> bool {
    match (earlier, later) {
        (RuleMatch::Port(lo, hi), RuleMatch::Port(l, h)) => lo <= l && h <= hi,
        (RuleMatch::Port(lo, hi), RuleMatch::Tcp(_, p)) => (lo..=hi).contains(&p),
        (RuleMatch::Domain(a), RuleMatch::Tcp(TcpHost::Name(b), _)) => domain_covers(a, b),
        (RuleMatch::Cidr(a), RuleMatch::Tcp(TcpHost::Ip(ip), _)) => {
            a.iter().any(|n| n.contains(*ip))
        }
        (RuleMatch::Tcp(a, p), RuleMatch::Tcp(b, q)) => p == q && tcp_host_equal(a, b),
        (RuleMatch::Domain(a), RuleMatch::Domain(b)) => domain_covers(a, b),
        (RuleMatch::Cidr(a), RuleMatch::Cidr(b)) => {
            a.len().saturating_mul(b.len()) <= MAX_PAIRS
                && b.iter().all(|l| a.iter().any(|e| net_covers(e, l)))
        }
        _ => false,
    }
}

/// The pairs `(later, earlier)` where the later rule is shadowed and its
/// action differs. `earlier` is the first rule that covers the later rule.
pub fn shadowed(items: &[LintItem]) -> Vec<(usize, usize)> {
    let index = DomainIndex::new(items);
    let mut nets = NetIndex::default();
    let mut found = Vec::new();
    for (j, later) in items.iter().enumerate() {
        let first = match later.matcher {
            RuleMatch::Domain(d) => index.first(d, j),
            RuleMatch::Cidr(list) => {
                let hit = nets.first(items, later.matcher, list);
                nets.add(j, list);
                hit
            }
            // A rule of a port or of a host and a port: the scan of the
            // earlier rules (a few rules in a list).
            _ => (0..j).find(|&i| covers(items[i].matcher, later.matcher)),
        };
        if let Some(i) = first.filter(|&i| items[i].action != later.action) {
            found.push((j, i));
        }
    }
    found
}

/// The key of the prefix of `net` with `prefix` bits: the family, the
/// length and the masked address.
fn net_key(net: &Net, prefix: u8) -> (bool, u8, u128) {
    let (v4, bits, width) = match net.addr {
        std::net::IpAddr::V4(a) => (true, u128::from(u32::from(a)), 32u32),
        std::net::IpAddr::V6(a) => (false, u128::from(a), 128u32),
    };
    let drop = width - u32::from(prefix);
    let masked = if drop >= 128 {
        0
    } else {
        (bits >> drop) << drop
    };
    (v4, prefix, masked)
}

/// The earlier prefix rules. A rule of one prefix goes in a map: a later
/// prefix rule of one prefix looks up each shorter prefix. A rule of many
/// prefixes goes in a list that is compared one by one.
#[derive(Default)]
struct NetIndex {
    single: HashMap<(bool, u8, u128), usize>,
    many: Vec<usize>,
    /// Each earlier prefix rule, for a later rule of many prefixes.
    all: Vec<usize>,
}

impl NetIndex {
    fn add(&mut self, n: usize, list: &[Net]) {
        self.all.push(n);
        match list {
            [net] => {
                self.single.entry(net_key(net, net.prefix)).or_insert(n);
            }
            _ => self.many.push(n),
        }
    }

    fn first(&self, items: &[LintItem], later: &RuleMatch, list: &[Net]) -> Option<usize> {
        let compare = |ids: &[usize]| {
            ids.iter()
                .copied()
                .find(|&i| covers(items[i].matcher, later))
        };
        let [net] = list else {
            return compare(&self.all);
        };
        let max = if net.addr.is_ipv4() { 32 } else { 128 };
        let hit = (0..=net.prefix.min(max))
            .filter_map(|p| self.single.get(&net_key(net, p)).copied())
            .min();
        match (hit, compare(&self.many)) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }
}

/// An index of the domain rules by name. It finds the first earlier rule
/// that covers a domain rule in a time of the number of labels of the
/// name, not of the number of rules.
struct DomainIndex<'a> {
    exact: HashMap<&'a str, usize>,
    dotted: HashMap<&'a str, usize>,
}

impl<'a> DomainIndex<'a> {
    fn new(items: &'a [LintItem]) -> Self {
        let mut index = DomainIndex {
            exact: HashMap::new(),
            dotted: HashMap::new(),
        };
        for (n, item) in items.iter().enumerate() {
            if let RuleMatch::Domain(d) = item.matcher {
                let d = d.trim_end_matches('.');
                match d.strip_prefix('.') {
                    Some(base) => index.dotted.entry(base).or_insert(n),
                    None => index.exact.entry(d).or_insert(n),
                };
            }
        }
        index
    }

    /// The first rule before `before` that covers the domain rule `later`.
    fn first(&self, later: &str, before: usize) -> Option<usize> {
        let later = later.trim_end_matches('.');
        let (name, is_dotted) = match later.strip_prefix('.') {
            Some(name) => (name, true),
            None => (later, false),
        };
        let mut best: Option<usize> = None;
        let mut take = |hit: Option<&usize>| {
            if let Some(&n) = hit.filter(|&&n| n < before) {
                best = Some(best.map_or(n, |b| b.min(n)));
            }
        };
        if !is_dotted {
            take(self.exact.get(name));
        }
        let mut rest = name;
        loop {
            take(self.dotted.get(rest));
            match rest.split_once('.') {
                Some((_, tail)) => rest = tail,
                None => break,
            }
        }
        best
    }
}

/// The lint: one warning text for each shadowed rule. A shadowed `reject`
/// is an error, unless `allow` is set.
pub fn check(items: &[LintItem], allow: bool) -> Result<Vec<String>> {
    let mut warnings = Vec::new();
    let mut refusals = Vec::new();
    for (j, i) in shadowed(items) {
        let (later, earlier) = (&items[j], &items[i]);
        let text = format!(
            "the rule {} ({}) is shadowed by the earlier rule {} ({}): the earlier rule matches first",
            later.place, later.spec, earlier.place, earlier.spec
        );
        if later.action == "reject" && !allow {
            refusals.push(text);
        } else {
            warnings.push(text);
        }
    }
    if !refusals.is_empty() {
        bail!(
            "{}\nMove the narrow rule above the broad rule, or give --allow-shadowed-rules.",
            refusals.join("\n")
        );
    }
    Ok(warnings)
}

#[cfg(test)]
mod tests;
