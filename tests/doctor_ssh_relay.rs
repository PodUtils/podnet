//! The SSH relay row of `podnet doctor` (T-165, T-322, T-318): the probe of a
//! relay on loopback, with and without a proxy. No test uses the network.

mod test_host;

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::sync::Arc;

const PASSWORD: &str = "doctor-secret-pw";

/// A port with no listener: a proxy of the process that carries nothing.
fn dead_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// `podnet doctor --json` through the process proxy `via` (`--proxy`), with
/// the variables `env` set, and the arguments `extra`.
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

fn doctor_with_env(
    port: u16,
    extra: &[&str],
    run: &std::path::Path,
) -> (Option<i32>, serde_json::Value, String) {
    doctor_via(&creds_proxy(port), extra, run, &[])
}

/// A relay on loopback that takes any key. Gives its address.
fn fake_ssh_relay() -> String {
    use russh::keys::ssh_key::rand_core::UnwrapErr;
    use russh::keys::ssh_key::Algorithm;
    use russh::server::{Auth, VerifiedPublicKey};
    use ssh_key::getrandom::SysRng;

    struct Conn;
    impl russh::server::Handler for Conn {
        type Error = anyhow::Error;
        async fn auth_publickey(
            &mut self,
            _: &str,
            _: VerifiedPublicKey<'_>,
        ) -> anyhow::Result<Auth> {
            Ok(Auth::Accept)
        }
    }
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async move {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            tx.send(listener.local_addr().unwrap().to_string()).unwrap();
            let key = russh::keys::PrivateKey::random(&mut UnwrapErr(SysRng), Algorithm::Ed25519)
                .unwrap();
            let config = Arc::new(russh::server::Config {
                keys: vec![key],
                ..Default::default()
            });
            while let Ok((stream, _)) = listener.accept().await {
                let config = config.clone();
                tokio::spawn(async move {
                    if let Ok(running) = russh::server::run_stream(config, stream, Conn).await {
                        let _ = running.await;
                    }
                });
            }
        });
    });
    rx.recv().unwrap()
}

// T-165: the row of the SSH relay is unknown when no relay is given.
#[test]
fn doctor_ssh_relay_row_is_unknown_without_a_relay() {
    test_host::need_loopback!();
    let port = dead_port();
    let run = tempfile::tempdir().unwrap();
    let (_, doc, all) = doctor_with_env(port, &["--quick", "--timeout", "1"], run.path());
    assert_eq!(doc["ssh_relay"]["verdict"], "unknown", "{all}");
    assert_eq!(doc["ssh_relay"]["detail"], "no SSH relay is given", "{all}");
}

// T-165: the row probes the relay of the flags.
#[test]
fn doctor_ssh_relay_row_probes_the_relay_of_the_flags() {
    test_host::need_loopback!();
    let port = dead_port();
    let run = tempfile::tempdir().unwrap();
    let address = fake_ssh_relay();
    let (_, doc, all) = doctor_via(
        &creds_proxy(port),
        &[
            "--quick",
            "--timeout",
            "2",
            "--ssh-relay",
            &address,
            "--ssh-user",
            "tester",
        ],
        run.path(),
        // The proxy of the process has credentials and the probe sends none:
        // the relay on loopback is a direct host.
        &[("NO_PROXY", "127.0.0.1")],
    );
    let row = &doc["ssh_relay"];
    assert_eq!(row["verdict"], "ok", "{all}");
    assert_eq!(row["status"], "ok", "{all}");
    assert_eq!(row["relay"], address.as_str(), "{all}");
    assert_eq!(row["user"], "tester", "{all}");
    assert_eq!(row["source"], "flag", "{all}");
    // A direct host: no proxy, in the row and in the text.
    assert_eq!(row["proxy"], serde_json::Value::Null, "{all}");
    assert!(
        row["detail"].as_str().unwrap().contains("proxy none"),
        "{all}"
    );
    assert_eq!(row["auth"], true, "{all}");
    assert_eq!(row["auth_method"], "publickey", "{all}");
    assert!(
        row["banner"].as_str().unwrap().starts_with("SSH-2.0-"),
        "{all}"
    );
    assert!(
        row["connect_ms"].is_number() && row["total_ms"].is_number(),
        "{all}"
    );

    // A port with no listener: fail, at the stage connect.
    let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let dead = free.local_addr().unwrap().to_string();
    drop(free);
    let (_, doc, all) = doctor_via(
        &creds_proxy(port),
        &[
            "--quick",
            "--timeout",
            "2",
            "--ssh-relay",
            &dead,
            "--ssh-user",
            "tester",
        ],
        run.path(),
        // The proxy of the process has credentials and the probe sends none:
        // the relay on loopback is a direct host.
        &[("NO_PROXY", "127.0.0.1")],
    );
    assert_eq!(doc["ssh_relay"]["verdict"], "fail", "{all}");
    assert!(
        doc["ssh_relay"]["detail"]
            .as_str()
            .unwrap()
            .contains("connect failed"),
        "{all}"
    );
}

