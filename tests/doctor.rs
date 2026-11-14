//! `podnet doctor` against a local test proxy that refuses, carries nothing,
//! or terminates TLS with its own CA (issue 11). No test uses the network.

mod test_host;

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

/// What the test proxy does with one CONNECT.
#[derive(Clone, Copy)]
enum Action {
    /// 403 with a body.
    Refuse,
    /// 200, then no byte.
    Silent,
    /// 200, then TLS with the certificate of the test CA.
    Terminate,
    /// 200 after 8 s, then no byte: the connect is slow and the TLS never
    /// starts.
    Slow,
}

const PASSWORD: &str = "doctor-secret-pw";

fn tls_config() -> Arc<rustls::ServerConfig> {
    let ca_key = rcgen::KeyPair::generate().unwrap();
    let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "podnet doctor test CA");
    let issuer = rcgen::Issuer::new(params, ca_key);
    let leaf_key = rcgen::KeyPair::generate().unwrap();
    let leaf = rcgen::CertificateParams::new(vec!["api.trycloudflare.com".to_string()])
        .unwrap()
        .signed_by(&leaf_key, &issuer)
        .unwrap();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![rustls::pki_types::CertificateDer::from(leaf.der().to_vec())],
            rustls::pki_types::PrivateKeyDer::Pkcs8(leaf_key.serialize_der().into()),
        )
        .unwrap();
    config.alpn_protocols = vec![b"h2".to_vec()];
    Arc::new(config)
}

/// A CONNECT proxy on a local port; `rule` chooses the action for each
/// authority. Gives the port.
fn proxy(rule: fn(&str) -> Action) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let config = tls_config();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let config = config.clone();
            std::thread::spawn(move || handle(stream, rule, config));
        }
    });
    port
}

fn handle(mut stream: TcpStream, rule: fn(&str) -> Action, config: Arc<rustls::ServerConfig>) {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        match stream.read(&mut byte) {
            Ok(1) => head.push(byte[0]),
            _ => return,
        }
    }
    let text = String::from_utf8_lossy(&head);
    let authority = text
        .lines()
        .next()
        .and_then(|l| l.split(' ').nth(1))
        .unwrap_or("")
        .to_string();
    match rule(&authority) {
        Action::Refuse => {
            let body = "port not on the allowlist";
            let _ = write!(
                stream,
                "HTTP/1.1 403 Forbidden\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
        }
        Action::Silent => {
            let _ = stream.write_all(b"HTTP/1.1 200 Connection established\r\n\r\n");
            std::thread::sleep(Duration::from_secs(5));
        }
        Action::Slow => {
            std::thread::sleep(Duration::from_secs(8));
            let _ = stream.write_all(b"HTTP/1.1 200 Connection established\r\n\r\n");
            std::thread::sleep(Duration::from_secs(30));
        }
        Action::Terminate => {
            let _ = stream.write_all(b"HTTP/1.1 200 Connection established\r\n\r\n");
            let conn = rustls::ServerConnection::new(config).unwrap();
            let mut tls = rustls::StreamOwned::new(conn, stream);
            let mut buf = [0u8; 1024];
            let _ = tls.read(&mut buf);
        }
    }
}

/// Run `podnet doctor --json` through the proxy on `port`. Gives the exit code,
/// the JSON document and all the output.
fn doctor(port: u16) -> (Option<i32>, serde_json::Value, String) {
    doctor_with(port, &["--samples", "2", "--timeout", "1"])
}

/// As [`doctor`], with the given arguments after `doctor --json`.
fn doctor_with(port: u16, extra: &[&str]) -> (Option<i32>, serde_json::Value, String) {
    doctor_with_env(port, extra, std::path::Path::new("/nonexistent/podnet-run"))
}

/// As [`doctor_with`], with the run directory (the instances) given.
fn doctor_with_env(
    port: u16,
    extra: &[&str],
    run: &std::path::Path,
) -> (Option<i32>, serde_json::Value, String) {
    doctor_via(
        &format!("http://tester:{PASSWORD}@127.0.0.1:{port}"),
        extra,
        run,
        &[],
    )
}

