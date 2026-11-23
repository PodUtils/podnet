//! EXIT-1 to EXIT-4: the six codes of the contract, one test for each.
//!
//! Each test invokes the binary and reads `.status.code()` from the child,
//! never through a pipe. The help table comes from the one constant of
//! `podnet_core::exit::CONTRACT`.

use std::io::{BufRead, BufReader};
use std::os::unix::fs::PermissionsExt as _;
use std::process::{Command, Stdio};

use podnet::exit;

fn podnet(args: &[&str], dir: &std::path::Path, env: &[(&str, &str)]) -> std::process::Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_podnet"));
    cmd.args(args)
        .env("PODNET_STATE_DIR", dir)
        .env_remove("PODNET_EDGE")
        .env_remove("HTTPS_PROXY")
        .env_remove("ALL_PROXY");
    for (key, value) in env {
        cmd.env(key, value);
    }
    cmd.output().expect("podnet runs")
}

fn stderr(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn exit_64_is_a_bad_command_line() {
    let dir = tempfile::tempdir().unwrap();
    let out = podnet(&["tunel"], dir.path(), &[]);
    assert_eq!(out.status.code(), Some(exit::USAGE), "{}", stderr(&out));
}

#[test]
fn exit_66_is_a_missing_allow_list() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("absent.txt");
    let socket = dir.path().join("listen.sock");
    let out = podnet(
        &[
            "link",
            "listen",
            socket.to_str().unwrap(),
            "--allow",
            missing.to_str().unwrap(),
            "--json",
        ],
        dir.path(),
        &[],
    );
    assert_eq!(out.status.code(), Some(exit::NO_INPUT), "{}", stderr(&out));
    assert!(stderr(&out).contains("no allow list"), "{}", stderr(&out));
}

#[test]
fn exit_69_is_a_listener_that_is_not_available() {
    let dir = tempfile::tempdir().unwrap();
    let out = podnet(&["relay", "connect", "nosuchpair"], dir.path(), &[]);
    assert_eq!(
        out.status.code(),
        Some(exit::UNAVAILABLE),
        "{}",
        stderr(&out)
    );
}

#[test]
fn exit_71_is_a_key_file_with_a_wide_mode() {
    let dir = tempfile::tempdir().unwrap();
    let key = dir.path().join("link-identity.key");
    std::fs::write(&key, b"not a key").unwrap();
    std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o644)).unwrap();
    let out = podnet(&["link", "status", "--create"], dir.path(), &[]);
    assert_eq!(out.status.code(), Some(exit::OS_ERROR), "{}", stderr(&out));
    assert!(stderr(&out).contains("chmod 600"), "{}", stderr(&out));
}

/// A listener whose allow list is empty refuses the dialer with 77, before
/// the dialer opens a stream.
#[test]
fn exit_77_is_a_peer_that_the_allow_list_does_not_admit() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let allow = dir.path().join("allow.txt");
    std::fs::write(&allow, "").unwrap();
    std::fs::set_permissions(&allow, std::fs::Permissions::from_mode(0o644)).unwrap();
    let socket = dir.path().join("listen.sock");
    let mut listener = Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args([
            "link",
            "listen",
            socket.to_str().unwrap(),
            "--allow",
            allow.to_str().unwrap(),
            "--json",
        ])
        .env("PODNET_STATE_DIR", &state)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the listener runs");
    let mut ready = String::new();
    BufReader::new(listener.stdout.take().unwrap())
        .read_line(&mut ready)
        .unwrap();
    let doc: serde_json::Value = serde_json::from_str(&ready).expect("the ready document");
    let fingerprint = doc["fingerprint"].as_str().unwrap().to_string();

    let dialer_state = dir.path().join("dialer");
    let out = podnet(
        &[
            "link",
            "dial",
            socket.to_str().unwrap(),
            "--peer",
            &fingerprint,
            "--check",
        ],
        &dialer_state,
        &[],
    );
    let _ = listener.kill();
    let _ = listener.wait();
    assert_eq!(out.status.code(), Some(exit::NO_PERM), "{}", stderr(&out));
}

