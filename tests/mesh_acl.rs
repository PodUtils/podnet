//! Per-hop access control (T-290): the allow list gives each peer a list of
//! services and a `forward` flag. A refused OPEN or forward is answered
//! with 6 and counted, and the link stays up. The links cross unix
//! sockets.

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pod_identity::PublicKey;
use podnet::carriers::{self, Options};
use podnet::exit::code_of;
use podnet::hello;
use podnet::identity::{self, AllowList, Identity, MAX_SERVICES};
use podnet::link::{self, Link, Settings};
use podnet::link_forward::Header;
use podnet::link_service as service;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Proc(Child);

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn host_command(dir: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_podnet"));
    cmd.args(args)
        .env("PODNET_STATE_DIR", dir.join("state"))
        .env("PODNET_RUN_DIR", dir.join("run"))
        .env_remove("PODNET_LINK_CONTROL")
        .env_remove("RUST_LOG");
    cmd
}

fn fingerprint(dir: &Path) -> String {
    let out = host_command(dir, &["link", "status", "--create", "--json"])
        .output()
        .unwrap();
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    doc["identity"]["fingerprint"].as_str().unwrap().to_string()
}

fn key_of(dir: &Path) -> Identity {
    identity::open(&dir.join("state")).unwrap()
}

/// An echo service on a unix socket.
fn echo(path: &Path) {
    let listener = std::os::unix::net::UnixListener::bind(path).unwrap();
    std::thread::spawn(move || {
        for conn in listener.incoming().flatten() {
            std::thread::spawn(move || {
                let (mut r, mut w) = (conn.try_clone().unwrap(), conn);
                let _ = std::io::copy(&mut r, &mut w);
            });
        }
    });
}

fn start(cmd: &mut Command) -> Proc {
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.as_mut().unwrap())
        .read_line(&mut line)
        .unwrap();
    if line.is_empty() {
        let mut err = String::new();
        let _ = child.stderr.take().unwrap().read_to_string(&mut err);
        panic!("no first line: {err}");
    }
    Proc(child)
}

struct World {
    dir: tempfile::TempDir,
    sock: PathBuf,
    control: PathBuf,
    b_fp: String,
    _listener: Proc,
}

impl World {
    fn host(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    /// The row of `status` of B for the peer `peer`.
    fn row(&self, peer: &Identity) -> Value {
        let out = host_command(
            &self.host("b"),
            &[
                "link",
                "status",
                "--control",
                self.control.to_str().unwrap(),
                "--json",
            ],
        )
        .output()
        .unwrap();
        let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
        let want = peer.fingerprint();
        doc["live"]["links"]
            .as_array()
            .unwrap()
            .iter()
            .find(|l| l["peer"] == want.as_str())
            .cloned()
            .unwrap_or_else(|| panic!("no link of {want}: {doc}"))
    }
}

/// B listens with `--announce`, the services `raw` (an echo) and `socks5`,
/// and the allow list `lines`.
fn world(dir: tempfile::TempDir, lines: &[String]) -> World {
    let b = dir.path().join("b");
    std::fs::create_dir(&b).unwrap();
    let b_fp = fingerprint(&b);
    let origin = b.join("origin.sock");
    echo(&origin);
    let allow = b.join("allow");
    std::fs::write(&allow, lines.join("\n")).unwrap();
    let sock = b.join("l.sock");
    let control = b.join("ctl.sock");
    let target = format!("raw=unix:{}", origin.display());
    let listener = start(&mut host_command(
        &b,
        &[
            "link",
            "listen",
            sock.to_str().unwrap(),
            "--allow",
            allow.to_str().unwrap(),
            "--service",
            &target,
            "--service",
            "socks5",
            "--announce",
            "--control",
            control.to_str().unwrap(),
            "--json",
        ],
    ));
    World {
        dir,
        sock,
        control,
        b_fp,
        _listener: listener,
    }
}

/// A new install `name` under the dir of the world, with its key made.
fn install(dir: &Path, name: &str) -> (PathBuf, Arc<Identity>) {
    let path = dir.join(name);
    std::fs::create_dir_all(&path).unwrap();
    fingerprint(&path);
    (path.clone(), Arc::new(key_of(&path)))
}

async fn connect(w: &World, me: &Identity) -> Link {
    let carrier = carriers::open("unix", &Options::default()).unwrap();
    let link = link::dial(&*carrier, w.sock.to_str().unwrap(), Settings::default())
        .await
        .unwrap();
    let allow = AllowList::parse(&w.b_fp).unwrap();
    hello::dial(&link, me, allow).await.unwrap();
    link
}

async fn echoes(link: &Link) {
    let mut stream = link.open().unwrap();
    service::request(&mut stream, "raw").await.unwrap();
    stream.write_all(b"ping").await.unwrap();
    let mut got = [0u8; 4];
    tokio::time::timeout(Duration::from_secs(20), stream.read_exact(&mut got))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&got, b"ping");
}

