//! `--announce` on a link (T-285, step 3): the announcements go over a
//! stream of the service `announce`. Two ends learn each other, a tampered
//! announcement is dropped and the link stays up. The links cross unix
//! sockets only.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use podnet::identity::{self, Identity};
use podnet::link::{self, Settings};
use podnet::link_announce::{self as announce, Dropped, Guard, Mesh};
use podnet::link_service as service;
use podnet::mesh_announce::Drop as Rule;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const FAST: u64 = 40;

fn install(dir: &Path, name: &str) -> Arc<Identity> {
    let sub = dir.join(name);
    std::fs::create_dir(&sub).unwrap();
    Arc::new(identity::create(&sub).unwrap())
}

async fn until(what: &str, mut ok: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(10);
    while !ok() {
        assert!(Instant::now() < end, "timeout: {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn direct(mesh: &Mesh, peer: &Identity) -> Option<Value> {
    let doc = mesh.routes(announce::now_ms());
    doc["routes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["origin"] == peer.public().display_fingerprint())
        .cloned()
}

#[tokio::test]
async fn two_ends_with_announce_learn_each_other() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (install(dir.path(), "a"), install(dir.path(), "b"));
    let (la, lb) = link::unix_pair(Settings::default()).unwrap();
    let (la, lb) = (Arc::new(la), Arc::new(lb));
    let ma = Mesh::new(a.clone(), vec!["socks5".into()], announce::now_ms());
    let mb = Mesh::new(b.clone(), Vec::new(), announce::now_ms());
    tokio::spawn(announce::serve_streams(
        ma.clone(),
        la.clone(),
        b.public(),
        Some(7),
    ));
    tokio::spawn(announce::serve_streams(
        mb.clone(),
        lb.clone(),
        a.public(),
        Some(9),
    ));
    let ga = Guard::start(&ma, &la, b.public(), Some(7), FAST);
    let gb = Guard::start(&mb, &lb, a.public(), Some(9), FAST);
    until("a learns b", || direct(&ma, &b).is_some()).await;
    until("b learns a", || direct(&mb, &a).is_some()).await;
    let row = direct(&ma, &b).unwrap();
    assert_eq!(row["cost"], 7, "the cost of the link at a: {row}");
    assert_eq!(row["hops"], 1);
    assert_eq!(row["direct"], true);
    assert_eq!(direct(&mb, &a).unwrap()["cost"], 9);
    assert_eq!(ma.routes(announce::now_ms())["dropped"], 0);
    // The sender sends again: the `seq` of the route rises.
    let seq = direct(&ma, &b).unwrap()["seq"].as_u64().unwrap();
    until("a new seq", || {
        direct(&ma, &b).unwrap()["seq"].as_u64().unwrap() > seq
    })
    .await;
    // The end of one side ends the route of the other at once.
    drop(gb);
    lb.close().await;
    until("a drops the route", || direct(&ma, &b).is_none()).await;
    drop(ga);
}

#[tokio::test]
async fn a_tampered_announcement_is_dropped_and_the_link_stays_up() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (install(dir.path(), "a"), install(dir.path(), "b"));
    let (la, lb) = link::unix_pair(Settings::default()).unwrap();
    let (la, lb) = (Arc::new(la), Arc::new(lb));
    let ma = Mesh::new(a.clone(), Vec::new(), announce::now_ms());
    tokio::spawn(announce::serve_streams(
        ma.clone(),
        la.clone(),
        b.public(),
        None,
    ));
    // The peer is a hand-made sender: it opens the stream itself.
    let mb = Mesh::new(b.clone(), vec!["raw".into()], announce::now_ms());
    mb.neighbour_up(a.public(), 3);
    let mut stream = lb.open().unwrap();
    service::request(&mut stream, announce::NAME).await.unwrap();
    let frame = |wire: &[u8]| {
        let mut f = (wire.len() as u16).to_be_bytes().to_vec();
        f.extend_from_slice(wire);
        f
    };
    let good = mb.make(announce::now_ms()).unwrap();
    let mut bad = mb.make(announce::now_ms()).unwrap();
    // A byte in the cost of the neighbour: the signature fails.
    let at = 1 + 32 + 8 + 8 + 8 + 2 + 32 + 3;
    bad[at] ^= 0x01;
    stream.write_all(&frame(&bad)).await.unwrap();
    until("the drop is counted", || {
        ma.routes(announce::now_ms())["dropped"] == 1
    })
    .await;
    assert!(
        direct(&ma, &b).is_none(),
        "a tampered announcement teaches nothing"
    );
    assert!(la.ended().is_none() && lb.ended().is_none());
    // The same stream goes on: a good announcement is taken.
    stream.write_all(&frame(&good)).await.unwrap();
    until("the good one", || direct(&ma, &b).is_some()).await;
    let doc = ma.routes(announce::now_ms());
    assert_eq!(
        (doc["received"].as_u64(), doc["dropped"].as_u64()),
        (Some(1), Some(1))
    );
    assert!(la.ended().is_none() && lb.ended().is_none());
    // A length above the limit ends the stream, not the link.
    stream.write_all(&[0xff, 0xff]).await.unwrap();
    let mut byte = [0u8; 1];
    assert_eq!(stream.read(&mut byte).await.unwrap_or(0), 0);
    assert!(la.ended().is_none() && lb.ended().is_none());
}

#[test]
fn an_announcement_of_another_origin_is_not_direct() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b, c) = (
        install(dir.path(), "a"),
        install(dir.path(), "b"),
        install(dir.path(), "c"),
    );
    let ma = Mesh::new(a, Vec::new(), 1_000);
    let mc = Mesh::new(c.clone(), Vec::new(), 1_000);
    let wire = mc.make(1_000).unwrap();
    // The peer b sends the announcement that c signed.
    assert_eq!(
        ma.receive(&b.public(), 1, &wire, 1_000).unwrap_err(),
        Dropped::NotDirect
    );
    // The refused announcement did not use the `seq` of c.
    assert!(ma.receive(&c.public(), 1, &wire, 1_000).is_ok());
    assert_eq!(
        ma.receive(&c.public(), 1, &wire, 1_001).unwrap_err(),
        Dropped::Announcement(Rule::SeenSeq)
    );
}

