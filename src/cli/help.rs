//! The top help (the commands by purpose, the "Start here" block, the
//! examples of each command), the hints after a usage error and the last
//! error line.

use clap::CommandFactory as _;

use crate::cli::argv::{CLOUDFLARED_FLAGS, LOG_JSON};
use crate::cli::Cli;

/// The commands of the top help, by purpose (E1). Each subcommand is in
/// one group; a test checks it.
pub(crate) const HELP_GROUPS: &[(&str, &[&str])] = &[
    (
        "Publish a local service",
        &[
            "tunnel",
            "expose",
            "ssh-relay",
            "serve",
            "cover",
            "share",
            "send",
            "receive",
            "git",
            "demo",
            "qr",
        ],
    ),
    (
        "Reach the internet from a limited host",
        &[
            "agent", "warp", "pod", "net", "connect", "exit", "forward", "route", "relay", "link",
            "wg",
        ],
    ),
    (
        "Run in the background",
        &["ps", "logs", "wait", "stop", "autostart"],
    ),
    ("Work with no network", &["offline"]),
    (
        "Check this host and podnet",
        &[
            "doctor",
            "status",
            "host",
            "stat",
            "rules",
            "init",
            "harness",
            "ci",
            "forge",
            "ports",
            "components",
            "bench",
            "sidecar",
            "bug-report",
            "update",
            "man",
            "completions",
            "help",
        ],
    ),
];

/// The "Start here" block of the top help.
const START_HERE: &str = "\
Start here:
  podnet doctor                              test the path from this host to the edge
  podnet tunnel --url http://localhost:8080  a public URL for a local service
  podnet agent up                            a sandbox: test, start WARP if needed, expose ports
  podnet man                                 the full manual";

/// The first part of `text`: up to the end of the first sentence or the
/// first colon, with no issue reference.
pub(crate) fn first_sentence(text: &str) -> String {
    let end = [". ", ": "]
        .iter()
        .filter_map(|mark| text.find(mark))
        .min()
        .unwrap_or(text.len());
    let mut part = text[..end].to_string();
    while let Some(start) = part.find(" (issue ") {
        match part[start..].find(')') {
            Some(len) => part.replace_range(start..start + len + 1, ""),
            None => break,
        }
    }
    part
}

/// The commands of `cmd` by purpose, with the first sentence of each.
fn grouped_commands(cmd: &clap::Command) -> String {
    let width = cmd
        .get_subcommands()
        .map(|c| c.get_name().len())
        .max()
        .unwrap_or(0);
    let mut out = String::new();
    for (title, names) in HELP_GROUPS {
        out.push_str(&format!("{title}:\n"));
        for name in *names {
            let about = cmd
                .find_subcommand(name)
                .and_then(|c| c.get_about())
                .map(|a| a.to_string())
                .unwrap_or_else(|| {
                    "Print this message or the help of the given subcommand(s)".into()
                });
            out.push_str(&format!("  {name:width$}  {}\n", first_sentence(&about)));
        }
        out.push('\n');
    }
    out
}

/// The top command, with the commands by purpose and a "Start here" block
/// in its help.
pub(crate) fn top_command() -> clap::Command {
    let cmd = Cli::command();
    let template = format!(
        "{{about-with-newline}}\n{{usage-heading}} {{usage}}\n\n{START_HERE}\n\n{}Exit codes:\n{}Options:\n{{options}}{{after-help}}",
        grouped_commands(&cmd),
        podnet::exit::contract_table()
    );
    let root = Cli::command();
    with_examples(cmd.help_template(template), "podnet", &root)
}

/// The command path of an example: `podnet`, then each word that names a
/// subcommand (`podnet warp proxy -r` is `podnet warp proxy`).
pub(crate) fn example_path(cmd: &clap::Command, line: &str) -> String {
    let mut path = String::from("podnet");
    let mut at = cmd;
    for word in line.split_whitespace().skip(1) {
        match at.find_subcommand(word) {
            Some(sub) => {
                path.push(' ');
                path.push_str(word);
                at = sub;
            }
            None => break,
        }
    }
    path
}

