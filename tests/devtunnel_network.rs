//! `podnet forward -L ... --network devtunnel://TUNNEL` (T-320): a TCP
//! stream goes through a Dev Tunnels relay to a port of the tunnel host. The
//! relay is the stand-in of `pod-devtunnel` on the loopback: no account and
//! no live service. The management token comes from the environment, never
//! from `argv`.

mod test_host;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use pod_devtunnel::standin::{Plan, Standin};

const TOKEN: &str = "made-up-user-token-of-the-tests";

/// The token variables are removed first, so that the host's own do not
/// reach the command.
fn forward(args: &[&str]) -> Command {
    podnet(&["forward"], args)
}

fn podnet(sub: &[&str], args: &[&str]) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_podnet"));
    cmd.args(sub).args(args);
    for name in [
        "PODNET_DEVTUNNEL_TOKEN",
        "PODNET_DEVTUNNEL_TOKEN_FILE",
        "PODNET_DEVTUNNEL_SCHEME",
        "PODNET_DEVTUNNEL_MANAGEMENT",
        "PODNET_PROXY",
        "RUST_LOG",
    ] {
        cmd.env_remove(name);
    }
    cmd
}

struct Proc(Child);

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn free_tcp_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_tcp_forward_reaches_the_tunnel_host_through_the_stand_in() {
    test_host::need_loopback!();
    let standin = Standin::start(Plan::echo("github", TOKEN)).await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("token");
    std::fs::write(&file, format!("{TOKEN}\n")).unwrap();
    let port = free_tcp_port();
    let listen = format!("127.0.0.1:{port}=127.0.0.1:22");
    let mut cmd = forward(&[
        "-L",
        &listen,
        "--network",
        "devtunnel://standin-box",
        "--run-for",
        "60",
    ]);
    cmd.env("PODNET_DEVTUNNEL_TOKEN_FILE", &file)
        .env("PODNET_DEVTUNNEL_MANAGEMENT", standin.base())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = Proc(cmd.spawn().unwrap());
    let mut stdout = BufReader::new(child.0.stdout.take().unwrap());
    let mut first = String::new();
    stdout.read_line(&mut first).unwrap();
    assert!(first.starts_with("podnet: forward"), "{first:?}");
    assert!(first.contains("via devtunnel://standin-box"), "{first:?}");

    for text in ["first", "second"] {
        let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(15)))
            .unwrap();
        client.write_all(text.as_bytes()).unwrap();
        let mut back = vec![0u8; text.len()];
        client
            .read_exact(&mut back)
            .expect("an echo through the relay");
        assert_eq!(back, text.as_bytes());
    }
    let seen = standin.seen();
    assert_eq!(seen.channels.len(), 2, "{seen:?}");
    assert!(seen.user_ok.iter().all(|ok| *ok), "{seen:?}");

    let _ = child.0.kill();
    let mut output = first;
    stdout.read_to_string(&mut output).unwrap();
    let mut err = String::new();
    child
        .0
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut err)
        .unwrap();
    output.push_str(&err);
    assert!(
        !output.contains(TOKEN) && !output.contains("standin-connect-"),
        "a token is in the output: {output}"
    );
}

#[test]
fn a_devtunnel_network_with_no_token_is_a_usage_error_that_names_the_variables() {
    // No loopback needed: the account is read before any connection.
    let out = forward(&[
        "-L",
        "127.0.0.1:5399=127.0.0.1:22",
        "--network",
        "devtunnel://lab",
    ])
    .output()
    .unwrap();
    assert_eq!(out.status.code(), Some(64), "{out:?}");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("PODNET_DEVTUNNEL_TOKEN_FILE"), "{err}");
}

#[test]
fn a_devtunnel_spec_with_a_token_in_it_is_refused_and_not_echoed() {
    let out = forward(&[
        "-L",
        "127.0.0.1:5399=127.0.0.1:22",
        "--network",
        "devtunnel://hunter2@lab",
    ])
    .output()
    .unwrap();
    assert_eq!(out.status.code(), Some(64), "{out:?}");
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(all.contains("PODNET_DEVTUNNEL_TOKEN_FILE"), "{all}");
    assert!(!all.contains("hunter2"), "{all}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_wrong_token_ends_the_command_at_the_management_step() {
    test_host::need_loopback!();
    let standin = Standin::start(Plan::echo("github", TOKEN)).await.unwrap();
    let out = forward(&[
        "-L",
        "127.0.0.1:5399=127.0.0.1:22",
        "--network",
        "devtunnel://standin-box",
        "--run-for",
        "30",
    ])
    .env("PODNET_DEVTUNNEL_TOKEN", "a-different-made-up-token")
    .env("PODNET_DEVTUNNEL_MANAGEMENT", standin.base())
    .output()
    .unwrap();
    assert!(!out.status.success(), "{out:?}");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("Dev Tunnels management API"), "{err}");
    assert!(!err.contains("a-different-made-up-token"), "{err}");
}

/// The row of `devtunnel://standin-box` in `net doctor --json --network`.
fn doctor_row(base: &str, token: &str) -> serde_json::Value {
    let out = podnet(
        &["net", "doctor"],
        &["--json", "--network", "devtunnel://standin-box"],
    )
    .env("PODNET_DEVTUNNEL_TOKEN", token)
    .env("PODNET_DEVTUNNEL_MANAGEMENT", base)
    .output()
    .unwrap();
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(!text.contains(token), "{text}");
    let doc: serde_json::Value = serde_json::from_str(&text).expect("one JSON document");
    doc["probes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["probe"] == "network devtunnel://standin-box")
        .cloned()
        .unwrap_or_else(|| panic!("no row of the network: {text}"))
}

/// `net doctor --network SPEC` (T-320): the row of the network is `ok` when
/// it comes up, and `fail` with the stage when it does not. No token reaches
/// the document.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn net_doctor_has_a_row_for_a_devtunnel_network_that_names_the_stage() {
    test_host::need_loopback!();
    let standin = Standin::start(Plan::echo("github", TOKEN)).await.unwrap();
    let base = standin.base().to_string();
    let b = base.clone();
    let good = tokio::task::spawn_blocking(move || doctor_row(&b, TOKEN))
        .await
        .unwrap();
    assert_eq!(good["status"], "ok", "{good}");
    let bad = tokio::task::spawn_blocking(move || doctor_row(&base, "a-different-made-up-token"))
        .await
        .unwrap();
    assert_eq!(bad["status"], "fail", "{bad}");
    let detail = bad["detail"].as_str().unwrap();
    assert!(detail.contains("Dev Tunnels management API"), "{bad}");
}
