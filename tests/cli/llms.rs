//! T-222: `--help --json`, `llms.txt` and `llms-full.txt`.

use super::*;

/// `--help --json` prints the JSON of `man --json` for the command, at each
/// level.
#[test]
fn help_json_matches_man_json() {
    let cases: &[(&[&str], &[&str])] = &[
        (&["--help", "--json"], &["man", "--json"]),
        (&["--json", "-h"], &["man", "--json"]),
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

/// `llms.txt` and `llms-full.txt` are the output of `scripts/gen-llms.py` for
/// this binary. A change of the command tree, of the manual or of a document
/// makes this test fail until `python3 scripts/gen-llms.py BINARY` runs again.
#[test]
fn llms_files_are_current() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let out = Command::new("python3")
        .arg("-I")
        .arg(root.join("scripts/gen-llms.py"))
        .arg("--check")
        .arg(env!("CARGO_BIN_EXE_podnet"))
        .output()
        .expect("python3 runs");
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        stderr(&out)
    );
}
