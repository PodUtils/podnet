//! The reverse mode of the relay (issue 88) against a local fake relay:
//! podnet's glue over the reverse road of `pod-relay` (PodUtils/podcrates):
//! the exit codes of the operator and of the node, the next stored pair at
//! the expiry, `relay stop`, and `serve` and `git serve` through a node. The
//! sessions, the drops, the half close and the idle time of the road are the
//! tests of `pod-relay`.

mod test_host;

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use podnet::relay_reverse::{channel, connect, load, run_node, run_node_with, Target};
use podnet::ws_relay::Relay;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

const NAME: &str = "testnode";
/// The tokens that the fake relay takes: made-up values of a test.
const NODE_TOKEN: &str = "testonly-node";
const CONNECT_TOKEN: &str = "testonly-connect";

#[path = "relay_reverse/fake.rs"]
mod fake;
use fake::{fake_relay, State};
#[path = "relay_reverse/resume.rs"]
mod resume;
#[path = "relay_reverse/seal.rs"]
mod seal;

/// The time `ahead` from now, in milliseconds since the epoch.
fn ms_from_now(ahead: Duration) -> u64 {
    (std::time::SystemTime::now() + ahead)
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

/// Write the pair `name` of the relay `url` to `dir` as the store of
/// `pod-relay` keeps it (`pair-NAME.json`, mode 0600, the directory 0700):
/// `pod_relay::pair::Pair` has no public constructor.
fn write_pair(dir: &Path, name: &str, url: &str, expires: u64, stop_token: &str) {
    use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _};
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .unwrap();
    let doc = serde_json::json!({
        "relay": url, "name": name, "node_token": NODE_TOKEN,
        "connect_token": CONNECT_TOKEN, "stop_token": stop_token, "expires": expires,
    });
    let path = dir.join(format!("pair-{name}.json"));
    std::fs::write(&path, doc.to_string()).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
}

/// A TCP echo server; gives its address.
async fn echo() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            tokio::spawn(async move {
                let (mut r, mut w) = socket.split();
                let _ = tokio::io::copy(&mut r, &mut w).await;
            });
        }
    });
    addr
}

