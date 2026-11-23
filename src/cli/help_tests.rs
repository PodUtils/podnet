use crate::cli::help::{
    example_path, first_sentence, scenarios, top_command, unknown_flag, usage_hints, word_hint,
    EXPERIMENTAL, HELP_GROUPS,
};
use crate::cli::Cli;
use clap::{CommandFactory as _, Parser as _};

/// Issues 253 and 254: a lint of each help screen. No issue numbers of
/// this project, no `[env: X=]`, no `<a|b>` value name next to a list
/// of possible values, and each shell of completions.
#[test]
fn each_help_screen_passes_the_lint() {
    fn walk(cmd: &clap::Command, path: String, out: &mut Vec<(String, String)>) {
        let mut cmd = cmd.clone();
        out.push((path.clone(), cmd.render_long_help().to_string()));
        for sub in cmd.get_subcommands() {
            if sub.get_name() != "help" {
                walk(sub, format!("{path} {}", sub.get_name()), out);
            }
        }
    }
    let mut screens = Vec::new();
    let mut root = Cli::command();
    root.build();
    walk(&root, "podnet".into(), &mut screens);
    let mut faults = Vec::new();
    for (path, help) in &screens {
        for line in help.lines() {
            let lower = line.to_ascii_lowercase();
            if let Some(at) = lower.find("issue ") {
                if lower[at + 6..].starts_with(|c: char| c.is_ascii_digit()) {
                    faults.push(format!("{path}: an issue number: {line}"));
                }
            }
            if line.contains("=]") && line.contains("[env: ") {
                faults.push(format!("{path}: an env value bracket: {line}"));
            }
            if let Some(at) = line.find(" <") {
                let name = &line[at + 2..];
                if let Some(end) = name.find('>') {
                    let value = &name[..end];
                    if value.contains('|')
                        && !value.contains("--")
                        && !line.trim_start().starts_with("Usage:")
                    {
                        faults.push(format!("{path}: a value name with |: {line}"));
                    }
                }
            }
        }
    }
    assert!(faults.is_empty(), "{}", faults.join("\n"));
    let completions = &screens
        .iter()
        .find(|(p, _)| p == "podnet completions")
        .unwrap()
        .1;
    assert!(completions.contains("PowerShell"), "{completions}");
    assert!(completions.contains("Elvish"), "{completions}");
}

/// E1: each command of podnet is in one group of the top help.
#[test]
fn each_command_is_in_one_help_group() {
    let cmd = Cli::command();
    let mut names: Vec<String> = cmd
        .get_subcommands()
        .map(|c| c.get_name().to_string())
        .collect();
    names.push("help".into());
    let listed: Vec<&str> = HELP_GROUPS
        .iter()
        .flat_map(|(_, n)| n.iter().copied())
        .collect();
    for name in &names {
        let count = listed.iter().filter(|l| **l == name.as_str()).count();
        assert_eq!(count, 1, "{name} is in {count} groups");
    }
    for name in &listed {
        assert!(names.iter().any(|n| n == name), "{name} is no command");
    }
    let help = top_command().render_help().to_string();
    assert!(help.contains("Start here:"), "{help}");
    assert!(!help.contains("(issue "), "{help}");
    assert_eq!(first_sentence("Test it: the proxy. More"), "Test it");
    assert_eq!(first_sentence("One command (issue 87): x"), "One command");
}

/// E7: an example is in the help of its command.
#[test]
fn the_help_of_a_command_has_its_examples() {
    let help = |path: &[&str]| -> String {
        let mut cmd = top_command();
        for name in path {
            cmd = cmd.find_subcommand(name).unwrap().clone();
        }
        cmd.render_long_help().to_string()
    };
    let tunnel = help(&["tunnel"]);
    assert!(tunnel.contains("Examples:"), "{tunnel}");
    assert!(
        tunnel.contains("podnet tunnel --url http://127.0.0.1:8080"),
        "{tunnel}"
    );
    assert!(!tunnel.contains("podnet warp"), "{tunnel}");
    let node = help(&["relay", "node"]);
    assert!(
        node.contains("podnet relay node --to 127.0.0.1:22"),
        "{node}"
    );
    assert_eq!(
        example_path(&Cli::command(), "podnet warp proxy -r 1080"),
        "podnet warp proxy"
    );
    // Each example belongs to a command (not to podnet alone).
    for (command, _) in podnet::manual::facts::EXAMPLES
        .iter()
        .filter(|(command, _)| command.starts_with("podnet "))
    {
        assert_ne!(
            example_path(&Cli::command(), command),
            "podnet",
            "{command}"
        );
    }
}

/// E8: a word that is no command names the command for it.
#[test]
fn a_word_that_is_no_command_gets_a_hint() {
    let hint = |w: &str| word_hint(w).unwrap_or_default();
    assert!(hint("8080").contains("podnet tunnel --url http://localhost:8080"));
    assert!(hint("https://example.org").contains("podnet tunnel --url https://example.org"));
    assert!(hint("wss://x.example").contains("podnet connect --local"));
    assert!(hint("/").contains("podnet serve --dir /"));
    assert!(hint("Cargo.toml").contains("podnet share --mode download --file Cargo.toml"));
    assert_eq!(word_hint("tunel"), None);
    assert_eq!(word_hint("123456"), None, "not a port");
}