/// As [`doctor_with_env`], through the process proxy `via` (`--proxy`), with
/// the variables `env` set.
fn doctor_via(
    via: &str,
    extra: &[&str],
    run: &std::path::Path,
    env: &[(&str, &str)],
) -> (Option<i32>, serde_json::Value, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_podnet"));
    cmd.env("PODNET_RUN_DIR", run);
    cmd.envs(env.iter().copied());
    cmd.args(["--proxy", via, "doctor", "--json"])
        .args(extra)
        // A TEST-NET address: no answer, and no proxy rule exempts it.
        .env("PODNET_EDGE", "192.0.2.1:7844")
        .env_remove("PODNET_ERROR_PROGRAM")
        .env_remove("SSL_CERT_FILE");
    for key in ["NO_PROXY", "no_proxy"] {
        if !env.iter().any(|(name, _)| *name == key) {
            cmd.env_remove(key);
        }
    }
    let out = cmd.output().unwrap();
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !all.contains(PASSWORD),
        "the proxy password is in the output"
    );
    let doc: serde_json::Value =
        serde_json::from_slice(&out.stdout).unwrap_or_else(|e| panic!("not JSON ({e}):\n{all}"));
    (out.status.code(), doc, all)
}

fn leg<'a>(doc: &'a serde_json::Value, name: &str) -> &'a serde_json::Value {
    doc["legs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["leg"] == name)
        .unwrap()
}

fn path<'a>(doc: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
    doc["paths"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == id)
        .unwrap()
}

#[test]
fn a_proxy_that_refuses_7844_and_terminates_tls() {
    test_host::need_loopback!();
    let port = proxy(|authority| {
        if authority.ends_with(":7844") {
            Action::Refuse
        } else {
            Action::Terminate
        }
    });
    let (code, doc, all) = doctor(port);
    assert_eq!(code, Some(1), "{all}");
    assert_eq!(doc["ok"], false);
    assert_eq!(doc["proxy"]["credentials"], true);
    let http2 = leg(&doc, "http2");
    assert_eq!(http2["verdict"], "fail", "{all}");
    assert_eq!(http2["status"], "fail", "{all}");
    assert!(
        http2["reason"].as_str().unwrap().contains("proxy 403"),
        "{all}"
    );
    assert_eq!(leg(&doc, "quic")["verdict"], "fail");
    // The TLS issuer of the proxy is named.
    let api = path(&doc, "api");
    assert_eq!(api["verdict"], "ok", "{all}");
    assert_eq!(api["tls"]["verdict"], "fail", "{all}");
    assert!(api["tls"]["issuer"]
        .as_str()
        .unwrap()
        .contains("podnet doctor test CA"));
    assert!(api["tls"]["detail"]
        .as_str()
        .unwrap()
        .contains("a proxy terminates TLS"));
    // Samples: a refusal is an answer, not a loss.
    let edge = path(&doc, "edge-address");
    assert_eq!(edge["samples"]["loss"], 0.0, "{all}");
    assert!(edge["samples"]["best_ms"].is_number());
    // UDP to an address that does not answer is never ok.
    assert_ne!(doc["udp"]["verdict"], "answered");
}

#[test]
fn a_proxy_that_allows_names_and_refuses_addresses() {
    test_host::need_loopback!();
    let port = proxy(|authority| {
        let host = authority.rsplit_once(':').map(|(h, _)| h).unwrap_or("");
        if host.parse::<std::net::IpAddr>().is_ok() {
            Action::Refuse
        } else {
            Action::Terminate
        }
    });
    let (_, doc, all) = doctor(port);
    assert_eq!(
        doc["names"],
        "the proxy allows names and refuses IP addresses; the edge legs dial addresses",
        "{all}"
    );
    assert_eq!(path(&doc, "edge-name")["verdict"], "ok");
    assert_eq!(path(&doc, "edge-address")["verdict"], "fail");
}

#[test]
fn a_proxy_that_answers_200_and_carries_nothing() {
    test_host::need_loopback!();
    let port = proxy(|_| Action::Silent);
    let (code, doc, all) = doctor(port);
    assert_eq!(code, Some(1));
    let api = path(&doc, "api");
    assert_eq!(api["verdict"], "ok", "{all}");
    assert!(
        api["tls"]["detail"]
            .as_str()
            .unwrap()
            .contains("the proxy answers and carries nothing"),
        "{all}"
    );
}

