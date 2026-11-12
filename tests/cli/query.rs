//! The query keys of a carrier address (T-306, GitHub #476, row E10-1).
//! Each refusal exits 64 before a socket opens, and no message shows a value.

use std::process::{Command, Output};

const SECRET: &str = "zzsecretzz";

fn run(sub: &str, address: &str) -> Output {
    let dir = tempfile::tempdir().unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_podnet"));
    cmd.args(["link", sub, address])
        .env("PODNET_STATE_DIR", dir.path().join("state"))
        .env("PODNET_RUN_DIR", dir.path().join("run"))
        .env_remove("RUST_LOG")
        .env_remove("PODNET_ERROR_PROGRAM");
    match sub {
        "listen" => cmd.args(["--allow", "/nonexistent-allow"]),
        _ => cmd.args(["--peer", "SHA256:AAAA"]),
    };
    cmd.output().unwrap()
}

fn text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// Run all three commands and check the exit code and the message.
fn refused(query: &str, needle: &str) {
    for sub in ["listen", "dial", "connect"] {
        let out = run(sub, &format!("/nonexistent/sock?{query}"));
        let t = text(&out);
        assert_eq!(out.status.code(), Some(64), "{sub} {query}: {t}");
        assert!(t.contains(needle), "{sub} {query}: {t}");
        assert!(!t.contains(SECRET), "{sub} {query}: a value is shown: {t}");
        assert!(
            !t.contains("nonexistent/sock"),
            "{sub} {query}: a socket was tried: {t}"
        );
    }
}

#[test]
fn a_repeated_key_exits_64() {
    refused("retry=1&retry=2", "given twice");
}

#[test]
fn an_unknown_key_exits_64_and_lists_the_keys() {
    refused(
        &format!("{SECRET}=1"),
        "carrier, tls, retry, lb, compress, pad, mtu, maxhops, poll, window, name",
    );
}

#[test]
fn a_secret_key_is_refused_with_or_without_a_value() {
    for key in ["password", "key", "token", "identity"] {
        refused(&format!("{key}={SECRET}"), "refused");
        refused(key, "refused");
        refused(&format!("{key}="), "refused");
    }
}

#[test]
fn a_bad_value_exits_64_and_is_not_echoed() {
    for q in [
        format!("retry={SECRET}"),
        "retry=99".to_string(),
        format!("name=A{SECRET}"),
        format!("lb={SECRET}"),
        format!("name=%25{SECRET}"),
        format!("name=caf%C3%A9{SECRET}"),
        format!("poll=%zz{SECRET}"),
    ] {
        refused(&q, "the query key");
    }
}

#[test]
fn the_limits_exit_64() {
    let many = vec!["tls=on"; 17].join("&");
    refused(&many, "16 keys at most");
    let long = format!("name={}", "a".repeat(1100));
    refused(&long, "1024 bytes");
}

#[test]
fn a_key_with_no_effect_names_its_entry() {
    for (q, entry) in [
        ("compress=snappy", "T-299"),
        ("pad=on", "T-300"),
        ("lb=rr", "T-288"),
        ("maxhops=3", "T-287"),
        ("name=ab-1", "--name"),
    ] {
        refused(q, entry);
    }
}

#[test]
fn mtu_of_a_stream_carrier_exits_64() {
    refused("mtu=1200", "stream carrier");
}

#[test]
fn the_carrier_key_takes_the_place_of_the_flag() {
    refused("carrier=nope", "no carrier is named");
}

#[test]
fn a_valid_carrier_key_reaches_the_carrier() {
    let out = run("dial", "/nonexistent/sock?carrier=unix");
    assert_ne!(out.status.code(), Some(64), "{}", text(&out));
}