/// E10: an unknown command or flag names the version and the update.
#[test]
fn an_unknown_command_names_the_version() {
    let err = top_command()
        .try_get_matches_from(["podnet", "8080"])
        .unwrap_err();
    let lines = usage_hints(&err);
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines[1].contains(env!("CARGO_PKG_VERSION")), "{lines:?}");
    assert!(lines[1].contains("podnet update --check"), "{lines:?}");
    let err = top_command()
        .try_get_matches_from(["podnet", "tunnel", "--bogus"])
        .unwrap_err();
    assert!(usage_hints(&err)[0].contains("'--bogus'"));
    let err = top_command()
        .try_get_matches_from(["podnet", "tunnel", "--port", "x"])
        .unwrap_err();
    assert!(
        usage_hints(&err).is_empty(),
        "a bad value is no unknown flag"
    );
}

/// Issue 616: a value, an extra positional or a flag of another command
/// is a syntax error: no update line. Only a flag that no command knows
/// gets it.
#[test]
fn only_a_flag_that_no_command_knows_names_the_update() {
    let hints = |line: &str| {
        let err = top_command()
            .try_get_matches_from(line.split_whitespace())
            .unwrap_err();
        usage_hints(&err)
    };
    for line in [
        "podnet serve www --pin 123456",
        "podnet relay node X --json",
        "podnet tunnel --hash-password",
        "podnet tunnel -L 1",
        "podnet tunnel -5",
    ] {
        assert!(hints(line).is_empty(), "{line}: {:?}", hints(line));
    }
    for line in ["podnet tunnel --bogus", "podnet relay node X --bogus-flag"] {
        let lines = hints(line);
        assert_eq!(lines.len(), 1, "{line}: {lines:?}");
        assert!(lines[0].contains("podnet update --check"), "{line}");
    }
    assert!(unknown_flag(&top_command(), "--bogus"));
    assert!(!unknown_flag(&top_command(), "--json"));
    assert!(!unknown_flag(&top_command(), "-q"));
    assert!(!unknown_flag(&top_command(), "123456"));
}

#[test]
fn a_bad_command_line_is_a_usage_error() {
    let err = Cli::try_parse_from(["podnet", "tunel"]).unwrap_err();
    assert_eq!(err.kind(), clap::error::ErrorKind::InvalidSubcommand);
    assert!(
        err.to_string().contains("tunnel"),
        "clap suggests the nearest command"
    );
}

#[test]
fn the_command_tree_is_valid() {
    Cli::command().debug_assert();
}

/// Each example of the manual is a command line that the parser accepts.
#[test]
fn each_example_of_the_manual_parses() {
    for (command, _) in podnet::manual::facts::EXAMPLES {
        let words: Vec<&str> = command.split_whitespace().collect();
        assert_eq!(words[0], "podnet", "{command}");
        if words.get(1) == Some(&"pod") {
            // pod has its own parser, in podnet-netns.
            continue;
        }
        if let Err(err) = Cli::try_parse_from(&words) {
            panic!("the example {command:?} does not parse: {err}");
        }
    }
}

/// The manual names each command and each flag of the command tree.
#[test]
fn the_manual_names_each_command_and_flag() {
    let manual = podnet::manual::manual(Cli::command());
    let text = pod_manual::text::render(&manual);
    fn walk(cmd: &clap::Command, path: &mut Vec<String>, text: &str) {
        for sub in cmd
            .get_subcommands()
            .filter(|c| c.get_name() != "help" && !c.is_hide_set())
        {
            path.push(sub.get_name().to_string());
            assert!(
                text.contains(&format!("podnet {}", path.join(" "))),
                "the manual has no section for podnet {}",
                path.join(" ")
            );
            for arg in sub.get_arguments().filter(|a| !a.is_hide_set()) {
                if let Some(long) = arg.get_long() {
                    if long != "help" && long != "version" {
                        assert!(
                            text.contains(&format!("--{long}")),
                            "the manual has no --{long} (podnet {})",
                            path.join(" ")
                        );
                    }
                }
            }
            walk(sub, path, text);
            path.pop();
        }
    }
    walk(&Cli::command(), &mut Vec::new(), &text);
    assert!(
        text.contains("podnet-netns"),
        "the pod section has the podnet-netns usage"
    );
}

/// T-358 step 4: `src/cli/scenarios.txt` has one line for each command with
/// no subcommand (not hidden), and no other line.
#[test]
fn the_table_of_scenarios_has_each_command() {
    fn leaves(cmd: &clap::Command, path: &str, out: &mut Vec<String>) {
        let subs: Vec<&clap::Command> = cmd
            .get_subcommands()
            .filter(|c| c.get_name() != "help" && !c.is_hide_set())
            .collect();
        if subs.is_empty() && !path.is_empty() {
            out.push(path.to_string());
        }
        for sub in subs {
            let next = format!("{path} {}", sub.get_name());
            leaves(sub, next.trim(), out);
        }
    }
    let mut want = Vec::new();
    leaves(&Cli::command(), "", &mut want);
    want.sort();
    let mut have: Vec<String> = scenarios().iter().map(|(p, _)| p.to_string()).collect();
    have.sort();
    assert_eq!(
        have, want,
        "run: python3 scripts/gate-scenarios.py --write, after a new line for a new command"
    );
}

