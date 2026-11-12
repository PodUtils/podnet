//! Exit codes, usage errors and configuration errors.

use super::*;

#[test]
fn help_and_version_exit_0() {
    for args in [&["--help"][..], &["--version"], &["tunnel", "--help"]] {
        let out = podnet(args);
        assert_eq!(out.status.code(), Some(0), "podnet {args:?}");
    }
}

#[test]
fn a_usage_error_exits_64() {
    let out = podnet(&["tunel"]);
    assert_eq!(out.status.code(), Some(64));
}

/// A flag with no value and an unknown flag give different messages.
#[test]
fn a_missing_value_is_not_an_unknown_flag() {
    let missing = podnet(&["tunnel", "--protocol"]);
    assert_eq!(missing.status.code(), Some(64));
    let text = stderr(&missing);
    assert!(
        text.contains("a value is required for '--protocol"),
        "{text}"
    );
    assert!(!text.contains("unexpected argument"), "{text}");

    let unknown = podnet(&["tunnel", "--protocl", "quic"]);
    assert_eq!(unknown.status.code(), Some(64));
    let text = stderr(&unknown);
    assert!(text.contains("unexpected argument '--protocl'"), "{text}");
    assert!(
        text.contains("--protocol"),
        "clap names the nearest flag: {text}"
    );
}

#[test]
fn a_bad_configuration_exits_78() {
    let out = Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(["tunnel", "--url", "http://127.0.0.1:9"])
        .env_remove("PODNET_ERROR_PROGRAM")
        .env("PODNET_EDGE", "not-an-address")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(78), "{}", stderr(&out));
    assert!(stderr(&out).contains("PODNET_EDGE"), "{}", stderr(&out));

    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing.yml");
    let out = podnet(&["tunnel", "--config", missing.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(78), "{}", stderr(&out));
}

/// A bad value of a tunnel flag is a usage error (exit 64) before any
/// network work.
#[test]
fn bad_flag_values_are_usage_errors() {
    for (args, want) in [
        (
            &["tunnel", "--hello-world", "--ha-connections", "0"][..],
            "1..=8",
        ),
        (
            &["tunnel", "--hello-world", "--ha-connections", "9"],
            "1..=8",
        ),
        (
            &["tunnel", "--hello-world", "--protocol", "bogus"],
            "possible values",
        ),
        (
            &["tunnel", "--hello-world", "--transport", "bogus"],
            "possible values",
        ),
        (&["tunnel", "--port", "0"], "1..=65535"),
        (
            &["tunnel", "--unix", "/nonexistent/x.sock"],
            "--unix /nonexistent/x.sock",
        ),
        (&["tunnel", "--tcp", "bogus"], "--tcp bogus"),
        (&["tunnel", "--tcp", "127.0.0.1:0"], "--tcp 127.0.0.1:0"),
        (&["tunnel", "--url", "http://127.0.0.1:0"], "the port 0"),
        (
            &[
                "connect",
                "-L",
                "tcp://127.0.0.1:1",
                "http://127.0.0.1:19200/",
            ],
            "not a WebSocket URL",
        ),
    ] {
        let start = std::time::Instant::now();
        let out = podnet(args);
        let text = stderr(&out);
        assert_eq!(out.status.code(), Some(64), "{args:?}: {text}");
        assert!(text.contains(want), "{args:?}: {text}");
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "{args:?} was slow"
        );
    }
}