/// An HTTP CONNECT proxy on loopback for the SSH relay probe (T-322). It
/// joins each CONNECT, whatever the name, to `relay`; with `refuse` it
/// answers that status line instead. Gives its address and the CONNECT lines
/// that it saw.
fn relay_proxy(
    relay: String,
    refuse: Option<&'static str>,
) -> (String, Arc<std::sync::Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let log = seen.clone();
    std::thread::spawn(move || {
        while let Ok((mut client, _)) = listener.accept() {
            let (relay, log) = (relay.clone(), log.clone());
            std::thread::spawn(move || {
                let mut head = Vec::new();
                let mut byte = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") {
                    match client.read(&mut byte) {
                        Ok(1) => head.push(byte[0]),
                        _ => return,
                    }
                }
                let text = String::from_utf8_lossy(&head).to_string();
                log.lock()
                    .unwrap()
                    .push(text.lines().next().unwrap_or_default().to_string());
                if let Some(status) = refuse {
                    let _ = client
                        .write_all(format!("{status}\r\nContent-Length: 0\r\n\r\n").as_bytes());
                    return;
                }
                let Ok(origin) = TcpStream::connect(&relay) else {
                    return;
                };
                let _ = client.write_all(b"HTTP/1.1 200 Connection established\r\n\r\n");
                let (mut up_in, up_out) =
                    (client.try_clone().unwrap(), origin.try_clone().unwrap());
                let up = std::thread::spawn(move || {
                    let _ = std::io::copy(&mut up_in, &mut &up_out);
                });
                let _ = std::io::copy(&mut &origin, &mut &client);
                let _ = up.join();
            });
        }
    });
    (address, seen)
}

/// The doctor with the SSH relay flags `extra`, through the process proxy
/// `via`. Gives the row `ssh_relay` and all the output.
fn ssh_relay_row(via: &str, extra: &[&str], env: &[(&str, &str)]) -> (serde_json::Value, String) {
    let run = tempfile::tempdir().unwrap();
    let mut args = vec!["--quick", "--timeout", "2"];
    args.extend_from_slice(extra);
    let (_, doc, all) = doctor_via(via, &args, run.path(), env);
    (doc["ssh_relay"].clone(), all)
}

fn creds_proxy(port: u16) -> String {
    format!("http://tester:{PASSWORD}@127.0.0.1:{port}")
}

// T-322: the flag `--connect-proxy` carries the probe through an HTTP proxy.
// The name of the relay does not exist: only the proxy can join it.
#[test]
fn doctor_ssh_relay_row_dials_through_the_proxy() {
    test_host::need_loopback!();
    let refusing = dead_port();
    let (proxy_address, seen) = relay_proxy(fake_ssh_relay(), None);
    let (row, all) = ssh_relay_row(
        &creds_proxy(refusing),
        &[
            "--ssh-relay",
            "relay.invalid:443",
            "--ssh-user",
            "tester",
            "--connect-proxy",
            &proxy_address,
        ],
        &[],
    );
    assert_eq!(row["verdict"], "ok", "{all}");
    assert_eq!(row["proxy"], format!("http://{proxy_address}"), "{all}");
    assert!(
        row["detail"]
            .as_str()
            .unwrap()
            .contains(&format!("proxy {proxy_address}")),
        "{all}"
    );
    assert_eq!(
        seen.lock().unwrap().as_slice(),
        ["CONNECT relay.invalid:443 HTTP/1.1"],
        "{all}"
    );
}

