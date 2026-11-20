//! T-222: `--help --json`. The llms files: `src/cli/manual_tests.rs`.

use super::*;

/// `--help --json` prints the JSON of `man --json COMMAND` for the command,
/// at each level; at the root, the JSON of the root command.
#[test]
fn help_json_matches_man_json() {
    for help in [&["--help", "--json"][..], &["--json", "-h"][..]] {
        let h = podnet(help);
        assert_eq!(h.status.code(), Some(0), "{help:?}: {}", stderr(&h));
        let json: serde_json::Value = serde_json::from_slice(&h.stdout).expect("valid JSON");
        assert_eq!(json["command"]["path"], serde_json::json!([]), "{json}");
        assert!(json["command"]["flags"].as_array().unwrap().len() > 5);
    }
    let cases: &[(&[&str], &[&str])] = &[
        (
            &["tunnel", "--help", "--json"],
            &["man", "--json", "tunnel"],
        ),
        (
            &["warp", "run", "--help", "--json"],
            &["man", "--json", "warp run"],
        ),
        (
            &["status", "--json", "--help"],
            &["man", "--json", "status"],
        ),
        (
            &["-q", "net", "ts-shims", "-h", "--json"],
            &["man", "--json", "net ts-shims"],
        ),
        // T-455: pod has its own parser; its help is JSON too.
        (&["pod", "--help", "--json"], &["man", "--json", "pod"]),
    ];
    for (help, man) in cases {
        let h = podnet(help);
        let m = podnet(man);
        assert_eq!(h.status.code(), Some(0), "{help:?}: {}", stderr(&h));
        assert_eq!(m.status.code(), Some(0), "{man:?}: {}", stderr(&m));
        assert_eq!(h.stdout, m.stdout, "{help:?} differs from {man:?}");
        serde_json::from_slice::<serde_json::Value>(&h.stdout).expect("valid JSON");
    }
    // Without --json the help is text.
    let text = podnet(&["tunnel", "--help"]);
    assert!(String::from_utf8_lossy(&text.stdout).contains("Usage:"));
}

/// T-455: the JSON of `pod` has the flags of the help of pod (its own
/// parser), not the global flags of the stub of clap; the words after `--`
/// stay with the program.
#[test]
fn the_help_json_of_pod_has_its_flags() {
    let h = podnet(&["pod", "--help", "--json"]);
    assert_eq!(h.status.code(), Some(0), "{}", stderr(&h));
    let json: serde_json::Value = serde_json::from_slice(&h.stdout).expect("valid JSON");
    let longs: Vec<&str> = json["command"]["flags"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| f["long"].as_str())
        .collect();
    for want in [
        "proxy",
        "rule",
        "backend",
        "report",
        "keep-proxy-env",
        "serve",
    ] {
        assert!(longs.contains(&want), "no --{want}: {longs:?}");
    }
    for global in ["color", "log-format", "ca-file"] {
        assert!(
            !longs.contains(&global),
            "--{global} is not a flag of pod: {longs:?}"
        );
    }
    let text = String::from_utf8_lossy(&podnet(&["pod", "--help"]).stdout).into_owned();
    assert!(text.contains("Usage: podnet pod"), "{text}");
}