/// Issue 57: no panic on a bad argument or a closed pipe; a missing
/// subcommand and a missing origin are usage errors; --json is accepted.
#[test]
fn bad_input_gives_usage_errors_not_panics() {
    use std::os::unix::ffi::OsStrExt;
    let bad = std::ffi::OsStr::from_bytes(b"\xff\xfe");
    for args in [vec![bad], vec![std::ffi::OsStr::new("tunnel"), bad]] {
        let out = Command::new(env!("CARGO_BIN_EXE_podnet"))
            .args(&args)
            .env_remove("PODNET_ERROR_PROGRAM")
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(64), "{args:?}: {}", stderr(&out));
    }
    for args in [&[][..], &["net"][..], &["warp"][..], &["tunnel"][..]] {
        let out = podnet(args);
        assert_eq!(out.status.code(), Some(64), "{args:?}: {}", stderr(&out));
    }
    assert_eq!(podnet(&["--help"]).status.code(), Some(0));
    let no_host = podnet(&["tunnel", "--check", "--url", "http://"]);
    assert_eq!(no_host.status.code(), Some(64), "{}", stderr(&no_host));
    assert!(stderr(&no_host).contains("has no host"));
    for args in [
        &["status", "--json"][..],
        &["net", "addresses", "--json"][..],
    ] {
        let out = podnet(args);
        assert_eq!(out.status.code(), Some(0), "{args:?}: {}", stderr(&out));
        serde_json::from_slice::<serde_json::Value>(&out.stdout).unwrap();
    }
    // A closed stdout: the default SIGPIPE ends the command, no panic (101).
    let mut child = Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(["completions", "bash"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let out = child.wait_with_output().unwrap();
    assert_ne!(out.status.code(), Some(101), "{}", stderr(&out));
    assert!(!stderr(&out).contains("panicked"), "{}", stderr(&out));
}

/// Issue 55: the failure report has the error in "error" only; "messages"
/// holds the log lines before it, not the same text again.
#[test]
fn the_failure_report_has_the_error_once() {
    let dir = tempfile::tempdir().unwrap();
    let saved = dir.path().join("report.json");
    let program = dir.path().join("save.sh");
    std::fs::write(&program, format!("#!/bin/sh\ncat > {}\n", saved.display())).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(["tunnel", "--url", "ftp://x"])
        .env("PODNET_ERROR_PROGRAM", &program)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(64));
    let doc: serde_json::Value = serde_json::from_slice(&std::fs::read(&saved).unwrap()).unwrap();
    let error = doc["error"].to_string();
    assert!(error.contains("ftp"), "{doc}");
    for line in doc["messages"].as_array().unwrap() {
        assert!(
            !line.as_str().unwrap().contains("ftp://x"),
            "the error is twice: {doc}"
        );
    }
}

/// Issue 172: a bad credentials file exits 78 as a flag and as a key; a
/// bad token exits 78; --token with --credentials-file exits 64.
#[test]
fn bad_credentials_exit_78() {
    let dir = tempfile::tempdir().unwrap();
    let creds = dir.path().join("c.json");
    std::fs::write(&creds, "{}").unwrap();
    let creds = creds.to_str().unwrap();
    let config = dir.path().join("config.yml");
    std::fs::write(
        &config,
        format!("credentials-file: {creds}\ningress:\n  - service: http_status:404\n"),
    )
    .unwrap();
    for args in [
        vec![
            "tunnel",
            "--hello-world",
            "--credentials-file",
            creds,
            "--check",
        ],
        vec!["tunnel", "--config", config.to_str().unwrap(), "--check"],
        vec!["tunnel", "--hello-world", "--token", "garbage", "--check"],
        vec!["serve", "--dir", ".", "--token", "garbage", "--check"],
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_podnet"))
            .args(&args)
            .env_remove("TUNNEL_TOKEN")
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(78), "{args:?}: {}", stderr(&out));
    }
    let out = Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args([
            "tunnel",
            "--hello-world",
            "--token",
            "x",
            "--credentials-file",
            creds,
        ])
        .env_remove("TUNNEL_TOKEN")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(64), "{}", stderr(&out));
}

/// Issue 269: a config file with no rule fails --check with exit 78.
#[test]
fn a_config_with_no_rule_fails_the_check() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("empty.yml");
    std::fs::write(&config, "ingress: []\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(["tunnel", "--config", config.to_str().unwrap(), "--check"])
        .env("PODNET_EDGE", "127.0.0.1:9")
        .env_remove("TUNNEL_TOKEN")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(78), "{}", stderr(&out));
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), stderr(&out));
    assert!(text.contains("no ingress rules"), "{text}");
}

