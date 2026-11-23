//! A configuration input that cannot be used exits 78 with one line that
//! names the input, for each command that reads it (T-518, T-531; GitHub
//! #700, #687). The inputs: a policy file (`rules check FILE`,
//! `PODNET_POLICY`) that is missing, a directory, empty, not JSON, or with a
//! bad rule; and a bad `PODNET_RESOLVE`. The commands: `rules check` (also
//! with `--json`), `bench`, and `pod` on the default backend and on the
//! seccomp backend. None of them runs: the check comes before any work.

use std::process::Command;

/// The arguments, the variable and its value, and the text that the line
/// names.
type Row<'a> = (Vec<&'a str>, Option<(&'a str, &'a str)>, &'a str);

#[test]
fn a_bad_configuration_input_exits_78_with_one_line_that_names_it() {
    let dir = tempfile::tempdir().unwrap();
    let file = |name: &str, text: &str| {
        let path = dir.path().join(name);
        std::fs::write(&path, text).unwrap();
        path.display().to_string()
    };
    let missing = dir.path().join("missing.json").display().to_string();
    let directory = dir.path().display().to_string();
    let empty = file("empty.json", "");
    let not_json = file("bad.json", r#"{"default":"refuse","rules":["#);
    let action = file("action.json", r#"{"default":"allow","rules":[]}"#);
    let twice = file(
        "twice.json",
        r#"{"rules":[{"name":"a","host":"x","action":"direct"},{"name":"a","host":"y","action":"direct"}]}"#,
    );
    let policies = [&missing, &directory, &empty, &not_json, &action, &twice];

    let bench = ["bench", "--duration", "1", "http://127.0.0.1:1/"];
    let pod = ["pod", "-x", "socks5://127.0.0.1:9", "--", "true"];
    let seccomp = [
        "pod",
        "--backend",
        "seccomp",
        "-x",
        "socks5://127.0.0.1:9",
        "--",
        "true",
    ];
    let mut rows: Vec<Row> = Vec::new();
    for policy in policies {
        rows.push((vec!["rules", "check", policy], None, policy));
        rows.push((vec!["rules", "check", policy, "--json"], None, policy));
        for command in [&bench[..], &pod[..], &seccomp[..]] {
            rows.push((command.to_vec(), Some(("PODNET_POLICY", policy)), policy));
        }
    }
    for command in [&bench[..], &pod[..], &seccomp[..]] {
        rows.push((
            command.to_vec(),
            Some(("PODNET_RESOLVE", "bad")),
            "PODNET_RESOLVE",
        ));
    }

    let mut faults = Vec::new();
    for (args, variable, named) in &rows {
        let mut command = Command::new(env!("CARGO_BIN_EXE_podnet"));
        command
            .args(args)
            .env_remove("PODNET_POLICY")
            .env_remove("PODNET_NODE_CONFIG")
            .env_remove("PODNET_RESOLVE");
        if let Some((key, value)) = variable {
            command.env(key, value);
        }
        let out = command.output().unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        let lines: Vec<&str> = stderr.lines().collect();
        let fault = if out.status.code() != Some(78) {
            Some(format!("exit {:?}, not 78", out.status.code()))
        } else if lines.len() != 1 {
            Some(format!("{} lines, not one", lines.len()))
        } else if !lines[0].contains(named) {
            Some(format!("the line does not name {named}"))
        } else {
            None
        };
        if let Some(fault) = fault {
            faults.push(format!(
                "{variable:?} podnet {}: {fault}\n{stderr}",
                args.join(" ")
            ));
        }
    }
    assert!(faults.is_empty(), "{}", faults.join("\n"));
}
