//! `podnet forward -L udp:... --network turn://...` (T-321): a UDP flow goes
//! through an allocation on a TURN server. The server is the stand-in of
//! `src/networks/turn_mock.rs` on the loopback: no account and no public
//! server. The account comes from the environment, never from `argv`.

mod test_host;

#[path = "../src/networks/turn_mock.rs"]
mod mock;

use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use pod_turn::stun::{ALLOCATE, CHANNEL_BIND};

/// The account variables are removed first, so that the host's own do not
/// reach the command.
fn forward(args: &[&str]) -> Command {
    podnet(&["forward"], args)
}

fn podnet(sub: &[&str], args: &[&str]) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_podnet"));
    cmd.args(sub).args(args);
    for name in [
        "PODNET_TURN_USER",
        "PODNET_TURN_PASSWORD",
        "PODNET_TURN_PASSWORD_FILE",
        "PODNET_TURN_REALM",
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

/// A password file in a directory of its own.
fn password_file(dir: &tempfile::TempDir, password: &str) -> String {
    let path = dir.path().join("turn-password");
    std::fs::write(&path, format!("{password}\n")).unwrap();
    path.to_str().unwrap().to_string()
}

fn free_udp_port() -> u16 {
    std::net::UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_udp_forward_goes_through_a_turn_allocation() {
    test_host::need_loopback!();
    let (turn, peer) = (mock::start().await, mock::echo_peer().await);
    let dir = tempfile::tempdir().unwrap();
    let file = password_file(&dir, mock::PASSWORD);
    let port = free_udp_port();
    let listen = format!("udp:127.0.0.1:{port}={peer}");
    let network = format!("turn://{}", turn.addr);
    let mut cmd = forward(&["-L", &listen, "--network", &network, "--run-for", "60"]);
    cmd.env("PODNET_TURN_USER", mock::USER)
        .env("PODNET_TURN_PASSWORD_FILE", &file)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = Proc(cmd.spawn().unwrap());
    let mut stdout = BufReader::new(child.0.stdout.take().unwrap());
    let mut first = String::new();
    stdout.read_line(&mut first).unwrap();
    assert!(first.starts_with("podnet: forward"), "{first:?}");
    assert!(
        first.contains(&format!("via turn://{}", turn.addr)),
        "{first:?}"
    );

    let client = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    for text in ["first", "second"] {
        client
            .send_to(text.as_bytes(), ("127.0.0.1", port))
            .unwrap();
        let mut buf = [0u8; 256];
        let n = client.recv(&mut buf).expect("an answer through TURN");
        assert_eq!(&buf[..n], format!("echo:{text}").as_bytes());
    }
    // The probe of `up` and the flow: two allocations, one channel.
    assert_eq!(turn.count(ALLOCATE, 0), 2, "{:?}", turn.seen());
    assert_eq!(turn.count(CHANNEL_BIND, 0), 1, "{:?}", turn.seen());
    assert_eq!(turn.control.lock().unwrap().channel_data, 2);

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
        !output.contains(mock::PASSWORD),
        "the password is in the output: {output}"
    );
}

#[test]
fn a_turn_network_with_no_account_is_a_usage_error_that_names_the_variables() {
    // No loopback needed: the account is read before any connection.
    let out = forward(&[
        "-L",
        "udp:127.0.0.1:5399=127.0.0.1:9",
        "--network",
        "turn://127.0.0.1:9",
    ])
    .output()
    .unwrap();
    assert_eq!(out.status.code(), Some(64), "{out:?}");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("PODNET_TURN_USER"), "{err}");
}

#[test]
fn a_turn_spec_with_a_password_in_it_is_refused_and_not_echoed() {
    let out = forward(&[
        "-L",
        "udp:127.0.0.1:5399=127.0.0.1:9",
        "--network",
        "turn://user:hunter2@127.0.0.1:9",
    ])
    .output()
    .unwrap();
    assert_eq!(out.status.code(), Some(64), "{out:?}");
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(all.contains("PODNET_TURN_PASSWORD_FILE"), "{all}");
    assert!(!all.contains("hunter2"), "{all}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_turn_wrong_password_ends_the_command_at_the_allocate_stage() {
    test_host::need_loopback!();
    let turn = mock::start().await;
    let dir = tempfile::tempdir().unwrap();
    let file = password_file(&dir, "not-the-password-xyz");
    let network = format!("turn://{}", turn.addr);
    let out = forward(&[
        "-L",
        "udp:127.0.0.1:5399=127.0.0.1:9",
        "--network",
        &network,
        "--run-for",
        "30",
    ])
    .env("PODNET_TURN_USER", mock::USER)
    .env("PODNET_TURN_PASSWORD_FILE", &file)
    .output()
    .unwrap();
    assert!(!out.status.success(), "{out:?}");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("allocate") && err.contains("401"), "{err}");
    assert!(
        !err.contains("not-the-password-xyz") && !err.contains(mock::PASSWORD),
        "{err}"
    );
}

/// The row of `network` in `net doctor --json --network`, with this password.
fn doctor_row(network: &str, file: &str) -> serde_json::Value {
    let out = podnet(&["net", "doctor"], &["--json", "--network", network])
        .env("PODNET_TURN_USER", mock::USER)
        .env("PODNET_TURN_PASSWORD_FILE", file)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        !text.contains(mock::PASSWORD) && !text.contains("not-the-password-xyz"),
        "{text}"
    );
    let doc: serde_json::Value = serde_json::from_str(&text).expect("one JSON document");
    let name = format!("network {network}");
    doc["probes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["probe"] == name.as_str())
        .cloned()
        .unwrap_or_else(|| panic!("no row of the network: {text}"))
}

/// `net doctor --network turn://...` (T-321): the row is `ok` when the
/// allocation comes up, and `fail` with the stage when it does not. No
/// password reaches the document.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn net_doctor_has_a_row_for_a_turn_network_that_names_the_stage() {
    test_host::need_loopback!();
    let turn = mock::start().await;
    let dir = tempfile::tempdir().unwrap();
    let good = password_file(&dir, mock::PASSWORD);
    let network = format!("turn://{}", turn.addr);
    let n = network.clone();
    let row = tokio::task::spawn_blocking(move || doctor_row(&n, &good))
        .await
        .unwrap();
    assert_eq!(row["status"], "ok", "{row}");
    let dir2 = tempfile::tempdir().unwrap();
    let bad = password_file(&dir2, "not-the-password-xyz");
    let row = tokio::task::spawn_blocking(move || doctor_row(&network, &bad))
        .await
        .unwrap();
    assert_eq!(row["status"], "fail", "{row}");
    let detail = row["detail"].as_str().unwrap();
    assert!(detail.contains("allocate"), "{row}");
}