#[test]
fn exit_78_is_a_configuration_with_a_listener() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("node.json");
    std::fs::write(
        &config,
        r#"{"schema":1,"node":"a","listen":["0.0.0.0:443"],"relay":"http://relay.example","allow":[],"egress":{"default":"refuse","rules":[]}}"#,
    )
    .unwrap();
    let out = podnet(
        &["init", "check", config.to_str().unwrap()],
        dir.path(),
        &[],
    );
    assert_eq!(out.status.code(), Some(exit::CONFIG), "{}", stderr(&out));
    assert!(stderr(&out).contains("INIT-1"), "{}", stderr(&out));
}

/// EXIT-2: the page holds the same table as the constant.
#[test]
fn the_document_holds_the_same_table() {
    let path = format!("{}/docs/exit-codes.md", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let mut rows = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if !line.starts_with('|') || !line.ends_with('|') {
            continue;
        }
        let cells: Vec<&str> = line.trim_matches('|').split('|').map(str::trim).collect();
        if cells.len() != 4 {
            continue;
        }
        let Ok(code) = cells[0].parse::<i32>() else {
            continue;
        };
        rows.push((
            code,
            cells[1].to_string(),
            cells[2].to_string(),
            cells[3].to_string(),
        ));
    }
    assert_eq!(rows.len(), exit::CONTRACT.len(), "{rows:?}");
    for (code, name, meaning, commands) in exit::CONTRACT {
        let row = rows
            .iter()
            .find(|(other, ..)| *other == code)
            .unwrap_or_else(|| panic!("the page has no row for {code}"));
        assert_eq!(row.1, name, "{code}");
        assert_eq!(row.2, meaning, "{code}");
        assert_eq!(row.3, commands, "{code}");
    }
}

#[test]
fn the_help_holds_the_table_of_the_one_constant() {
    let out = Command::new(env!("CARGO_BIN_EXE_podnet"))
        .arg("--help")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("Exit codes:"), "{text}");
    for (code, name, meaning, _) in exit::CONTRACT {
        assert!(text.contains(&code.to_string()), "{code}: {text}");
        assert!(text.contains(name), "{name}: {text}");
        assert!(
            text.replace('\n', " ")
                .contains(meaning.split(' ').next().unwrap()),
            "{meaning}: {text}"
        );
    }
}

/// T-528: a command that reports "no leg" or "no request" exits with a code
/// whose row of `podnet man exit-status --json` names that command. Port 1
/// of the loopback refuses each connection, and 192.0.2.1 (TEST-NET) never
/// answers: no leg and no request can work.
#[test]
fn a_failed_check_exits_with_a_code_whose_row_names_the_command() {
    let dir = tempfile::tempdir().unwrap();
    let man = podnet(&["man", "exit-status", "--json"], dir.path(), &[]);
    assert_eq!(man.status.code(), Some(0), "{}", stderr(&man));
    let man: serde_json::Value = serde_json::from_slice(&man.stdout).expect("the manual is JSON");
    let edge = [
        ("PODNET_EDGE", "192.0.2.1:7844"),
        ("PODNET_RUN_DIR", "/nonexistent/podnet-run"),
    ];
    let doctor = [
        "--proxy",
        "http://127.0.0.1:1",
        "doctor",
        "--samples",
        "1",
        "--timeout",
        "1",
    ];
    let doctor_json = [&doctor[..], &["--json"]].concat();
    let bench = [
        "bench",
        "--duration",
        "1",
        "--timeout",
        "1",
        "http://127.0.0.1:1/",
    ];
    let rows: [(&str, &[&str]); 3] = [
        ("podnet doctor", &doctor),
        ("podnet doctor", &doctor_json),
        ("podnet bench", &bench),
    ];
    let mut wrong = Vec::new();
    for (command, args) in rows {
        let out = podnet(args, dir.path(), &edge);
        let code = out.status.code();
        let row = man["exits"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["code"]["value"].as_i64().map(|v| v as i32) == code);
        let names = row.is_some_and(|e| e["meaning"].as_str().unwrap_or("").contains(command));
        if code == Some(0) || !names {
            wrong.push(format!(
                "{args:?}: exit {code:?}, and its row does not name {command}: {row:?}\n{}",
                stderr(&out)
            ));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}