/// Issue 173: the exit code of each class of error.
#[test]
fn exit_codes_of_flags_and_configuration() {
    let run = |args: &[&str], env: &[(&str, &str)]| {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_podnet"));
        cmd.args(args)
            .env_remove("PODNET_EDGE")
            .env_remove("TUNNEL_TOKEN");
        for (k, v) in env {
            cmd.env(k, v);
        }
        let out = cmd.output().unwrap();
        (out.status.code(), stderr(&out))
    };
    for (args, env, want) in [
        // Issue 250: a bare `podnet share` is a usage error.
        // PODNET_EDGE=bogus:: makes sure that no tunnel starts.
        (&["share"][..], &[("PODNET_EDGE", "bogus::")][..], 64),
        (&["share", "--check"], &[], 64),
        (&["share", "--mode", "download", "--check"], &[], 64),
        (&["share", "--mode", "proxy", "--check"], &[], 64),
        (
            &["tunnel", "--hello-world", "--check"],
            &[("PODNET_EDGE", "bogus::")][..],
            78,
        ),
        (
            &[
                "tunnel",
                "--hello-world",
                "--protocol",
                "http2",
                "--transport",
                "warp",
                "--check",
            ],
            &[],
            64,
        ),
        (&["--ca-file", "/nonexistent/roots.pem", "status"], &[], 78),
        (
            &[
                "--ca-file",
                "/nonexistent/roots.pem",
                "tunnel",
                "--hello-world",
                "--check",
            ],
            &[],
            78,
        ),
        (&["autostart", "add", "bad name", "--", "tunnel"], &[], 64),
        // Issue 249.
        (&["tunnel", "--hello-world", "--name", ""], &[], 64),
        (&["tunnel", "--hello-world", "--name", "a b"], &[], 64),
        (
            &["connect", "--local", "tcp://abc", "wss://x.example/y"],
            &[],
            64,
        ),
        (
            &["connect", "--local", "tcp://99999", "wss://x.example/y"],
            &[],
            64,
        ),
        (&["update", "--version", ""], &[], 64),
        (&["update", "--version", "v1 x"], &[], 64),
    ] {
        let (code, err) = run(args, env);
        assert_eq!(code, Some(want), "{args:?}: {err}");
    }
    // Issue 249: no pair is 69, a pair file that does not parse is 78, a
    // --file that is not there is 78.
    let state = tempfile::tempdir().unwrap();
    let st = state.path().to_str().unwrap();
    let env = [("PODNET_STATE_DIR", st)];
    for args in [
        &["relay", "connect", "nosuchpair"][..],
        &["relay", "stop", "nosuchpair"],
    ] {
        let (code, err) = run(args, &env);
        assert_eq!(code, Some(69), "{args:?}: {err}");
    }
    std::fs::create_dir_all(state.path().join("relay")).unwrap();
    std::fs::write(state.path().join("relay/pair-bad.json"), "{").unwrap();
    for args in [&["relay", "connect", "bad"][..], &["relay", "stop", "bad"]] {
        let (code, err) = run(args, &env);
        assert_eq!(code, Some(78), "{args:?}: {err}");
    }
    let (code, err) = run(&["share", "--file", "/nonexistent/f"], &[]);
    assert_eq!(code, Some(78), "{err}");
}

/// Issue 196: the fault flags of net proxy need --switch and a value from
/// 0.0 to 1.0.
#[test]
fn the_fault_flags_need_a_switch_and_a_probability() {
    let out = podnet(&["net", "proxy", "--drop", "0.1", "--run-for", "1"]);
    assert_eq!(out.status.code(), Some(64), "{}", stderr(&out));
    let out = podnet(&[
        "net",
        "proxy",
        "--switch",
        "unix:/nonexistent.sock",
        "--address",
        "10.66.0.3",
        "--reorder",
        "1.5",
    ]);
    assert_eq!(out.status.code(), Some(64), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("not from 0.0 to 1.0"),
        "{}",
        stderr(&out)
    );
}

