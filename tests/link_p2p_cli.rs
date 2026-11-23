//! GitHub #485: `podnet link p2p` takes the direct path first and the TURN
//! relay when the direct path is refused or times out. The TURN server is
//! the stand-in of `src/networks/turn_mock.rs` on the loopback: no account
//! and no public server. The account comes from the environment, never from
//! `argv`, and it never appears in the output.

mod test_host;

#[path = "../src/networks/turn_mock.rs"]
mod mock;

use std::process::Stdio;

use tokio::io::AsyncReadExt as _;
use tokio::net::TcpListener;

/// A podnet command with the account variables of the host removed, so that
/// only the values of the test reach it.
fn podnet(args: &[&str]) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_podnet"));
    cmd.args(["link", "p2p"]).args(args);
    for name in [
        "PODNET_TURN_USER",
        "PODNET_TURN_PASSWORD",
        "PODNET_TURN_PASSWORD_FILE",
        "PODNET_TURN_REALM",
        "PODNET_TURN_LIFETIME",
        "PODNET_TURN_RATE_LIMIT",
        "PODNET_PROXY",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "RUST_LOG",
    ] {
        cmd.env_remove(name);
    }
    cmd.kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd
}

/// A TCP peer that accepts one stream and holds it.
async fn peer() -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        if let Ok((mut stream, _)) = listener.accept().await {
            let mut byte = [0u8; 1];
            let _ = stream.read(&mut byte).await;
        }
    });
    (addr, task)
}

/// A password file in a directory of its own.
fn password_file(dir: &tempfile::TempDir) -> String {
    let path = dir.path().join("turn-password");
    std::fs::write(&path, format!("{}\n", mock::PASSWORD)).unwrap();
    path.to_str().unwrap().to_string()
}

/// The text of a finished command, and its exit code.
struct Done {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

impl Done {
    fn text(&self) -> String {
        format!("{}{}", self.stdout, self.stderr)
    }
}

async fn run(cmd: &mut tokio::process::Command) -> Done {
    let out = cmd.output().await.expect("the podnet binary runs");
    Done {
        code: out.status.code(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

/// TURN-7: a peer that answers the direct path takes no relay. The relay is
/// named in the command, and it is not used.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn link_p2p_prefers_the_direct_path() {
    test_host::need_loopback!();
    let (peer_addr, _held) = peer().await;
    let turn = mock::start().await;
    let dir = tempfile::tempdir().unwrap();
    let relay = format!("turn://{}", turn.addr);
    let mut cmd = podnet(&[
        "--peer",
        &peer_addr.to_string(),
        "--relay",
        &relay,
        "--check",
    ]);
    cmd.env("PODNET_TURN_USER", mock::USER)
        .env("PODNET_TURN_PASSWORD_FILE", password_file(&dir));
    let done = run(&mut cmd).await;
    assert_eq!(done.code, Some(0), "{}", done.text());
    assert!(
        done.stdout.contains("path        direct"),
        "{}",
        done.text()
    );
    assert!(
        !done.stdout.contains("path        relay"),
        "{}",
        done.text()
    );
    assert!(
        !done.stdout.contains("lifetime"),
        "a direct path has no allocation"
    );
}

/// A refused direct path moves to the relay. The path, the failure, the
/// lifetime and the rate limit are printed, and the credential is not.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn link_p2p_falls_back_to_the_relay_and_prints_its_allocation() {
    test_host::need_loopback!();
    let (peer_addr, _held) = peer().await;
    let turn = mock::start().await;
    let dir = tempfile::tempdir().unwrap();
    let relay = format!("turn://{}", turn.addr);
    // Nothing listens on port 1: the direct path is refused at once.
    let mut cmd = podnet(&[
        "--peer",
        &peer_addr.to_string(),
        "--direct",
        "127.0.0.1:1",
        "--relay",
        &relay,
        "--timeout",
        "1000",
        "--check",
    ]);
    cmd.env("PODNET_TURN_USER", mock::USER)
        .env("PODNET_TURN_PASSWORD_FILE", password_file(&dir))
        .env("PODNET_TURN_LIFETIME", "120")
        .env("PODNET_TURN_RATE_LIMIT", "500000");
    let done = run(&mut cmd).await;
    assert_eq!(done.code, Some(0), "{}", done.text());
    assert!(done.stdout.contains("path        relay"), "{}", done.text());
    assert!(
        done.stdout.contains("direct      refused"),
        "{}",
        done.text()
    );
    assert!(done.stdout.contains("lifetime    120 s"), "{}", done.text());
    assert!(
        done.stdout.contains("rate limit  500000 B/s"),
        "{}",
        done.text()
    );
    assert!(
        !done.text().contains(mock::PASSWORD),
        "the credential is in the output: {}",
        done.text()
    );
}

/// The same fallback as one JSON document, with the allocation.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn link_p2p_json_names_the_relay_and_the_allocation() {
    test_host::need_loopback!();
    let (peer_addr, _held) = peer().await;
    let turn = mock::start().await;
    let dir = tempfile::tempdir().unwrap();
    let relay = format!("turn://{}", turn.addr);
    let mut cmd = podnet(&[
        "--peer",
        &peer_addr.to_string(),
        "--direct",
        "127.0.0.1:1",
        "--relay",
        &relay,
        "--timeout",
        "1000",
        "--check",
        "--json",
    ]);
    cmd.env("PODNET_TURN_USER", mock::USER)
        .env("PODNET_TURN_PASSWORD_FILE", password_file(&dir))
        .env("PODNET_TURN_LIFETIME", "120")
        .env("PODNET_TURN_RATE_LIMIT", "500000");
    let done = run(&mut cmd).await;
    assert_eq!(done.code, Some(0), "{}", done.text());
    let doc: serde_json::Value =
        serde_json::from_str(done.stdout.trim()).expect("one JSON document");
    assert_eq!(doc["path"], "relay", "{doc}");
    assert_eq!(doc["direct_failure"], "refused", "{doc}");
    assert_eq!(doc["lifetime_s"], 120, "{doc}");
    assert_eq!(doc["rate_limit"], 500000, "{doc}");
    assert!(
        !done.text().contains(mock::PASSWORD),
        "the credential is in the document: {}",
        done.text()
    );
}

/// TURN-7 and the named skip: a host error that names UDP is the host's
/// refusal, printed with its own words. This is the composition of the
/// command, without a socket of the test.
#[test]
fn a_host_that_refuses_udp_is_a_named_skip() {
    let err = anyhow::anyhow!(
        "turn://127.0.0.1:3478: allocate: TURN: no UDP socket for TURN: \
         Operation not permitted (os error 1)"
    );
    let line = podnet::link_p2p::skip_line(&format!("{err:#}"));
    assert!(
        line.starts_with("path relay skipped: the host refuses UDP: "),
        "{line}"
    );
    assert!(
        line.contains("no UDP socket for TURN: Operation not permitted"),
        "{line}"
    );
    let fault = podnet::exit::fault(podnet::exit::Fault::Unavailable, anyhow::anyhow!(line));
    assert_eq!(podnet::exit::code_of(&fault), 69);
}

/// A refused direct path with no relay asks for the flag: the command does
/// not invent a relay.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn link_p2p_names_the_relay_flag_when_no_relay_is_given() {
    test_host::need_loopback!();
    let mut cmd = podnet(&["--peer", "127.0.0.1:1", "--timeout", "1000", "--check"]);
    let done = run(&mut cmd).await;
    assert_eq!(done.code, Some(77), "{}", done.text());
    assert!(done.stderr.contains("--relay"), "{}", done.text());
}
