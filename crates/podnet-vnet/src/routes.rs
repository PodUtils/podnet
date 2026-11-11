//! The rules of the router (T-098): each rule sends a target to a named
//! way (a network or a group) or refuses it. The first rule that matches
//! wins; a target that no rule matches takes the default way of the door.
//! The rules come from a file that is read again when it changes, with no
//! restart. A rule is `domain:NAME=WAY`, `domain:.NAME=WAY`,
//! `cidr:NET=WAY` or `cidr:@FILE=WAY`, with WAY a name of the router or
//! `reject` (or `block`); `#` starts a comment.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use anyhow::{bail, Context, Result};
use podnet_core::network::{
    BoxFuture, Connected, Datagrams, Dial, Failure, Rule, Rules, Target, Way,
};
use tokio::time::Instant;

use crate::dialer::PodRule;

/// Parse the rules of `text` over the named `ways`, with the way of a
/// target that no rule matches (`None`: refuse it).
pub fn parse(text: &str, ways: &HashMap<String, Way>, default: Option<Way>) -> Result<Rules> {
    parse_with(text, ways, default, false)
}

/// As [`parse`]. A rule that an earlier rule shadows gives a warning. A
/// shadowed `reject` is an error, unless `allow_shadowed` is set (T-242).
pub fn parse_with(
    text: &str,
    ways: &HashMap<String, Way>,
    default: Option<Way>,
    allow_shadowed: bool,
) -> Result<Rules> {
    let mut rules = Vec::new();
    let mut lint = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let (matcher, way) = line
            .rsplit_once('=')
            .with_context(|| format!("line {}: give MATCH=WAY", n + 1))?;
        let matcher = PodRule::parse(matcher.trim()).with_context(|| format!("line {}", n + 1))?;
        let way_text = way;
        let way = match way.trim() {
            "reject" | "block" => None,
            name => match ways.get(name) {
                Some(way) => Some(way.clone()),
                None => bail!(
                    "line {}: the way {name:?} is not a network or a group of the router",
                    n + 1
                ),
            },
        };
        lint.push((
            n + 1,
            matcher.matcher().clone(),
            podnet_netns::rules::shadow::action_key(way_text),
            line.to_string(),
        ));
        rules.push(Rule {
            matcher: Arc::new(matcher),
            way,
        });
    }
    let items: Vec<_> = lint
        .iter()
        .map(
            |(n, matcher, action, spec)| podnet_netns::rules::shadow::LintItem {
                matcher,
                action: action.clone(),
                place: format!("line {n}"),
                spec: spec.clone(),
            },
        )
        .collect();
    for warning in podnet_netns::rules::shadow::check(&items, allow_shadowed)? {
        tracing::warn!("{warning}");
    }
    Ok(Rules::new(rules, default)?)
}

/// A rules file that is read again when its time of change moves. A file
/// that cannot be read or parsed keeps the rules before it, with a warning.
pub struct RulesFile {
    path: PathBuf,
    ways: HashMap<String, Way>,
    default: Option<Way>,
    allow_shadowed: bool,
    current: Mutex<(Option<SystemTime>, Arc<Rules>)>,
}

impl RulesFile {
    /// Read `path` now: an error here is the error of the start.
    pub fn open(
        path: &Path,
        ways: HashMap<String, Way>,
        default: Option<Way>,
    ) -> Result<RulesFile> {
        RulesFile::open_with(path, ways, default, false)
    }

    /// As [`RulesFile::open`]. With `allow_shadowed`, a shadowed `reject`
    /// rule is a warning, not an error (T-242).
    pub fn open_with(
        path: &Path,
        ways: HashMap<String, Way>,
        default: Option<Way>,
        allow_shadowed: bool,
    ) -> Result<RulesFile> {
        let (stamp, text) = read(path)?;
        let rules = parse_with(&text, &ways, default.clone(), allow_shadowed)
            .with_context(|| path.display().to_string())?;
        Ok(RulesFile {
            path: path.to_path_buf(),
            ways,
            default,
            allow_shadowed,
            current: Mutex::new((stamp, Arc::new(rules))),
        })
    }

    /// The rules now: the file is read again when it changed.
    pub fn rules(&self) -> Arc<Rules> {
        let mut current = self.current.lock().unwrap_or_else(|e| e.into_inner());
        let stamp = std::fs::metadata(&self.path)
            .and_then(|m| m.modified())
            .ok();
        if stamp.is_some() && stamp != current.0 {
            match read(&self.path).and_then(|(stamp, text)| {
                Ok((
                    stamp,
                    parse_with(&text, &self.ways, self.default.clone(), self.allow_shadowed)?,
                ))
            }) {
                Ok((stamp, rules)) => {
                    tracing::info!(path = %self.path.display(), "the rules of the router changed");
                    *current = (stamp, Arc::new(rules));
                }
                Err(e) => {
                    tracing::warn!(path = %self.path.display(), "the rules keep their last form: {e:#}");
                    current.0 = stamp;
                }
            }
        }
        current.1.clone()
    }
}

fn read(path: &Path) -> Result<(Option<SystemTime>, String)> {
    let text = std::fs::read_to_string(path).with_context(|| path.display().to_string())?;
    let stamp = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    Ok((stamp, text))
}

impl Dial for RulesFile {
    fn label(&self) -> String {
        format!("rules of {}", self.path.display())
    }

    fn dial<'a>(
        &'a self,
        target: &'a Target,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(async move { self.rules().dial(target, deadline).await })
    }

    fn dial_http<'a>(
        &'a self,
        target: &'a Target,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(async move { self.rules().dial_http(target, deadline).await })
    }

    fn names_inside(&self) -> bool {
        true
    }

    fn datagrams<'a>(&'a self) -> BoxFuture<'a, Result<Arc<dyn Datagrams>, Failure>> {
        Box::pin(async move { self.rules().datagrams().await })
    }
}

// The tests are with the fake SOCKS5 upstreams of `dialer::tests`.
