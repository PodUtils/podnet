//! The control socket of a link instance (T-267, GitHub #466, row E0-6):
//! mode 0600, a token for each run, the uid check, one JSON object for each
//! line, the limits, and the verb `status`. The tests use unix sockets
//! only: they bind no TCP port.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use podnet::exit::code_of;
use podnet::hello;
use podnet::identity::{self, AllowList, Identity};
use podnet::link::{self, Settings};
use podnet::link_ctl::{Client, Control, Hub};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

fn mode(path: &Path) -> u32 {
    std::fs::symlink_metadata(path)
        .unwrap()
        .permissions()
        .mode()
        & 0o7777
}

fn install(dir: &Path, name: &str) -> Identity {
    let sub = dir.join(name);
    std::fs::create_dir(&sub).unwrap();
    identity::create(&sub).unwrap()
}

/// A control socket over a hub with the fingerprint of `me`.
fn control(dir: &Path, me: &Identity) -> (Control, Arc<Hub>, PathBuf) {
    let hub = Hub::new(&me.fingerprint());
    let path = dir.join("ctl.sock");
    let control = Control::start(&path, hub.clone()).unwrap();
    (control, hub, path)
}

/// One line to the socket and the answer line.
async fn ask(path: &Path, line: &str) -> Value {
    let mut s = UnixStream::connect(path).await.unwrap();
    s.write_all(line.as_bytes()).await.unwrap();
    s.write_all(b"\n").await.unwrap();
    let mut got = Vec::new();
    let mut b = [0u8; 1];
    while s.read(&mut b).await.unwrap() == 1 && b[0] != b'\n' {
        got.push(b[0]);
    }
    serde_json::from_slice(&got)
        .unwrap_or_else(|e| panic!("{e}: {:?}", String::from_utf8_lossy(&got)))
}

#[tokio::test]
async fn the_socket_and_the_token_file_have_mode_0600() {
    let dir = tempfile::tempdir().unwrap();
    let me = install(dir.path(), "me");
    let (control, _hub, path) = control(dir.path(), &me);
    assert_eq!(mode(&path), 0o600);
    let token_file = PathBuf::from(format!("{}.token", path.display()));
    assert_eq!(mode(&token_file), 0o600);
    let token = std::fs::read_to_string(&token_file).unwrap();
    assert_eq!(token.trim().len(), 64, "32 random bytes in hex");
    assert_eq!(token.trim(), control.token());
    drop(control);
    assert!(
        !path.exists() && !token_file.exists(),
        "the drop removes both"
    );
}

#[tokio::test]
async fn the_token_is_new_for_each_run() {
    let dir = tempfile::tempdir().unwrap();
    let me = install(dir.path(), "me");
    let a = {
        let (control, _hub, _) = control(dir.path(), &me);
        control.token().to_string()
    };
    let (control, _hub, _) = control(dir.path(), &me);
    assert_ne!(a, control.token());
}

#[tokio::test]
async fn a_bad_or_missing_token_gets_77_and_a_good_one_gets_status() {
    let dir = tempfile::tempdir().unwrap();
    let me = install(dir.path(), "me");
    let (control, _hub, path) = control(dir.path(), &me);
    for line in [
        json!({"verb": "status", "token": "0".repeat(64)}).to_string(),
        json!({"verb": "status"}).to_string(),
    ] {
        let answer = ask(&path, &line).await;
        assert_eq!(answer["ok"], false, "{answer}");
        assert_eq!(answer["code"], 77, "{answer}");
        assert!(!answer.to_string().contains(control.token()), "{answer}");
    }
    let good = json!({"verb": "status", "token": control.token()}).to_string();
    let answer = ask(&path, &good).await;
    assert_eq!(answer["ok"], true, "{answer}");
    assert_eq!(answer["fingerprint"], me.fingerprint().as_str());
    assert_eq!(answer["links"], json!([]));
}

#[tokio::test]
async fn the_client_maps_a_bad_token_to_exit_77() {
    let dir = tempfile::tempdir().unwrap();
    let me = install(dir.path(), "me");
    let (_control, _hub, path) = control(dir.path(), &me);
    let mut client = Client::connect(&path).await.unwrap();
    let err = client
        .call(&"f".repeat(64), "status", json!({}))
        .await
        .unwrap_err();
    assert_eq!(code_of(&err), 77, "{err:#}");
}

#[tokio::test]
async fn a_user_that_the_gate_refuses_gets_77_with_the_right_token() {
    let dir = tempfile::tempdir().unwrap();
    let me = install(dir.path(), "me");
    let hub = Hub::new(&me.fingerprint());
    let path = dir.path().join("gate.sock");
    let control = Control::start_gated(&path, hub, Arc::new(|_uid| false)).unwrap();
    let good = json!({"verb": "status", "token": control.token()}).to_string();
    let answer = ask(&path, &good).await;
    assert_eq!(answer["code"], 77, "{answer}");
    assert_eq!(answer["ok"], false);
}