// Issue 252: invalid values are errors, not silent.
#[test]
fn invalid_values_are_refused() {
    for args in [
        &["warp", "scan", "--timeout-ms", "0"][..],
        &["warp", "scan", "--timeout-ms", "999999"],
    ] {
        let out = podnet(args);
        assert_eq!(out.status.code(), Some(64), "{args:?}: {}", stderr(&out));
    }
    let out = Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(["ports"])
        .env("PODNET_RESOLVE", "bad")
        .env_remove("PODNET_ERROR_PROGRAM")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(78), "{}", stderr(&out));
    assert!(stderr(&out).contains("PODNET_RESOLVE"), "{}", stderr(&out));
    let out = podnet(&["--resolve", "bad", "ports"]);
    assert_eq!(out.status.code(), Some(64), "{}", stderr(&out));
    let state = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(["status"])
        .env("PODNET_EDGE", "zz")
        .env("PODNET_STATE_DIR", state.path())
        .env_remove("PODNET_ERROR_PROGRAM")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("PODNET_EDGE entry \"zz\""),
        "{}",
        stderr(&out)
    );
}

// Issue 253: the shape of the error messages.
#[test]
fn error_messages_have_one_shape() {
    // A parent command with no subcommand: an error and a short usage.
    for parent in ["net", "warp", "relay", "autostart", "agent"] {
        let out = podnet(&[parent]);
        let err = stderr(&out);
        assert_eq!(out.status.code(), Some(64), "{parent}: {err}");
        assert!(err.contains("error:"), "{parent}: {err}");
        assert!(err.lines().count() < 15, "{parent}: {err}");
    }
    // --ca-file is named once.
    let out = podnet(&["--ca-file", "/nonexistent/roots.pem", "status"]);
    assert_eq!(out.status.code(), Some(78));
    assert_eq!(
        stderr(&out).matches("--ca-file").count(),
        1,
        "{}",
        stderr(&out)
    );
    // --port 0 is refused by the parser, for tunnel and share.
    for args in [&["tunnel", "--port", "0"][..], &["share", "--port", "0"]] {
        let out = podnet(args);
        assert_eq!(out.status.code(), Some(64), "{args:?}: {}", stderr(&out));
        assert!(
            stderr(&out).contains("--port"),
            "{args:?}: {}",
            stderr(&out)
        );
    }
    // An empty value names its flag.
    for flag in ["--url", "--tcp", "--forward", "--unix"] {
        let out = podnet(&["tunnel", flag, ""]);
        assert_eq!(out.status.code(), Some(64), "{flag}: {}", stderr(&out));
        assert!(stderr(&out).contains(flag), "{flag}: {}", stderr(&out));
    }
    // stop with no target names --all too.
    let out = podnet(&["stop"]);
    assert_eq!(out.status.code(), Some(64));
    assert!(stderr(&out).contains("--all"), "{}", stderr(&out));
    // net addresses names its columns.
    let out = podnet(&["net", "addresses"]);
    assert!(out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("ADDRESS"),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
}

/// Issue 74: a write that fails (a full disk) does not panic. Before, the
/// std print macros panicked with exit 101.
#[test]
fn a_full_stdout_is_no_panic() {
    let full = std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/full")
        .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(["status"])
        .stdout(full)
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!String::from_utf8_lossy(&out.stderr).contains("panicked"));
}

/// Issue 189: a global option before `pod` does not panic, and a network
/// option of podnet is refused with a pointer to `-x`.
#[test]
fn a_global_flag_before_pod_does_not_panic() {
    for args in [
        &["-v", "pod", "--", "true"][..],
        &["-q", "pod", "--", "true"],
    ] {
        let out = podnet(args);
        let err = stderr(&out);
        assert_ne!(out.status.code(), Some(101), "{args:?}: {err}");
        assert!(!err.contains("panicked"), "{args:?}: {err}");
    }
    let out = podnet(&["--proxy", "http://127.0.0.1:1", "pod", "--", "true"]);
    assert_eq!(out.status.code(), Some(64), "{}", stderr(&out));
    assert!(stderr(&out).contains("-x"), "{}", stderr(&out));
}