/// The sorted keys of a JSON object.
fn keys(v: &serde_json::Value) -> Vec<&str> {
    let mut keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort_unstable();
    keys
}

/// T-216: the keys of `doctor --json` are the keys of schema 1
/// (`docs/protocol.md`, section 9). A key is added, never removed or renamed.
#[test]
fn doctor_json_keys_are_fixed() {
    test_host::need_loopback!();
    let port = proxy(|authority| {
        if authority.ends_with(":7844") {
            Action::Refuse
        } else {
            Action::Terminate
        }
    });
    let (_, doc, all) = doctor_with(port, &["--quick", "--timeout", "1"]);
    assert_eq!(doc["schema"], 1, "{all}");
    assert_eq!(doc["quick"], true, "{all}");
    assert_eq!(
        keys(&doc),
        [
            "carriers",
            "clock",
            "git",
            "lanes",
            "legs",
            "names",
            "next",
            "ok",
            "paths",
            "podnet",
            "proxy",
            "quick",
            "roads",
            "schema",
            "ssh_relay",
            "udp"
        ]
    );
    assert_eq!(
        keys(&doc["clock"]),
        ["detail", "skew_s", "status", "verdict"]
    );
    assert_eq!(
        keys(&doc["git"]),
        ["detail", "status", "verdict", "version"]
    );
    assert_eq!(
        keys(&doc["proxy"]),
        ["credentials", "direct", "text", "url"]
    );
    assert_eq!(keys(&doc["udp"]), ["detail", "status", "target", "verdict"]);
    assert_eq!(
        keys(&doc["ssh_relay"]),
        [
            "auth",
            "auth_method",
            "banner",
            "connect_ms",
            "detail",
            "proxy",
            "relay",
            "source",
            "status",
            "total_ms",
            "user",
            "verdict"
        ]
    );
    for road in doc["roads"].as_array().unwrap() {
        assert_eq!(keys(road), ["reason", "road", "status", "verdict"]);
    }
    for leg in doc["legs"].as_array().unwrap() {
        assert_eq!(keys(leg), ["leg", "reason", "status", "verdict"]);
    }
    let paths = doc["paths"].as_array().unwrap();
    assert!(paths.len() >= 5, "{all}");
    for p in paths {
        assert_eq!(
            keys(p),
            ["detail", "dns", "id", "samples", "status", "target", "tls", "verdict", "via"]
        );
        assert_eq!(
            keys(&p["samples"]),
            [
                "answered",
                "best_ms",
                "jitter_ms",
                "loss",
                "median_ms",
                "p95_ms",
                "sent"
            ]
        );
    }
    for lane in doc["lanes"].as_object().unwrap().values() {
        assert_eq!(keys(lane), ["ms", "reason", "verdict"]);
    }
    // A path with a TLS result has these keys.
    let tls = &path(&doc, "api")["tls"];
    assert_eq!(
        keys(tls),
        ["detail", "issuer", "status", "verdict"],
        "{all}"
    );
}

/// T-216: `--quick` ends in the limit of 15 s. The fake proxy connects in 8 s
/// and then never speaks TLS; with `--timeout 10` the path alone needs 18 s.
#[test]
fn doctor_quick_ends_in_the_limit() {
    test_host::need_loopback!();
    let port = proxy(|_| Action::Slow);
    let begin = std::time::Instant::now();
    let (_, doc, all) = doctor_with(port, &["--quick", "--timeout", "10"]);
    let took = begin.elapsed();
    assert!(took < Duration::from_secs(17), "{took:?}\n{all}");
    assert!(took >= Duration::from_secs(14), "{took:?}");
    assert_eq!(doc["quick"], true);
    let api = path(&doc, "api");
    assert_eq!(api["verdict"], "unknown", "{all}");
    assert!(
        api["detail"].as_str().unwrap().contains("time limit"),
        "{all}"
    );
    for leg in doc["legs"].as_array().unwrap() {
        assert_ne!(leg["verdict"], "ok", "{all}");
    }
    assert!(doc["udp"]["detail"].is_string());
}

