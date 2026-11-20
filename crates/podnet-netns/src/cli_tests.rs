//! The parser of `podnet pod` and its help (T-500).

use super::*;

/// Each flag of the first column of the help (`  -x, --proxy <SPEC>`,
/// `      --map <NAME=ADDR>`) is a flag of the parser: with its value when
/// the help names one, the parser gives no "unexpected argument". The
/// manual of podnet reads the flags of `pod` from this help
/// (`src/manual/pod_flags.rs`), so the manual and the parser agree.
#[test]
fn each_flag_of_the_help_is_a_flag_of_the_parser() {
    let usage = usage_for("podnet pod");
    let options = &usage[usage.find("\nOptions:\n").expect("an Options section")..];
    let mut seen = 0;
    for line in options.lines().skip(2) {
        if line.trim().is_empty() {
            break;
        }
        let term = if let Some(rest) = line.strip_prefix("  -").filter(|r| !r.starts_with('-')) {
            rest.get(3..).unwrap_or("")
        } else if let Some(rest) = line.strip_prefix("      --") {
            rest
        } else {
            continue;
        };
        let term = term.trim_start_matches('-');
        let term = term.split("  ").next().unwrap_or("");
        let mut words = term.split(' ');
        let long = format!("--{}", words.next().unwrap_or(""));
        if long == "--help" || long == "--version" {
            continue;
        }
        let mut argv = vec![long.clone()];
        if let Some(value) = words.next() {
            argv.push(sample(value));
        }
        argv.extend(["--".to_string(), "true".to_string()]);
        if let Err(e) = parse_args(&argv) {
            let text = format!("{e:#}");
            assert!(!text.contains("unexpected argument"), "{argv:?}: {text}");
        }
        seen += 1;
    }
    assert!(seen >= 20, "{seen} flags in the help");
}

/// A value of the form that the help names.
fn sample(value: &str) -> String {
    match value {
        "<SPEC>" => "direct".into(),
        "<NAME=ADDR>" => "a.example=192.0.2.1".into(),
        "<NAME>" => "auto".into(),
        "<A>" => "allow".into(),
        "<N>" => "1024".into(),
        other => other.trim_matches(['<', '>']).to_lowercase(),
    }
}