// T-322: with no flag the probe uses the proxy of the environment, as the
// other commands do (here the variable HTTPS_PROXY, with `--no-proxy` off).
#[test]
fn doctor_ssh_relay_row_uses_the_proxy_of_the_environment() {
    test_host::need_loopback!();
    let (proxy_address, seen) = relay_proxy(fake_ssh_relay(), None);
    let run = tempfile::tempdir().unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_podnet"));
    cmd.env("PODNET_RUN_DIR", run.path())
        .env("PODNET_EDGE", "192.0.2.1:7844")
        .env("HTTPS_PROXY", format!("http://{proxy_address}"))
        .env("HTTP_PROXY", format!("http://{proxy_address}"))
        .args(["doctor", "--json", "--quick", "--timeout", "2"])
        .args(["--ssh-relay", "relay.invalid:443", "--ssh-user", "tester"]);
    for key in ["NO_PROXY", "no_proxy", "ALL_PROXY", "all_proxy"] {
        cmd.env_remove(key);
    }
    let out = cmd.output().unwrap();
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let row = &doc["ssh_relay"];
    assert_eq!(row["verdict"], "ok", "{doc}");
    assert_eq!(row["proxy"], format!("http://{proxy_address}"), "{doc}");
    // The other rows use the same proxy, so the list holds more lines.
    let seen = seen.lock().unwrap();
    assert!(
        seen.iter()
            .any(|l| l == "CONNECT relay.invalid:443 HTTP/1.1"),
        "{seen:?}"
    );
}

// T-322: a proxy that answers 407 is a refusal at the stage connect, not a
// timeout.
#[test]
fn doctor_ssh_relay_row_shows_a_proxy_that_answers_407_as_a_refusal() {
    test_host::need_loopback!();
    let refusing = dead_port();
    let (proxy_address, _) = relay_proxy(
        "127.0.0.1:9".into(),
        Some("HTTP/1.1 407 Proxy Authentication Required"),
    );
    let (row, all) = ssh_relay_row(
        &creds_proxy(refusing),
        &[
            "--ssh-relay",
            "relay.invalid:443",
            "--ssh-user",
            "tester",
            "--connect-proxy",
            &proxy_address,
        ],
        &[],
    );
    assert_eq!(row["verdict"], "fail", "{all}");
    let detail = row["detail"].as_str().unwrap();
    assert!(
        detail.contains("connect failed") && detail.contains("407"),
        "{all}"
    );
}

// T-322: the proxy of the process has credentials, and the probe sends
// none: the row names the refusal and the probe does not dial.
#[test]
fn doctor_ssh_relay_row_refuses_a_proxy_with_credentials() {
    test_host::need_loopback!();
    let refusing = dead_port();
    let (row, all) = ssh_relay_row(
        &creds_proxy(refusing),
        &["--ssh-relay", "relay.invalid:443", "--ssh-user", "tester"],
        &[],
    );
    assert_eq!(row["verdict"], "fail", "{all}");
    assert!(
        row["detail"]
            .as_str()
            .unwrap()
            .contains("unauthenticated HTTP CONNECT proxy"),
        "{all}"
    );
    assert_eq!(row["proxy"], serde_json::Value::Null, "{all}");
    assert_eq!(row["connect_ms"], serde_json::Value::Null, "{all}");
}

// T-322: a SOCKS5 proxy of the process is a named refusal, and no direct dial.
#[test]
fn doctor_ssh_relay_row_refuses_a_socks5_proxy() {
    test_host::need_loopback!();
    let relay = fake_ssh_relay();
    let (row, all) = ssh_relay_row(
        "socks5h://127.0.0.1:9",
        &["--ssh-relay", &relay, "--ssh-user", "tester"],
        &[],
    );
    // The relay is on loopback and answers, but the probe does not dial it.
    assert_eq!(row["verdict"], "fail", "{all}");
    assert!(
        row["detail"]
            .as_str()
            .unwrap()
            .contains("HTTP CONNECT proxy"),
        "{all}"
    );
    assert_eq!(row["banner"], serde_json::Value::Null, "{all}");
}

// T-322: the flag needs --ssh-relay.
#[test]
fn doctor_connect_proxy_needs_ssh_relay() {
    let out = Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(["doctor", "--connect-proxy", "127.0.0.1:3128"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(64), "{text}");
    assert!(text.contains("--ssh-relay"), "{text}");
    let out = Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(["doctor", "--ssh-relay", "r.example:443", "--ssh-user", "u"])
        .args(["--connect-proxy", "no-port"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(64), "{text}");
    assert!(text.contains("--connect-proxy"), "{text}");
}