#[test]
fn the_uid_gate_is_the_one_of_the_router() {
    // The same rule as `router::control::peer_allowed`: the user of the
    // instance, or root.
    assert!(link_ctl_allowed(1000, 1000));
    assert!(link_ctl_allowed(0, 1000));
    assert!(!link_ctl_allowed(1001, 1000));
}

fn link_ctl_allowed(peer: u32, own: u32) -> bool {
    podnet::link_ctl::peer_allowed(peer, own)
}

#[tokio::test]
async fn a_request_over_4_kib_is_refused_and_a_bad_line_gets_64() {
    let dir = tempfile::tempdir().unwrap();
    let me = install(dir.path(), "me");
    let (control, _hub, path) = control(dir.path(), &me);
    let long = json!({"verb": "status", "token": control.token(), "pad": "x".repeat(5000)});
    let answer = ask(&path, &long.to_string()).await;
    assert_eq!(answer["ok"], false, "{answer}");
    assert_eq!(answer["code"], 64, "{answer}");
    let answer = ask(&path, "not json").await;
    assert_eq!(answer["code"], 64, "{answer}");
    let unknown = json!({"verb": "format-disk", "token": control.token()}).to_string();
    let answer = ask(&path, &unknown).await;
    assert_eq!(answer["code"], 64, "{answer}");
    assert!(
        answer["error"].as_str().unwrap().contains("status"),
        "{answer}"
    );
}

#[tokio::test]
async fn the_seventeenth_client_is_refused_and_a_closed_one_frees_its_place() {
    let dir = tempfile::tempdir().unwrap();
    let me = install(dir.path(), "me");
    let (control, _hub, path) = control(dir.path(), &me);
    let mut held = Vec::new();
    for _ in 0..16 {
        let mut s = UnixStream::connect(&path).await.unwrap();
        // A round trip proves that the server holds this client.
        let line = json!({"verb": "status", "token": control.token()}).to_string();
        s.write_all(format!("{line}\n").as_bytes()).await.unwrap();
        let mut b = [0u8; 1];
        s.read_exact(&mut b).await.unwrap();
        held.push(s);
    }
    let answer = ask(
        &path,
        &json!({"verb": "status", "token": control.token()}).to_string(),
    )
    .await;
    assert_eq!(answer["ok"], false, "{answer}");
    assert_eq!(answer["code"], 69, "{answer}");
    held.pop();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let answer = ask(
        &path,
        &json!({"verb": "status", "token": control.token()}).to_string(),
    )
    .await;
    assert_eq!(answer["ok"], true, "{answer}");
}

#[tokio::test]
async fn status_shows_a_held_link_its_peer_and_its_counters_and_no_seed() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (install(dir.path(), "a"), install(dir.path(), "b"));
    let list = |k: &Identity| AllowList::parse(&k.fingerprint()).unwrap();
    let (dialer, listener) = link::memory_pair(1 << 20, Settings::default()).unwrap();
    let (d, l) = tokio::join!(
        hello::dial(&dialer, &a, list(&b)),
        hello::accept(&listener, &b, list(&a))
    );
    let (listener_key, dialer_key) = (d.unwrap(), l.unwrap());
    let (_control, hub, path) = control(dir.path(), &b);
    let slot = hub.hold(Arc::new(listener), dialer_key, "memory", "pair", "listener");
    slot.count_accepted();
    let token = std::fs::read_to_string(format!("{}.token", path.display())).unwrap();
    let mut client = Client::connect(&path).await.unwrap();
    let doc = client
        .call(token.trim(), "status", json!({}))
        .await
        .unwrap();
    let row = &doc["links"][0];
    assert_eq!(row["carrier"], "memory", "{doc}");
    assert_eq!(row["role"], "listener");
    assert_eq!(row["peer"], a.fingerprint().as_str());
    assert_eq!(row["state"], "up");
    assert_eq!(row["accepted"], 1);
    assert_eq!(row["opened"], 0);
    assert!(row["streams"].is_u64() && row["since"].is_u64(), "{doc}");
    let _ = listener_key;
    // The seed of the key never shows, in any form.
    let seed = std::fs::read_to_string(a.path()).unwrap();
    let seed_b = std::fs::read_to_string(b.path()).unwrap();
    let shown = doc.to_string();
    assert!(!shown.contains(seed.trim()) && !shown.contains(seed_b.trim()));
    assert!(!shown.contains(token.trim()), "the token never shows");
    // A link that ends leaves the table.
    drop(slot);
    let doc = client
        .call(token.trim(), "status", json!({}))
        .await
        .unwrap();
    assert_eq!(doc["links"], json!([]), "{doc}");
    drop(dialer);
}

// The command: `podnet link listen` runs the socket, and `podnet link
// status` reads it.

struct Host {
    dir: tempfile::TempDir,
}