/// Send the header of a forwarded stream and read the answer byte.
async fn forward_answer(link: &Link, dest: &PublicKey) -> u8 {
    let mut stream = link.open().unwrap();
    let header = Header {
        dest: *dest,
        hops: 4,
        service: "raw".to_string(),
        visited: Vec::new(),
    };
    let request = format!("{}{}\n{}", "PNSVC1 ", "fwd", header.line());
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut answer = [0u8; 1];
    tokio::time::timeout(Duration::from_secs(20), stream.read_exact(&mut answer))
        .await
        .unwrap()
        .unwrap();
    answer[0]
}

#[tokio::test]
async fn a_peer_allowed_raw_only_cannot_open_socks5() {
    let dir = tempfile::tempdir().unwrap();
    let (_, a) = install(dir.path(), "a");
    // The old form: a bare key and a comment word. It keeps every service.
    let (_, old) = install(dir.path(), "old");
    let lines = [
        format!("{} services=raw", a.fingerprint()),
        format!("{} laptop", old.fingerprint()),
    ];
    let w = world(dir, &lines);
    let link = connect(&w, &a).await;
    echoes(&link).await;
    let mut stream = link.open().unwrap();
    let error = service::request(&mut stream, "socks5").await.unwrap_err();
    assert_eq!(code_of(&error), 77, "{error:#}");
    // The link stays up: the next stream works.
    echoes(&link).await;
    let row = w.row(&a);
    assert_eq!(row["refused_service"], 1, "{row}");
    assert_eq!(row["refused_forward"], 0, "{row}");
    assert_eq!(row["state"], "up", "{row}");
    // The peer of the old form opens socks5 (the answer is 0).
    let old_link = connect(&w, &old).await;
    let mut stream = old_link.open().unwrap();
    service::request(&mut stream, "socks5").await.unwrap();
    echoes(&old_link).await;
    assert_eq!(w.row(&old)["refused_service"], 0);
}

