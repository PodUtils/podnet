use crate::cli::help::{
    example_path, first_sentence, top_command, usage_hints, word_hint, HELP_GROUPS,
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
    for e in podnet::manual::facts::EXAMPLES
        .iter()
        .filter(|e| e.command.starts_with("podnet "))
    {
        assert_ne!(
            example_path(&Cli::command(), e.command),
            "podnet",
            "{}",
            e.command
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
    for example in podnet::manual::facts::EXAMPLES {
        let words: Vec<&str> = example.command.split_whitespace().collect();
        assert_eq!(words[0], "podnet", "{}", example.command);
        if words.get(1) == Some(&"pod") {
            // pod has its own parser, in podnet-netns.
            continue;
        }
        if let Err(err) = Cli::try_parse_from(&words) {
            panic!("the example {:?} does not parse: {err}", example.command);
        }
    }
}

/// The manual names each command and each flag of the command tree.
#[test]
fn the_manual_names_each_command_and_flag() {
    let manual = podnet::manual::model::build(Cli::command());
    let text = podnet::manual::text::render(&manual);
    fn walk(cmd: &clap::Command, path: &mut Vec<String>, text: &str) {
        for sub in cmd.get_subcommands().filter(|c| c.get_name() != "help") {
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