impl Host {
    fn new() -> Host {
        Host {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_podnet"));
        cmd.args(args)
            .env("PODNET_STATE_DIR", self.path("state"))
            .env("PODNET_RUN_DIR", self.path("run"))
            .env_remove("RUST_LOG");
        cmd
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command(args).output().unwrap()
    }

    fn fingerprint(&self) -> String {
        let out = self.run(&["link", "status", "--create", "--json"]);
        let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
        doc["identity"]["fingerprint"].as_str().unwrap().to_string()
    }

    fn allow(&self, lines: &[&str]) -> PathBuf {
        let path = self.path("allow");
        std::fs::write(&path, lines.join("\n")).unwrap();
        path
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// A detached listener `name` at `sock`; its ready document.
fn detached(host: &Host, sock: &Path, allow: &Path, name: &str, more: &[&str]) -> Value {
    let mut args = vec![
        "link",
        "listen",
        sock.to_str().unwrap(),
        "--allow",
        allow.to_str().unwrap(),
        "--detach",
        "--name",
        name,
    ];
    args.extend_from_slice(more);
    let out = host.run(&args);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    serde_json::from_str(text(&out.stdout).lines().last().unwrap()).unwrap()
}

/// An echo service on a unix socket: it copies each connection back.
fn echo(path: &Path) {
    let listener = std::os::unix::net::UnixListener::bind(path).unwrap();
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut conn) = conn else { return };
            std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                while let Ok(n) = std::io::Read::read(&mut conn, &mut buf) {
                    if n == 0 || std::io::Write::write_all(&mut conn, &buf[..n]).is_err() {
                        break;
                    }
                }
            });
        }
    });
}

struct Dialer(Child);

impl Drop for Dialer {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn the_listener_runs_a_control_socket_that_status_reads() {
    let server = Host::new();
    let client = Host::new();
    let (sfp, cfp) = (server.fingerprint(), client.fingerprint());
    let allow = server.allow(&[&cfp]);
    let sock = server.path("l.sock");
    let origin = server.path("origin.sock");
    echo(&origin);
    let to = format!("unix:{}", origin.display());
    let ready = detached(&server, &sock, &allow, "lk", &["--to", &to]);
    let control = PathBuf::from(ready["control"].as_str().expect(&ready.to_string()));
    assert_eq!(mode(&control), 0o600);
    assert!(!ready.to_string().contains(
        &std::fs::read_to_string(format!("{}.token", control.display()))
            .unwrap()
            .trim()
            .to_string()
    ));

    // A link that stays up: the dialer keeps its stdin open.
    let mut dial = Dialer(
        client
            .command(&["link", "dial", sock.to_str().unwrap(), "--peer", &sfp])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    let doc = loop {
        let out = server.run(&["link", "status", "--json"]);
        assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
        let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
        if !doc["links"][0]["live"]["links"]
            .as_array()
            .is_none_or(|l| l.is_empty())
            || Instant::now() > deadline
        {
            break doc;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let live = &doc["links"][0]["live"];
    assert_eq!(live["fingerprint"], sfp.as_str(), "{doc}");
    assert_eq!(live["links"][0]["peer"], cfp.as_str(), "{doc}");
    assert_eq!(live["links"][0]["role"], "listener");
    assert!(
        !doc.to_string().contains("token"),
        "no token in the status: {doc}"
    );

    let shown = text(&server.run(&["link", "status"]).stdout);
    assert!(
        shown.contains(&cfp),
        "the text form names the peer: {shown}"
    );

    // The explicit form: the socket and the token of the instance.
    let out = server.run(&[
        "link",
        "status",
        "--control",
        control.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let direct: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(direct["live"]["links"][0]["peer"], cfp.as_str(), "{direct}");

    // A wrong token exits 77.
    let wrong = server.path("wrong.token");
    std::fs::write(&wrong, "0".repeat(64)).unwrap();
    std::fs::set_permissions(&wrong, std::fs::Permissions::from_mode(0o600)).unwrap();
    let out = server.run(&[
        "link",
        "status",
        "--control",
        control.to_str().unwrap(),
        "--token-file",
        wrong.to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(77), "{}", text(&out.stderr));

    drop(dial.0.stdin.take());
    drop(dial);
    let out = server.run(&["link", "close", "lk"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(!control.exists(), "the stop removes the control socket");
    assert!(!PathBuf::from(format!("{}.token", control.display())).exists());
}

#[test]
fn status_falls_back_to_the_record_when_the_socket_is_gone() {
    let server = Host::new();
    let allow = server.allow(&[&server.fingerprint()]);
    let sock = server.path("l.sock");
    let ready = detached(&server, &sock, &allow, "lk", &[]);
    let control = PathBuf::from(ready["control"].as_str().unwrap());
    std::fs::remove_file(&control).unwrap();
    let out = server.run(&["link", "status", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["links"][0]["name"], "lk", "{doc}");
    assert!(doc["links"][0]["live"].is_null(), "{doc}");
    let _ = server.run(&["link", "close", "lk"]);
}
