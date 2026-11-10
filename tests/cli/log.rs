//! The log: no secret, colours, controls and formats.

use super::*;

/// The log at the highest level holds no tunnel secret, no WARP token and no
/// proxy password.
#[test]
fn the_log_holds_no_secret() {
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD;
    let secret = b64.encode(b"tunnel-secret-0123456789abcdef!!");
    let token = b64.encode(format!(
        r#"{{"a":"0123456789abcdef0123456789abcdef","t":"6ff42ae2-765d-4adf-8112-31c55c1551ef","s":"{secret}"}}"#
    ));
    let (code, text) = trace(
        &["tunnel", "--token", &token, "--hello-world"],
        &[("PODNET_EDGE", "not-an-address")],
    );
    assert_eq!(code, Some(78), "{text}");
    assert!(!text.contains(&token), "the token is in the log:\n{text}");
    assert!(!text.contains(&secret), "the secret is in the log:\n{text}");

    // A quick tunnel through a proxy with a password; the proxy is closed.
    let password = "Pr0xyPassw0rdValue";
    let proxy = format!("http://alice:{password}@127.0.0.1:9");
    let (code, text) = trace(
        &["tunnel", "--hello-world"],
        &[("HTTPS_PROXY", &proxy), ("ALL_PROXY", &proxy)],
    );
    assert_ne!(code, Some(0), "{text}");
    assert!(
        text.contains("127.0.0.1:9"),
        "the run used the proxy:\n{text}"
    );
    assert!(
        !text.contains(password),
        "the proxy password is in the log:\n{text}"
    );

    // A WARP identity with a bearer token, refreshed through the closed proxy.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("warp.json");
    let warp_token = "warp-bearer-token-0123456789abcdef";
    let device: podnet::warp::register::Device = serde_json::from_value(serde_json::json!({
        "id": "t.0123456789",
        "account": {"id": "acct", "license": "LICENSE-KEY-0123"},
        "config": {},
        "token": warp_token,
        "tunnel_type": "masque",
    }))
    .unwrap();
    let identity = podnet::warp::identity::Identity::generate().unwrap();
    podnet::warp::WarpClient::from_parts(device, identity)
        .save(&path)
        .unwrap();
    let (code, text) = trace(
        &["warp", "--identity", path.to_str().unwrap(), "status"],
        &[("HTTPS_PROXY", &proxy), ("ALL_PROXY", &proxy)],
    );
    assert_ne!(code, Some(0), "{text}");
    assert!(
        !text.contains(warp_token),
        "the WARP token is in the log:\n{text}"
    );
    assert!(
        !text.contains("LICENSE-KEY-0123"),
        "the license is in the log:\n{text}"
    );
    assert!(
        !text.contains(password),
        "the proxy password is in the log:\n{text}"
    );
}

/// A log that goes to a file or a pipe has no escape sequences.
#[test]
fn a_log_to_a_pipe_has_no_colours() {
    let out = Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(["tunnel", "--hello-world"])
        .env("PODNET_EDGE", "not-an-address")
        .env("RUST_LOG", "podnet=debug")
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stderr);
    assert!(!text.contains('\u{1b}'), "{text:?}");
}

/// --log-format json writes one JSON object for each log line; -q drops
/// the info and debug lines; -v adds the debug lines.
#[test]
fn log_controls() {
    test_host::need_loopback!();
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for mut s in listener.incoming().flatten() {
            let mut buf = [0u8; 4096];
            let _ = s.read(&mut buf);
            let _ = s.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 2\r\n\r\nno");
        }
    });
    let url = format!("http://localhost:{port}");
    let run = |flags: &[&str]| {
        let mut args = vec!["--no-proxy"];
        args.extend_from_slice(flags);
        args.extend_from_slice(&["tunnel", "--hello-world", "--service-url", &url]);
        let out = Command::new(env!("CARGO_BIN_EXE_podnet"))
            .args(&args)
            .env_remove("RUST_LOG")
            .env_remove("PODNET_ERROR_PROGRAM")
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stderr).into_owned()
    };
    let json = run(&["--log-format", "json", "-v"]);
    let lines: Vec<&str> = json.lines().filter(|l| l.starts_with('{')).collect();
    assert!(!lines.is_empty(), "{json}");
    for line in lines {
        let value: serde_json::Value = serde_json::from_str(line).unwrap();
        assert!(value["level"].is_string(), "{line}");
    }
    assert!(run(&["-v"]).contains("DEBUG"));
    let quiet = run(&["-q"]);
    assert!(
        !quiet.contains("DEBUG") && !quiet.contains(" INFO "),
        "{quiet}"
    );
    assert!(quiet.contains("HTTP 403"), "the error line stays: {quiet}");
}

/// Issue 181: with --log-format json, each line of stderr is JSON, also
/// the last error line and a usage error.
#[test]
fn log_format_json_gives_json_lines_only() {
    for args in [
        &[
            "--log-format",
            "json",
            "tunnel",
            "--url",
            "http://127.0.0.1:1",
            "--check",
        ][..],
        &["--log-format=json", "tunnel", "--bogus"],
    ] {
        let out = podnet(args);
        assert!(!out.status.success());
        let err = stderr(&out);
        for line in err.lines().filter(|l| !l.trim().is_empty()) {
            let v: serde_json::Value = serde_json::from_str(line)
                .unwrap_or_else(|e| panic!("{args:?}: not JSON ({e}): {line}"));
            assert!(v["level"].is_string(), "{line}");
        }
    }
}
