//! The flags of `podnet pod` in the manual (T-455).
//!
//! `podnet pod` has its own parser and help (podnet-netns, with no clap):
//! the command of clap in podnet is a stub that takes the rest of the line.
//! The manual reads the flags from the one source, the text of
//! `podnet_netns::usage_for`: each line of `Options:` that starts with a
//! flag, and the lines below it that continue its help. `--help --json`,
//! `man --json` and the llms files then have the flags of `pod`.

use pod_manual::model::Flag;

/// The flags of the `Options:` section of `usage`.
pub(super) fn flags(usage: &str) -> Vec<Flag> {
    let Some(start) = usage.find("\nOptions:\n") else {
        return Vec::new();
    };
    let mut out: Vec<Flag> = Vec::new();
    for line in usage[start + "\nOptions:\n".len()..].lines() {
        if line.trim().is_empty() {
            break;
        }
        match flag_line(line) {
            Some(flag) => out.push(flag),
            None => {
                // A line that continues the help of the flag above.
                if let Some(last) = out.last_mut() {
                    last.help.push(' ');
                    last.help.push_str(line.trim());
                }
            }
        }
    }
    // The flags of the help and the version are not shown (as for the
    // other commands).
    out.retain(|f| !matches!(f.long.as_deref(), Some("help" | "version")));
    out
}

/// One line that starts a flag: `  -x, --proxy <SPEC>  help` or
/// `      --map <NAME=ADDR>  help` or `      --allow-raw icmp  help`.
fn flag_line(line: &str) -> Option<Flag> {
    let rest = line.strip_prefix("  ")?;
    let (short, rest) = match rest.strip_prefix('-') {
        Some(r) if !r.starts_with('-') => {
            let mut chars = r.chars();
            let c = chars.next()?;
            (Some(c), chars.as_str().strip_prefix(", ")?)
        }
        // A flag with no short name starts at column 6; a line of help that
        // starts with `--` is indented more.
        _ => (None, rest.strip_prefix("    ")?),
    };
    let rest = rest.strip_prefix("--")?;
    // The term ends at two spaces, or at the end of the line.
    let (term, help) = match rest.find("  ") {
        Some(i) => (&rest[..i], rest[i..].trim()),
        None => (rest, ""),
    };
    let mut words = term.split(' ');
    let long = words.next()?;
    if long.is_empty() || !long.chars().all(|c| c.is_ascii_lowercase() || c == '-') {
        return None;
    }
    let value = words
        .next()
        .map(|v| v.trim_start_matches('<').trim_end_matches('>').to_string());
    let mut flag = Flag::long(long, help);
    flag.short = short;
    flag.value = value;
    flag.repeatable = help.contains("repeatable");
    Some(flag)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T-455: each `--flag` that the help of `podnet pod` names in its
    /// first column is a flag of the manual, and the reverse; each takes a
    /// value when the help names one.
    #[test]
    fn the_manual_has_each_flag_of_the_help_of_pod() {
        let usage = podnet_netns::usage_for("podnet pod");
        let flags = flags(&usage);
        let longs: Vec<&str> = flags.iter().filter_map(|f| f.long.as_deref()).collect();
        let mut column = Vec::new();
        for line in usage.lines() {
            let t = line.trim_start();
            let t = t
                .strip_prefix("-x, ")
                .or(t.strip_prefix("-r, "))
                .unwrap_or(t);
            let t = t
                .split_once(", ")
                .map_or(t, |(a, b)| if a.len() == 2 { b } else { t });
            if line.starts_with("  ") && !line.starts_with("        ") && t.starts_with("--") {
                let word = t[2..].split([' ', '\t']).next().unwrap_or("");
                if !matches!(word, "help" | "version") {
                    column.push(word);
                }
            }
        }
        assert!(
            column.len() >= 20,
            "the help of pod has few flags: {column:?}"
        );
        assert_eq!(
            longs, column,
            "the flags of the manual and the help of pod differ"
        );
        let proxy = flags
            .iter()
            .find(|f| f.long.as_deref() == Some("proxy"))
            .unwrap();
        assert_eq!(
            (proxy.short, proxy.value.as_deref()),
            (Some('x'), Some("SPEC"))
        );
        assert!(proxy.help.contains("Default: $ALL_PROXY"), "{}", proxy.help);
        let keep = flags
            .iter()
            .find(|f| f.long.as_deref() == Some("keep-proxy-env"))
            .unwrap();
        assert_eq!(keep.value, None);
    }
}