/// The scenarios of each command with no subcommand (T-358 step 4):
/// `PATH | SCENARIO; ...`. `scripts/gate-scenarios.py --write` writes the
/// file from the workflows and the lane, and `--check` checks it.
pub(crate) const SCENARIOS: &str = include_str!("scenarios.txt");

/// The line in the help of a command with no scenario.
pub(crate) const EXPERIMENTAL: &str = "Experimental: no scenario of the release binary runs this command (a step of \
live.yml, of the job static of ci.yml, or a row of the release gate). Only the tests with stand-ins check it.";

/// Each command path of [`SCENARIOS`] (`warp scan`), with its scenarios.
pub(crate) fn scenarios() -> Vec<(&'static str, Vec<&'static str>)> {
    SCENARIOS
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .filter_map(|l| l.split_once('|'))
        .map(|(path, rest)| {
            let list = rest.split(';').map(str::trim).filter(|s| !s.is_empty());
            (path.trim(), list.collect())
        })
        .collect()
}

/// The command at `path` (`podnet warp scan`), with [`EXPERIMENTAL`] before
/// its help when no scenario runs it.
fn with_mark(cmd: clap::Command, path: &str) -> clap::Command {
    let Some(words) = path.strip_prefix("podnet ") else {
        return cmd;
    };
    if scenarios()
        .iter()
        .any(|(p, list)| *p == words && list.is_empty())
    {
        return cmd.before_help(EXPERIMENTAL);
    }
    cmd
}

/// E7: the examples of `podnet man examples` in the help of their command
/// (`podnet tunnel --help`), from the one table of the manual.
fn with_examples(cmd: clap::Command, path: &str, root: &clap::Command) -> clap::Command {
    let examples: Vec<&(&str, &str)> = podnet::manual::facts::EXAMPLES
        .iter()
        .filter(|(command, _)| path != "podnet" && example_path(root, command) == path)
        .collect();
    let names: Vec<String> = cmd
        .get_subcommands()
        .map(|c| c.get_name().to_string())
        .collect();
    let mut cmd = cmd;
    for name in names {
        let sub_path = format!("{path} {name}");
        cmd = cmd.mut_subcommand(&name, |sub| with_examples(sub, &sub_path, root));
    }
    let cmd = with_mark(cmd, path);
    if examples.is_empty() {
        return cmd;
    }
    let mut text = String::from("Examples:");
    for (command, meaning) in examples {
        text.push_str(&format!("\n  {command}\n      {meaning}"));
    }
    cmd.after_help(text)
}

/// The lines after a usage error (E8, E10): a word that is no command but
/// looks like a port, a URL or a path names the command for it; an unknown
/// command or flag names the version and `podnet update --check`.
pub(crate) fn usage_hints(err: &clap::Error) -> Vec<String> {
    use clap::error::{ContextKind, ContextValue, ErrorKind};
    let invalid = || match err
        .get(ContextKind::InvalidSubcommand)
        .or(err.get(ContextKind::InvalidArg))
    {
        Some(ContextValue::String(word)) => Some(word.clone()),
        _ => None,
    };
    let mut lines = Vec::new();
    match err.kind() {
        ErrorKind::InvalidSubcommand => {
            if let Some(hint) = invalid().as_deref().and_then(word_hint) {
                lines.push(hint);
            }
        }
        ErrorKind::UnknownArgument => {
            let word = invalid().unwrap_or_default();
            let name = word.split('=').next().unwrap_or("");
            if let Some((flag, advice)) = CLOUDFLARED_FLAGS.iter().find(|(f, _)| *f == name) {
                lines.push(format!(
                    "hint: {flag} is a flag of cloudflared, not of podnet; {advice}"
                ));
                return lines;
            }
            // T-060: the road of serve and git serve is --ingress; --egress
            // is the way out of each command.
            if name == "--via" {
                lines.push(
                    "hint: --via is gone: the road to the visitors of serve and git serve is \
                     --ingress tunnel|relay|none, and the way out of a command is --egress SPEC"
                        .to_string(),
                );
                return lines;
            }
            // Issue 616: a newer release can know only an unknown flag
            // name. A value, an extra positional or a flag of another
            // command gets no update line.
            if !unknown_flag(&Cli::command(), name) {
                return lines;
            }
        }
        _ => return lines,
    }
    lines.push(format!(
        "podnet {} does not know {}; a newer release can: podnet update --check",
        env!("CARGO_PKG_VERSION"),
        invalid().map_or_else(|| "it".to_string(), |w| format!("'{w}'"))
    ));
    lines
}