/// T-358 step 4: a command with no scenario says that it is experimental in
/// its help; a command with a scenario does not.
#[test]
fn a_command_with_no_scenario_is_experimental_in_its_help() {
    let help = |path: &str| -> String {
        let mut cmd = top_command();
        for name in path.split_whitespace() {
            cmd = cmd.find_subcommand(name).unwrap().clone();
        }
        cmd.render_long_help().to_string()
    };
    let list = scenarios();
    let (bare, _) = list
        .iter()
        .find(|(_, s)| s.is_empty())
        .expect("an experimental command");
    let (tested, _) = list
        .iter()
        .find(|(_, s)| !s.is_empty())
        .expect("a command with a scenario");
    assert!(help(bare).contains("Experimental: "), "{}", help(bare));
    assert!(!help(tested).contains("Experimental: "), "{}", help(tested));
    assert!(EXPERIMENTAL.starts_with("Experimental: "));
}

/// T-451 (GitHub #612, #647): each `--flag` that a long help names is a flag
/// of the command, of a command above it (a global flag) or below it, or of
/// a command that the same line names before the flag ("as share --password
/// does", "podnet doctor --agent"). A help that names a flag that the
/// command refuses is a false promise.
#[test]
fn each_flag_that_a_help_names_exists() {
    use std::collections::{BTreeMap, BTreeSet};
    fn flags(cmd: &clap::Command) -> BTreeSet<String> {
        cmd.get_arguments()
            .flat_map(|a| {
                a.get_long()
                    .into_iter()
                    .chain(a.get_all_aliases().unwrap_or_default())
                    .map(str::to_string)
            })
            .collect()
    }
    // Each command by its path ("warp tun"), and its flags with those of
    // each command below it.
    fn index(
        cmd: &clap::Command,
        path: &str,
        out: &mut BTreeMap<String, BTreeSet<String>>,
    ) -> BTreeSet<String> {
        let mut all = flags(cmd);
        for sub in cmd.get_subcommands().filter(|s| s.get_name() != "help") {
            let p = format!("{path} {}", sub.get_name()).trim().to_string();
            all.extend(index(sub, &p, out));
        }
        out.insert(path.to_string(), all.clone());
        all
    }
    let mut root = Cli::command();
    root.build();
    let mut commands = BTreeMap::new();
    index(&root, "", &mut commands);
    let flag = regex::Regex::new(r"--([a-z][a-z0-9]*(?:-[a-z0-9]+)*)").unwrap();
    fn walk(
        cmd: &clap::Command,
        path: &str,
        above: &BTreeSet<String>,
        commands: &BTreeMap<String, BTreeSet<String>>,
        flag: &regex::Regex,
        faults: &mut Vec<String>,
    ) {
        let mut own = commands[path].clone();
        own.extend(above.iter().cloned());
        let help = cmd.clone().render_long_help().to_string();
        // One text for each paragraph: clap wraps the lines of a help.
        for para in help.split("\n\n") {
            let line = para.split_whitespace().collect::<Vec<_>>().join(" ");
            for m in flag.captures_iter(&line) {
                let name = &m[1];
                if own.contains(name) || name == "help" || name == "version" {
                    continue;
                }
                // The sentence before the flag.
                let at = m.get(0).unwrap().start();
                let sentence = &line[line[..at].rfind(". ").map_or(0, |i| i + 2)..at];
                // A flag of another program: "magic-wormhole's --code-length".
                if sentence.ends_with("'s ") {
                    continue;
                }
                // A command that the sentence names, as whole words.
                let named = commands.iter().any(|(p, f)| {
                    !p.is_empty()
                        && f.contains(name)
                        && regex::Regex::new(&format!(
                            r"(^|[^a-z-]){}([^a-z-]|$)",
                            regex::escape(p)
                        ))
                        .unwrap()
                        .is_match(sentence)
                });
                if !named {
                    faults.push(format!("podnet {path}: --{name}: {line}"));
                }
            }
        }
        let globals: BTreeSet<String> = cmd
            .get_arguments()
            .filter(|a| a.is_global_set())
            .filter_map(|a| a.get_long().map(str::to_string))
            .chain(above.iter().cloned())
            .collect();
        for sub in cmd.get_subcommands().filter(|s| s.get_name() != "help") {
            let p = format!("{path} {}", sub.get_name()).trim().to_string();
            walk(sub, &p, &globals, commands, flag, faults);
        }
    }
    let mut faults = Vec::new();
    walk(&root, "", &BTreeSet::new(), &commands, &flag, &mut faults);
    assert!(
        faults.is_empty(),
        "{} flags that the command does not have:\n{}",
        faults.len(),
        faults.join("\n")
    );
}