#[tokio::test]
async fn a_peer_without_forward_cannot_forward_and_the_link_stays_up() {
    let dir = tempfile::tempdir().unwrap();
    let (_, a) = install(dir.path(), "a");
    let (_, bare) = install(dir.path(), "bare");
    let (c_dir, c) = install(dir.path(), "c");
    let lines = [
        format!("{} services=raw", a.fingerprint()),
        format!("{} forward=yes", bare.fingerprint()),
        c.fingerprint(),
    ];
    let w = world(dir, &lines);
    // C holds a link to B. It has no service of its own.
    let _c = Proc(
        host_command(
            &c_dir,
            &[
                "link",
                "dial",
                w.sock.to_str().unwrap(),
                "--peer",
                &w.b_fp,
                "--announce",
            ],
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap(),
    );
    // A forward through B to C: refused, answered 6, counted.
    let link = connect(&w, &a).await;
    assert_eq!(forward_answer(&link, &c.public()).await, service::FORBIDDEN);
    assert_eq!(w.row(&a)["refused_forward"], 1);
    // The A-B link stays up and A still opens raw.
    echoes(&link).await;
    assert_eq!(w.row(&a)["state"], "up");
    // A peer with `forward=yes` is relayed to C. C has no service, so the
    // answer 1 comes back from C once B knows the route.
    let relay = connect(&w, &bare).await;
    let end = Instant::now() + Duration::from_secs(30);
    loop {
        match forward_answer(&relay, &c.public()).await {
            service::NO_SERVICE => break,
            service::NO_ROUTE => {}
            other => panic!("the answer is {other}"),
        }
        assert!(Instant::now() < end, "B has no route to C");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert_eq!(w.row(&bare)["refused_forward"], 0);
    // `forward=yes` grants no service of B itself.
    let mut stream = relay.open().unwrap();
    let error = service::request(&mut stream, "raw").await.unwrap_err();
    assert_eq!(code_of(&error), 77, "{error:#}");
}

#[test]
fn the_limits_and_the_old_form_of_the_file() {
    let key = |n: u32| format!("{:064x}", n + 1);
    // 64 services for a peer: good. 65: exit 78.
    let names = |n: usize| {
        (0..n)
            .map(|i| format!("s{i}"))
            .collect::<Vec<_>>()
            .join(",")
    };
    assert!(AllowList::parse(&format!("{} services={}", key(1), names(MAX_SERVICES))).is_ok());
    let error =
        AllowList::parse(&format!("{} services={}", key(1), names(MAX_SERVICES + 1))).unwrap_err();
    assert_eq!(code_of(&error), 78, "{error:#}");
    // 4096 entries with a grant: good. The 4097th: exit 78.
    let full: String = (0..identity::MAX_ALLOWED as u32)
        .map(|n| format!("{} services=raw forward=yes\n", key(n)))
        .collect();
    assert_eq!(
        AllowList::parse(&full).unwrap().len(),
        identity::MAX_ALLOWED
    );
    let error = AllowList::parse(&format!("{full}{} services=raw\n", key(9999))).unwrap_err();
    assert_eq!(code_of(&error), 78, "{error:#}");
    // A bad name, an empty list and a bad flag value: exit 78.
    for bad in ["services=a/b", "services=", "forward=maybe"] {
        let error = AllowList::parse(&format!("{} {bad}", key(1))).unwrap_err();
        assert_eq!(code_of(&error), 78, "{bad}: {error:#}");
    }
    // The old form: a bare key, a key with comment words (also `forward`),
    // and a comment after `#`. Each has the open grant.
    let list = AllowList::parse(&format!(
        "{}\n{} laptop of me\n{} forward\n{} # services=raw\n",
        key(1),
        key(2),
        key(3),
        key(4)
    ))
    .unwrap();
    for n in 1..=4 {
        let grant = list.grant(&pk(n)).unwrap();
        assert!(grant.allows_service("socks5") && grant.may_forward(), "{n}");
    }
    // The first entry of a key decides. A key that is not listed has none.
    assert!(list.grant(&pk(77)).is_none());
    let list = AllowList::parse(&format!("{} services=raw,echo\n", key(5))).unwrap();
    let grant = list.grant(&pk(5)).unwrap();
    assert!(grant.allows_service("raw") && grant.allows_service("echo"));
    assert!(!grant.allows_service("socks5") && !grant.may_forward());
    let list =
        AllowList::parse(&format!("{} services=*\n{} forward=yes\n", key(5), key(6))).unwrap();
    assert!(list.grant(&pk(5)).unwrap().allows_service("anything"));
    assert!(!list.grant(&pk(5)).unwrap().may_forward());
    let only = list.grant(&pk(6)).unwrap();
    assert!(only.may_forward() && !only.allows_service("raw"));
}

fn pk(n: u32) -> PublicKey {
    let mut bytes = [0u8; 32];
    bytes[28..].copy_from_slice(&(n + 1).to_be_bytes());
    PublicKey(bytes)
}