/// T-217: `lanes` holds the seven egress lanes, each `ok`, `fail` or
/// `unknown`, with the time.
#[test]
fn doctor_json_has_the_seven_lanes() {
    test_host::need_loopback!();
    let port = proxy(|authority| {
        if authority.ends_with(":7844") {
            Action::Refuse
        } else {
            Action::Terminate
        }
    });
    let (_, doc, all) = doctor_with(port, &["--quick", "--timeout", "1"]);
    let lanes = doc["lanes"].as_object().unwrap();
    let mut names: Vec<&str> = lanes.keys().map(String::as_str).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "masque_tcp443",
            "masque_udp443",
            "proxy",
            "tunnel_http2",
            "tunnel_quic",
            "tunnel_quic_warp",
            "ws_upgrade"
        ],
        "{all}"
    );
    for (name, lane) in lanes {
        let verdict = lane["verdict"].as_str().unwrap();
        assert!(
            ["ok", "fail", "unknown"].contains(&verdict),
            "{name}: {all}"
        );
        assert!(lane["ms"].is_null() || lane["ms"].is_number(), "{name}");
        assert!(lane["reason"].is_string(), "{name}");
    }
    // The proxy refuses 7844 and carries no UDP.
    assert_eq!(lanes["tunnel_http2"]["verdict"], "fail", "{all}");
    assert_eq!(lanes["tunnel_quic"]["verdict"], "fail", "{all}");
    // A proxy is set, and the paths through it answer TCP: the time is set.
    assert_ne!(lanes["proxy"]["reason"], "no proxy is set", "{all}");
    // T-260: a proxy carries no UDP, so the lane is unknown with that reason.
    assert_eq!(lanes["masque_udp443"]["verdict"], "unknown", "{all}");
    assert!(
        lanes["masque_udp443"]["reason"]
            .as_str()
            .unwrap()
            .contains("no proxy carries UDP"),
        "{all}"
    );
    // The upgrade goes through the proxy, which refuses 7844.
    assert_eq!(lanes["ws_upgrade"]["verdict"], "fail", "{all}");
}

/// T-349: `podnet` with the arguments `args` and the variables `env`, with
/// no proxy variable of the host. Gives the exit code and stdout with stderr.
fn podnet_with(args: &[&str], env: &[(&str, &str)]) -> (Option<i32>, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_podnet"));
    for key in [
        "HTTPS_PROXY",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
        "HTTP_PROXY",
        "http_proxy",
        "NO_PROXY",
        "no_proxy",
        "PODNET_PROXY",
        "PODNET_ERROR_PROGRAM",
    ] {
        cmd.env_remove(key);
    }
    let out = cmd
        .args(args)
        .envs(env.iter().copied())
        .env("PODNET_RUN_DIR", "/nonexistent/podnet-run")
        .env("PODNET_EDGE", "192.0.2.1:7844")
        .output()
        .unwrap();
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.code(), all)
}

/// T-349 (fuzz run 43590953876): a proxy on a Unix socket with a user and a
/// password in its path is refused with exit 64 as `--proxy`, and passed
/// over as a variable. No line of `podnet doctor` holds the password. A
/// normal Unix path still works.
#[test]
fn doctor_a_unix_proxy_with_user_info_shows_no_password() {
    let doctor = ["doctor", "--json", "--samples", "1", "--timeout", "1"];
    let spec = "socks5h+unix://user:hunter2@x";
    let mut args = vec!["--proxy", spec];
    args.extend(doctor);
    let (code, all) = podnet_with(&args, &[]);
    assert_eq!(code, Some(64), "{all}");
    assert!(!all.contains("hunter2"), "{all}");
    for var in ["HTTPS_PROXY", "ALL_PROXY"] {
        let (_, all) = podnet_with(&doctor, &[(var, spec)]);
        assert!(!all.contains("hunter2"), "{var}: {all}");
        let (_, all) = podnet_with(&["status", "--json"], &[(var, spec)]);
        assert!(!all.contains("hunter2"), "{var}: {all}");
    }
    let mut args = vec!["--proxy", "socks5h+unix:/run/podnet-test-none.sock"];
    args.extend(doctor);
    let (_, all) = podnet_with(&args, &[]);
    assert!(
        all.contains("socks5h+unix:/run/podnet-test-none.sock"),
        "{all}"
    );
}