async fn wait_for_node(state: &Arc<Mutex<State>>, sockets: usize) {
    for _ in 0..200 {
        if state.lock().unwrap().node_sockets >= sockets {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("no node socket {sockets}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_wrong_connect_token_is_a_config_error() {
    test_host::need_loopback!();
    let (url, _state) = fake_relay(None).await;
    let relay = Relay::new(&url, None).unwrap();
    let err = connect(
        &relay,
        NAME,
        "testonly-wrong",
        tokio::io::empty(),
        tokio::io::sink(),
    )
    .await
    .unwrap_err();
    assert_eq!(podnet::exit::code_of(&err), podnet::exit::CONFIG, "{err:#}");
}

// podssh: at the expiry, a node with no pair name takes the next stored
// pair, with no new start.
#[tokio::test(flavor = "multi_thread")]
async fn a_node_takes_the_next_pair_at_the_expiry() {
    test_host::need_loopback!();
    let (url, state) = fake_relay(None).await;
    let target = Target::parse(&echo().await).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let soon = ms_from_now(Duration::from_secs(3));
    write_pair(dir.path(), NAME, &url, soon, "");
    let ending = load(dir.path(), Some(NAME)).unwrap();
    let path = dir.path().to_path_buf();
    let node = tokio::spawn(async move { run_node(Some(&path), ending, &target).await });
    wait_for_node(&state, 1).await;
    // The next pair is made while the node runs.
    let later = ms_from_now(Duration::from_secs(3600));
    write_pair(dir.path(), "testnode2", &url, later, "");
    for _ in 0..400 {
        if state.lock().unwrap().node_sockets >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(
        state.lock().unwrap().node_sockets >= 2,
        "the node opened the next pair"
    );
    assert!(!node.is_finished(), "the node goes on");
    node.abort();
}

// A node of a named pair ends at the expiry (exit 78).
#[tokio::test(flavor = "multi_thread")]
async fn a_named_node_ends_at_the_expiry() {
    test_host::need_loopback!();
    let (url, _state) = fake_relay(None).await;
    let target = Target::parse(&echo().await).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let soon = ms_from_now(Duration::from_secs(2));
    write_pair(dir.path(), NAME, &url, soon, "");
    let ending = load(dir.path(), Some(NAME)).unwrap();
    let err = tokio::time::timeout(Duration::from_secs(20), run_node(None, ending, &target))
        .await
        .expect("the node ends")
        .unwrap_err();
    assert_eq!(podnet::exit::code_of(&err), 78, "{err:#}");
    assert!(format!("{err:#}").contains("expired"), "{err:#}");
}

/// Run `podnet` with a state directory and a run directory.
fn podnet_in(
    state: &std::path::Path,
    run: &std::path::Path,
    args: &[&str],
) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(args)
        .env("PODNET_STATE_DIR", state)
        .env("PODNET_RUN_DIR", run)
        .env_remove("PODNET_RELAY")
        .output()
        .unwrap()
}

// T-195: a pair with no stop token stops no local instance. The command
// exits 69 and keeps the pair file. Before, it stopped the instance first.
#[test]
fn relay_stop_checks_the_pair_before_it_stops_an_instance() {
    use std::os::fd::AsRawFd as _;
    let state = tempfile::tempdir().unwrap();
    let run = tempfile::tempdir().unwrap();
    let relay_dir = state.path().join("relay");
    let expires = ms_from_now(Duration::from_secs(3600));
    write_pair(&relay_dir, "notoken", "http://127.0.0.1:1", expires, "");
    // A live instance of the node of this pair: a held lock and a record.
    let mut child = std::process::Command::new("sleep")
        .arg("60")
        .spawn()
        .unwrap();
    std::fs::set_permissions(
        run.path(),
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .unwrap();
    let lock = std::fs::File::create(run.path().join("n1.lock")).unwrap();
    // SAFETY: flock on a descriptor of this process.
    assert_eq!(unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) }, 0);
    let record = serde_json::json!({
        "name": "n1", "pid": child.id(), "started": 1,
        "command": "relay node", "args": ["relay", "node", "notoken"],
        "hash": "x", "cwd": "/", "state": "ready",
        "ready": {"pair": "notoken"}, "log": "n1.log",
    });
    std::fs::write(run.path().join("n1.json"), record.to_string()).unwrap();
    let out = podnet_in(state.path(), run.path(), &["relay", "stop", "notoken"]);
    let err = String::from_utf8_lossy(&out.stderr);
    let alive = child.try_wait().unwrap().is_none();
    let _ = child.kill();
    let _ = child.wait();
    assert_eq!(out.status.code(), Some(69), "{err}");
    assert!(err.contains("stop token"), "{err}");
    assert!(alive, "the instance was stopped: {err}");
    assert!(
        relay_dir.join("pair-notoken.json").exists(),
        "the file stays"
    );
    assert!(!String::from_utf8_lossy(&out.stdout).contains("stopped"));
}

// T-091: `serve DIR --ingress relay --pair NAME` serves the files through the
// node of the pair: an operator session gets the HTTP answer of the file.
#[tokio::test(flavor = "multi_thread")]
async fn serve_via_relay_answers_through_the_relay() {
    test_host::need_loopback!();
    let (url, state) = fake_relay(None).await;
    let (home, run, www) = (
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
    );
    std::fs::set_permissions(
        run.path(),
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .unwrap();
    let expires = ms_from_now(Duration::from_secs(3600));
    write_pair(&home.path().join("relay"), NAME, &url, expires, "");
    std::fs::write(www.path().join("a.txt"), "over the relay").unwrap();
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args([
            "serve",
            www.path().to_str().unwrap(),
            "--ingress",
            "relay",
            "--pair",
            NAME,
            "--list",
        ])
        .env("PODNET_STATE_DIR", home.path())
        .env("PODNET_RUN_DIR", run.path())
        .env_remove("PODNET_RELAY")
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    wait_for_node(&state, 1).await;
    let relay = Relay::new(&url, None).unwrap();
    let (ours, theirs) = tokio::io::duplex(1 << 16);
    let session = tokio::spawn(async move {
        let (read, write) = tokio::io::split(theirs);
        connect(&relay, NAME, CONNECT_TOKEN, read, write).await
    });
    let (mut read, mut write) = tokio::io::split(ours);
    write
        .write_all(b"GET /a.txt HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut reply = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(20), read.read_to_end(&mut reply)).await;
    let reply = String::from_utf8_lossy(&reply).into_owned();
    drop((read, write));
    let _ = tokio::time::timeout(Duration::from_secs(5), session).await;
    unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
    let status = child.wait().unwrap();
    assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
    assert!(reply.ends_with("over the relay"), "{reply}");
    assert_eq!(status.code(), Some(0), "{status:?}");
    let left: Vec<_> = std::fs::read_dir(run.path())
        .unwrap()
        .flatten()
        .map(|e| e.file_name())
        .collect();
    assert!(
        !left.iter().any(|n| n.to_string_lossy().ends_with(".sock")),
        "{left:?}"
    );
}

// T-094: `git serve --ingress relay --pair NAME` gives the SSH server of git
// to the node of the pair: an operator session gets the SSH banner of
// podnet, and the socket of the run directory goes at the end.
#[tokio::test(flavor = "multi_thread")]
async fn git_serve_via_relay_gives_the_ssh_server() {
    test_host::need_loopback!();
    let (url, state) = fake_relay(None).await;
    let (home, run, repos) = (
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
    );
    std::fs::set_permissions(
        run.path(),
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .unwrap();
    let expires = ms_from_now(Duration::from_secs(3600));
    write_pair(&home.path().join("relay"), NAME, &url, expires, "");
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(["git", "serve", "--root", repos.path().to_str().unwrap()])
        .args(["--ingress", "relay", "--pair", NAME, "--anon-read"])
        .env("PODNET_STATE_DIR", home.path())
        .env("PODNET_RUN_DIR", run.path())
        .env_remove("PODNET_RELAY")
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    wait_for_node(&state, 1).await;
    let relay = Relay::new(&url, None).unwrap();
    let (ours, theirs) = tokio::io::duplex(1 << 16);
    let session = tokio::spawn(async move {
        let (read, write) = tokio::io::split(theirs);
        connect(&relay, NAME, CONNECT_TOKEN, read, write).await
    });
    let (mut read, mut write) = tokio::io::split(ours);
    write.write_all(b"SSH-2.0-test\r\n").await.unwrap();
    let mut banner = vec![0u8; 15];
    let got = tokio::time::timeout(Duration::from_secs(20), read.read_exact(&mut banner)).await;
    drop((read, write));
    let _ = tokio::time::timeout(Duration::from_secs(5), session).await;
    unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
    let status = child.wait().unwrap();
    assert!(got.is_ok_and(|r| r.is_ok()), "no banner");
    assert_eq!(
        &banner,
        b"SSH-2.0-podnet_",
        "{:?}",
        String::from_utf8_lossy(&banner)
    );
    assert_eq!(status.code(), Some(0), "{status:?}");
    let left: Vec<_> = std::fs::read_dir(run.path())
        .unwrap()
        .flatten()
        .map(|e| e.file_name())
        .collect();
    assert!(
        !left.iter().any(|n| n.to_string_lossy().ends_with(".sock")),
        "{left:?}"
    );
    assert!(home.path().join("git/ssh_host_ed25519_key").is_file());
}

// T-083: a node hands each session to a handler of the process: a service of
// the process takes the stream with the session id, and no socket file is
// made. Two sessions run at one time, and each one gets its own stream.
#[tokio::test(flavor = "multi_thread")]
async fn a_node_serves_an_in_process_handler() {
    test_host::need_loopback!();
    let (url, state) = fake_relay(None).await;
    let dir = tempfile::tempdir().unwrap();
    write_pair(
        dir.path(),
        NAME,
        &url,
        ms_from_now(Duration::from_secs(3600)),
        "",
    );
    let pair = load(dir.path(), Some(NAME)).unwrap();
    let (handler, mut sessions) = channel(4);
    // The service: it answers `hello <id>` and then echoes in upper case.
    let served = Arc::new(Mutex::new(Vec::new()));
    let seen = served.clone();
    tokio::spawn(async move {
        while let Some((id, mut stream)) = sessions.recv().await {
            seen.lock().unwrap().push(id.as_str().to_string());
            tokio::spawn(async move {
                let mut line = [0u8; 5];
                if stream.read_exact(&mut line).await.is_ok() {
                    let _ = stream.write_all(&line.to_ascii_uppercase()).await;
                }
            });
        }
    });
    let node =
        tokio::spawn(
            async move { run_node_with(None, pair, Arc::new(handler), "in-process").await },
        );
    wait_for_node(&state, 1).await;
    let relay = Relay::new(&url, None).unwrap();
    let mut replies = Vec::new();
    for word in ["alpha", "bravo"] {
        let (ours, theirs) = tokio::io::duplex(1 << 16);
        let relay = relay.clone();
        let session = tokio::spawn(async move {
            let (read, write) = tokio::io::split(theirs);
            connect(&relay, NAME, CONNECT_TOKEN, read, write).await
        });
        let (mut read, mut write) = tokio::io::split(ours);
        write.write_all(word.as_bytes()).await.unwrap();
        let mut reply = [0u8; 5];
        tokio::time::timeout(Duration::from_secs(20), read.read_exact(&mut reply))
            .await
            .expect("the answer in time")
            .unwrap();
        replies.push(String::from_utf8(reply.to_vec()).unwrap());
        drop((read, write));
        let _ = tokio::time::timeout(Duration::from_secs(5), session).await;
    }
    assert_eq!(replies, ["ALPHA", "BRAVO"]);
    let ids = served.lock().unwrap().clone();
    assert_eq!(ids.len(), 2, "{ids:?}");
    assert_ne!(ids[0], ids[1], "each session has its own id");
    node.abort();
}

// T-083: a closed channel refuses a session: the operator learns it at once.
#[tokio::test(flavor = "multi_thread")]
async fn a_closed_channel_refuses_the_session() {
    test_host::need_loopback!();
    let (url, state) = fake_relay(None).await;
    let dir = tempfile::tempdir().unwrap();
    write_pair(
        dir.path(),
        NAME,
        &url,
        ms_from_now(Duration::from_secs(3600)),
        "",
    );
    let pair = load(dir.path(), Some(NAME)).unwrap();
    let (handler, sessions) = channel(1);
    drop(sessions);
    let node =
        tokio::spawn(
            async move { run_node_with(None, pair, Arc::new(handler), "in-process").await },
        );
    wait_for_node(&state, 1).await;
    let relay = Relay::new(&url, None).unwrap();
    let err = tokio::time::timeout(
        Duration::from_secs(20),
        connect(
            &relay,
            NAME,
            CONNECT_TOKEN,
            tokio::io::empty(),
            tokio::io::sink(),
        ),
    )
    .await
    .expect("an answer in time")
    .unwrap_err();
    assert!(format!("{err:#}").contains("gone"), "{err:#}");
    node.abort();
}
