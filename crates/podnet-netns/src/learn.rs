//! `--learn FILE` and `--fail-on-violation FILE` (T-158): a CI job records
//! the destinations of a workload, then asserts that a later run touches
//! only those destinations.
//!
//! The file has one destination on each line, `PROTO TARGET`; a blank line
//! and a line that starts with `#` are ignored:
//!
//! ```text
//! # podnet pod --learn: the destinations of one run
//! tcp 93.184.216.34:80
//! udp example.com:53
//! ```
//!
//! * Learn: the pod records each destination. A flow that a rule refuses
//!   goes through the default `-x` chain, so the run sees each destination.
//!   At the exit of the program, the pod writes the file.
//! * Enforce: a destination that is not in the file is refused, as a
//!   `reject` rule does (the rule name is `not-learned`). A destination of
//!   the file that a rule refuses is also a violation. At the exit of the
//!   program, the pod writes each violation on stderr. When the program
//!   exits 0 and a violation occurred, the exit status is
//!   [`EXIT_VIOLATION`].

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, OnceLock};

use anyhow::{bail, Context, Result};

use crate::rules::{select_chain, select_chain_udp, Choice, Rule, RuleAction, Selected};
use crate::upstream::Proxy;

/// The exit status of a run with a violation of the learned set, when the
/// program exits 0.
pub(crate) const EXIT_VIOLATION: i32 = 4;

/// One destination: the protocol (`tcp` or `udp`) and the target.
type Destination = (String, String);

enum Mode {
    Learn {
        path: PathBuf,
        seen: BTreeSet<Destination>,
    },
    Enforce {
        learned: BTreeSet<Destination>,
        violations: BTreeSet<String>,
    },
}

/// The mode of this pod, when `--learn` or `--fail-on-violation` is given.
static MODE: OnceLock<Mutex<Mode>> = OnceLock::new();
/// The result of [`finish`]: true when a violation occurred.
static FINISHED: OnceLock<bool> = OnceLock::new();

/// The rule of a refusal of a destination that is not in the learned set.
static NOT_LEARNED: LazyLock<Rule> = LazyLock::new(|| Rule {
    matcher: None,
    action: RuleAction::Reject,
    spec: "not-learned".into(),
});

/// Start learning: [`finish`] writes the destinations to `path`.
pub(crate) fn learn(path: &Path) -> Result<()> {
    // Fail now, not at the end of the run, when the file cannot be written.
    std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)
        .with_context(|| format!("open the learned set {}", path.display()))?;
    set(Mode::Learn {
        path: path.to_path_buf(),
        seen: BTreeSet::new(),
    });
    Ok(())
}

/// Start the enforcement of the learned set in `path`.
pub(crate) fn enforce(path: &Path) -> Result<()> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("read the learned set {}", path.display()))?;
    set(Mode::Enforce {
        learned: parse(&text).with_context(|| path.display().to_string())?,
        violations: BTreeSet::new(),
    });
    Ok(())
}

fn set(mode: Mode) {
    let _ = MODE.set(Mutex::new(mode));
}

/// The destinations of the text of a learned set.
fn parse(text: &str) -> Result<BTreeSet<Destination>> {
    let mut set = BTreeSet::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        match line.split_once(' ') {
            Some((proto @ ("tcp" | "udp"), target)) if !target.trim().is_empty() => {
                set.insert((proto.to_string(), target.trim().to_string()));
            }
            _ => bail!("line {}: give PROTO TARGET (tcp or udp)", n + 1),
        }
    }
    Ok(set)
}

/// The text of a learned set.
fn render(seen: &BTreeSet<Destination>) -> String {
    let mut text = String::from("# podnet pod --learn: the destinations of one run\n");
    for (proto, target) in seen {
        text.push_str(&format!("{proto} {target}\n"));
    }
    text
}

/// [`select_chain`], then the learned set: learn records the destination,
/// enforce refuses a destination that is not learned. `proto` is `tcp` or
/// `udp`.
pub(crate) fn select<'a>(
    rules: &'a [Rule],
    default: &'a [Proxy],
    target: &str,
    proto: &str,
) -> Selected<'a> {
    // A datagram flow has no port for a rule of a port (T-328).
    let picked = match proto {
        "udp" => select_chain_udp(rules, default, target),
        _ => select_chain(rules, default, target),
    };
    let Some(mode) = MODE.get() else {
        return picked;
    };
    let mut mode = mode.lock().unwrap_or_else(|e| e.into_inner());
    let destination = (proto.to_string(), target.to_string());
    match &mut *mode {
        Mode::Learn { seen, .. } => {
            seen.insert(destination);
            match picked.choice {
                Choice::Reject if !default.is_empty() => Selected {
                    choice: Choice::Chain(default),
                    rule: None,
                },
                _ => picked,
            }
        }
        Mode::Enforce {
            learned,
            violations,
        } => {
            if !learned.contains(&destination) {
                violations.insert(format!("{proto} {target}: not in the learned set"));
                return Selected {
                    choice: Choice::Reject,
                    rule: Some(&NOT_LEARNED),
                };
            }
            if picked.chain().is_none() {
                violations.insert(format!(
                    "{proto} {target}: learned, refused by the rule {}",
                    picked.rule_name()
                ));
            }
            picked
        }
    }
}

/// At the exit of the program: write the learned set, or print each
/// violation. Return the exit status: `code`, or [`EXIT_VIOLATION`] when
/// `code` is 0 and a violation occurred. Only the first call writes; a
/// later call gives the same status.
pub(crate) fn finish(code: i32) -> i32 {
    let violated = *FINISHED.get_or_init(|| {
        let Some(mode) = MODE.get() else {
            return false;
        };
        let mode = mode.lock().unwrap_or_else(|e| e.into_inner());
        let name = crate::cli::program_name();
        match &*mode {
            Mode::Learn { path, seen } => {
                if let Err(e) = std::fs::write(path, render(seen)) {
                    eprintln!("{name}: write the learned set {}: {e}", path.display());
                }
                false
            }
            Mode::Enforce { violations, .. } => {
                for violation in violations {
                    eprintln!("{name}: violation: {violation}");
                }
                !violations.is_empty()
            }
        }
    });
    match (violated, code) {
        (true, 0) => EXIT_VIOLATION,
        _ => code,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // T-158: the file of a learned set reads back to the same set.
    #[test]
    fn a_learned_set_reads_back() {
        let mut seen = BTreeSet::new();
        seen.insert(("tcp".to_string(), "93.184.216.34:80".to_string()));
        seen.insert(("udp".to_string(), "[2001:db8::1]:53".to_string()));
        assert_eq!(parse(&render(&seen)).unwrap(), seen);
        assert!(parse("tcp 10.0.0.1:80\nicmp 10.0.0.1\n").is_err());
        assert!(parse("tcp\n").is_err());
        assert!(parse("\n# only a comment\n").unwrap().is_empty());
    }
}