/// Whether `name` has the form of a flag (`--word` or `-w`) and no command
/// of `cmd` knows it.
pub(crate) fn unknown_flag(cmd: &clap::Command, name: &str) -> bool {
    fn knows(cmd: &clap::Command, long: Option<&str>, short: Option<char>) -> bool {
        cmd.get_arguments().any(|a| {
            long.is_some_and(|l| {
                a.get_long() == Some(l) || a.get_all_aliases().is_some_and(|n| n.contains(&l))
            }) || short.is_some_and(|c| {
                a.get_short() == Some(c)
                    || a.get_all_short_aliases().is_some_and(|n| n.contains(&c))
            })
        }) || cmd.get_subcommands().any(|sub| knows(sub, long, short))
    }
    let (long, short) = match name.strip_prefix("--") {
        Some(long) => (Some(long), None),
        None => match name.strip_prefix('-') {
            Some(rest) if rest.chars().count() == 1 => (None, rest.chars().next()),
            _ => return false,
        },
    };
    let first = long.and_then(|l| l.chars().next()).or(short);
    if !first.is_some_and(|c| c.is_ascii_alphabetic()) {
        return false;
    }
    !knows(cmd, long, short)
}

/// The command for a word that is no command of podnet.
pub(crate) fn word_hint(word: &str) -> Option<String> {
    if !word.is_empty() && word.len() <= 5 && word.bytes().all(|b| b.is_ascii_digit()) {
        return Some(format!(
            "hint: for a public URL of the local port {word}: podnet tunnel --url http://localhost:{word}"
        ));
    }
    let lower = word.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return Some(format!(
            "hint: for a public URL of the local service {word}: podnet tunnel --url {word}"
        ));
    }
    if lower.starts_with("wss://") || lower.starts_with("ws://") {
        return Some(format!(
            "hint: for a local socket to the WebSocket forward {word}: podnet connect --local 127.0.0.1:2222 {word}"
        ));
    }
    let path = std::path::Path::new(word);
    if word.starts_with('/') || word.starts_with("./") || word.starts_with("~/") || path.exists() {
        return Some(if path.is_file() {
            format!("hint: to send the file {word}: podnet share --mode download --file {word}")
        } else {
            format!("hint: to serve the directory {word}: podnet serve --dir {word}")
        });
    }
    None
}

/// Whether `argv` asks for `--log-format json` (before a parse).
pub(crate) fn json_wanted(argv: &[String]) -> bool {
    argv.windows(2)
        .any(|w| w[0] == "--log-format" && w[1] == "json")
        || argv.iter().any(|a| a == "--log-format=json")
}

/// The last error line as one JSON object, in the shape of a log line of
/// `--log-format json`.
pub(crate) fn error_json(text: &str, code: i32) -> String {
    let message = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.strip_prefix("podnet: ").unwrap_or(l))
        .collect::<Vec<_>>()
        .join("; ");
    serde_json::json!({
        "level": "ERROR",
        "fields": { "message": message.trim_end(), "exit_code": code },
        "target": "podnet",
    })
    .to_string()
}

/// Print the last error line: text, or JSON with --log-format json.
pub(crate) fn print_error(line: &str, code: i32) {
    if LOG_JSON.load(std::sync::atomic::Ordering::Relaxed) {
        eprintln!("{}", error_json(line, code));
    } else {
        eprintln!("{line}");
    }
}