#[test]
fn the_cost_and_the_delay_follow_their_rules() {
    let ms = Duration::from_millis;
    assert_eq!(announce::link_cost(Some(5), Some(ms(40))), 5);
    assert_eq!(announce::link_cost(None, Some(ms(40))), 40);
    assert_eq!(
        announce::link_cost(None, Some(Duration::from_micros(20))),
        1
    );
    assert_eq!(announce::link_cost(None, None), announce::DEFAULT_COST);
    for r in [0, 1, 1999, 2000, 2001, u64::MAX] {
        let d = announce::next_delay(10_000, r);
        assert!(d >= ms(9_000) && d <= ms(11_000), "{d:?}");
    }
    assert_eq!(announce::next_delay(10_000, 0), ms(9_000));
}

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

fn run(dir: &Path, args: &[&str]) -> Output {
    host_command(dir, args).output().unwrap()
}

fn fingerprint(dir: &Path) -> String {
    let out = run(dir, &["link", "status", "--create", "--json"]);
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    doc["identity"]["fingerprint"].as_str().unwrap().to_string()
}

fn routes(socket: &Path) -> Value {
    use std::io::Write;
    let token = std::fs::read_to_string(format!("{}.token", socket.display())).unwrap();
    let mut s = std::os::unix::net::UnixStream::connect(socket).unwrap();
    let line = json!({"verb": "routes", "token": token.trim()}).to_string();
    s.write_all(line.as_bytes()).unwrap();
    s.write_all(b"\n").unwrap();
    let mut got = String::new();
    BufReader::new(&mut s).read_line(&mut got).unwrap();
    serde_json::from_str(&got).unwrap()
}

#[test]
fn the_commands_announce_and_the_control_socket_shows_the_routes() {
    let (server, client) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (sfp, cfp) = (fingerprint(server.path()), fingerprint(client.path()));
    let allow = server.path().join("allow");
    std::fs::write(&allow, &cfp).unwrap();
    let sock: PathBuf = server.path().join("l.sock");
    let ctl: PathBuf = server.path().join("c.sock");
    let mut listen = host_command(
        server.path(),
        &[
            "link",
            "listen",
            sock.to_str().unwrap(),
            "--allow",
            allow.to_str().unwrap(),
            "--service",
            "socks5",
            "--announce",
            "--announce-weight",
            "4",
            "--control",
            ctl.to_str().unwrap(),
            "--json",
        ],
    )
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()
    .unwrap();
    let mut line = String::new();
    BufReader::new(listen.stdout.as_mut().unwrap())
        .read_line(&mut line)
        .unwrap();
    let _listen = Proc(listen);
    assert!(!line.is_empty());
    // Before a peer comes, the table is empty.
    let none = routes(&ctl);
    assert_eq!(none["ok"], true, "{none}");
    assert_eq!(none["routes"].as_array().unwrap().len(), 0);
    let dial = host_command(
        client.path(),
        &[
            "link",
            "dial",
            sock.to_str().unwrap(),
            "--peer",
            &sfp,
            "--service",
            "socks5",
            "--announce",
        ],
    )
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()
    .unwrap();
    let _dial = Proc(dial);
    let end = Instant::now() + Duration::from_secs(10);
    let doc = loop {
        let doc = routes(&ctl);
        if doc["routes"].as_array().unwrap().len() == 1 {
            break doc;
        }
        assert!(Instant::now() < end, "no route: {doc}");
        std::thread::sleep(Duration::from_millis(50));
    };
    let row = &doc["routes"][0];
    assert_eq!(row["cost"], 4, "{doc}");
    assert_eq!(row["direct"], true);
    assert_eq!(doc["dropped"], 0);
    assert_eq!(doc["neighbours"], 1);
}

#[test]
fn announce_flags_are_checked() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("l.sock");
    let allow = dir.path().join("allow");
    std::fs::write(&allow, "").unwrap();
    let out = run(
        dir.path(),
        &[
            "link",
            "listen",
            sock.to_str().unwrap(),
            "--allow",
            allow.to_str().unwrap(),
            "--announce",
            "--service",
            "announce=unix:/x",
        ],
    );
    assert_eq!(out.status.code(), Some(64), "{out:?}");
    let out = run(
        dir.path(),
        &[
            "link",
            "listen",
            sock.to_str().unwrap(),
            "--allow",
            allow.to_str().unwrap(),
            "--announce-weight",
            "3",
        ],
    );
    assert_eq!(out.status.code(), Some(64), "{out:?}");
}
