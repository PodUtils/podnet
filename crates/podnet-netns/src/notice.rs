//! One line for each target whose connect fails (T-452).
//!
//! A program in a pod sees a failed connect as `ECONNREFUSED` or a reset:
//! a dead upstream, a name with no answer and a reject rule all look the
//! same to it. The line names the cause. At the default level it is printed
//! once for each target (a program can try one target many times); with
//! `-v`, once for each connect. `-q` silences it ([`crate::quiet`]).
//! A failure through an upstream that the warning at the start named as
//! dead gets no line at the default level: that warning names the cause.

use std::collections::HashSet;
use std::sync::Mutex;

/// The count of targets that get a line at the default level. After it,
/// one last line says that `-v` shows the rest.
const LIMIT: usize = 64;

/// The targets that got a line.
static SHOWN: Mutex<Option<HashSet<String>>> = Mutex::new(None);

/// The connect contexts of the upstreams that the warning at the start
/// named as dead ([`crate::upstream::Proxy::connect_context`]).
static DEAD: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Record that the warning at the start named the upstream of `context`.
pub(crate) fn upstream_dead(context: String) {
    DEAD.lock().unwrap_or_else(|e| e.into_inner()).push(context);
}

/// True when `line` fails at the connect to an upstream of [`DEAD`]. The
/// context ends with ": " and its cause, so 127.0.0.1:1 is not 127.0.0.1:10.
fn through_dead(line: &str) -> bool {
    let dead = DEAD.lock().unwrap_or_else(|e| e.into_inner());
    dead.iter()
        .any(|context| line.contains(&format!("{context}: ")))
}

/// Print the failure `e` of the upstream of a connect to `target`; `tail`
/// says what the program sees. At the default level, a direct dial that
/// the target refused gets no line: the program sees `ECONNREFUSED`, which
/// is the cause.
pub(crate) fn upstream_failed(target: &str, e: &anyhow::Error, tail: &str, verbose: bool) {
    if !verbose && refused_by_the_target(e) {
        return;
    }
    connect_failed(
        target,
        &format!("upstream for {target} failed: {e:#}{tail}"),
        verbose,
    );
}

/// True when `e` is a refusal of a direct dial: an I/O error
/// `ConnectionRefused` with no proxy in its chain.
fn refused_by_the_target(e: &anyhow::Error) -> bool {
    let refused = e
        .root_cause()
        .downcast_ref::<std::io::Error>()
        .is_some_and(|io| io.kind() == std::io::ErrorKind::ConnectionRefused);
    refused && !e.chain().any(|c| c.to_string().contains(" proxy "))
}

/// What to print for a failed connect to `key` at the default level.
#[derive(Debug, PartialEq, Eq)]
enum Show {
    /// The first failure of the target: the line.
    Line,
    /// The first target after [`LIMIT`]: the line that names `-v`.
    Last,
    /// Nothing: the target got its line.
    Nothing,
}

fn show(key: &str) -> Show {
    let mut shown = SHOWN.lock().unwrap_or_else(|e| e.into_inner());
    show_in(shown.get_or_insert_with(HashSet::new), key)
}

fn show_in(shown: &mut HashSet<String>, key: &str) -> Show {
    if shown.contains(key) {
        return Show::Nothing;
    }
    match shown.len() {
        n if n < LIMIT => {
            shown.insert(key.to_string());
            Show::Line
        }
        n if n == LIMIT => {
            // An empty key stands for the last line: no target is empty.
            shown.insert(String::new());
            Show::Last
        }
        _ => Show::Nothing,
    }
}

/// Print `line` (the failure of a connect to `target`): with `verbose`
/// always, else for the first failure of `target` only.
pub(crate) fn connect_failed(target: &str, line: &str, verbose: bool) {
    if verbose {
        eprintln!("{}: {line}", crate::cli::program_name());
        return;
    }
    if through_dead(line) {
        return;
    }
    match show(target) {
        Show::Line => crate::warn(&format!(
            "{line} (once for each target; -v shows each connect)"
        )),
        Show::Last => crate::warn(&format!(
            "more than {LIMIT} targets failed; -v shows each connect"
        )),
        Show::Nothing => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T-452: one line for the first failure of each target, none for the
    /// next failure of a target, and one last line at the limit.
    #[test]
    fn only_a_direct_refusal_is_the_cause_of_the_program() {
        let io = |kind| anyhow::Error::from(std::io::Error::from(kind));
        let refused = io(std::io::ErrorKind::ConnectionRefused);
        assert!(refused_by_the_target(
            &refused.context("direct TCP to a.invalid:80")
        ));
        let refused = io(std::io::ErrorKind::ConnectionRefused);
        assert!(!refused_by_the_target(
            &refused.context("connect to proxy 127.0.0.1:1")
        ));
        let other = io(std::io::ErrorKind::TimedOut);
        assert!(!refused_by_the_target(
            &other.context("direct TCP to a.invalid:80")
        ));
        assert!(!refused_by_the_target(&anyhow::anyhow!("no such name")));
    }

    #[test]
    fn each_target_gets_one_line_up_to_the_limit() {
        let mut shown = HashSet::new();
        let key = |n: usize| format!("t{n}.invalid:80");
        assert_eq!(show_in(&mut shown, &key(0)), Show::Line);
        assert_eq!(show_in(&mut shown, &key(0)), Show::Nothing);
        let mut lines = 1;
        let mut last = 0;
        for n in 1..LIMIT + 10 {
            match show_in(&mut shown, &key(n)) {
                Show::Line => lines += 1,
                Show::Last => last += 1,
                Show::Nothing => {}
            }
        }
        assert_eq!(lines, LIMIT, "one line for each target up to the limit");
        assert_eq!(last, 1, "one line that names -v");
    }
}
