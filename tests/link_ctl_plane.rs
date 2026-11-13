//! The write verbs of the link control socket (T-307, GitHub #476, row
//! E10-2): `close-link`, `close-stream` and `select`. Each one sits behind
//! the token and the uid check of T-267 and answers the status document
//! after the change. `rotate` and `reconnect` need a carrier that the
//! instance dials again: they answer 69 until that entry lands. The tests
//! use unix sockets and memory pairs only: they bind no TCP port.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use podnet::exit::code_of;
use podnet::hello;
use podnet::identity::{self, AllowList, Identity};
use podnet::link::{self, Link, Settings, Stream};
use podnet::link_ctl::{Client, Control, Hub, Slot};
use podnet::link_service;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::sync::mpsc;

fn install(dir: &Path, name: &str) -> Identity {
    let sub = dir.join(name);
    std::fs::create_dir(&sub).unwrap();
    identity::create(&sub).unwrap()
}

/// A hub of the install `me` with `links` links to the install `peer`. The
/// far end of each link answers every stream with the service reply and
/// sends the stream on `streams`: (index of the link, stream).
struct Rig {
    control: Control,
    hub: Arc<Hub>,
    path: PathBuf,
    peer: String,
    slots: Vec<Slot>,
    far: Vec<Arc<Link>>,
    streams: mpsc::UnboundedReceiver<(usize, Stream)>,
    _dir: tempfile::TempDir,
}

impl Rig {
    async fn new(links: usize, gate: Option<fn(u32) -> bool>) -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let (me, peer) = (install(dir.path(), "me"), install(dir.path(), "peer"));
        let list = |k: &Identity| AllowList::parse(&k.fingerprint()).unwrap();
        let hub = Hub::new(&me.fingerprint());
        let path = dir.path().join("ctl.sock");
        let control = match gate {
            None => Control::start(&path, hub.clone()).unwrap(),
            Some(g) => Control::start_gated(&path, hub.clone(), Arc::new(g)).unwrap(),
        };
        let (tx, streams) = mpsc::unbounded_channel();
        let (mut slots, mut far) = (Vec::new(), Vec::new());
        for index in 0..links {
            let (dialer, listener) = link::memory_pair(1 << 20, Settings::default()).unwrap();
            let (d, l) = tokio::join!(
                hello::dial(&dialer, &peer, list(&me)),
                hello::accept(&listener, &me, list(&peer))
            );
            let (_me_key, peer_key) = (d.unwrap(), l.unwrap());
            slots.push(hub.hold(
                Arc::new(listener),
                peer_key,
                "memory",
                &format!("pair{index}"),
                "listener",
            ));
            let dialer = Arc::new(dialer);
            far.push(dialer.clone());
            let tx = tx.clone();
            tokio::spawn(async move {
                while let Ok(mut stream) = dialer.accept().await {
                    let tx = tx.clone();
                    tokio::spawn(async move {
                        if link_service::accept(&mut stream).await.is_ok()
                            && link_service::reply(&mut stream, link_service::OPEN)
                                .await
                                .is_ok()
                        {
                            let _ = tx.send((index, stream));
                        }
                    });
                }
            });
        }
        Rig {
            control,
            hub,
            path,
            peer: peer.fingerprint(),
            slots,
            far,
            streams,
            _dir: dir,
        }
    }

    fn token(&self) -> String {
        self.control.token().to_string()
    }

    async fn call(&self, verb: &str, args: Value) -> anyhow::Result<Value> {
        Client::connect(&self.path)
            .await?
            .call(self.control.token(), verb, args)
            .await
    }

    /// A piped stream to the service `raw` of the peer, and the far end.
    async fn open(&mut self) -> (UnixStream, usize, Stream) {
        let conn = Client::connect(&self.path)
            .await
            .unwrap()
            .open(self.control.token(), &self.peer, "raw")
            .await
            .unwrap();
        let (index, far) = tokio::time::timeout(Duration::from_secs(5), self.streams.recv())
            .await
            .expect("the far end got the stream")
            .unwrap();
        (conn, index, far)
    }
}

fn row(doc: &Value, id: u64) -> &Value {
    doc["links"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["id"] == id)
        .unwrap_or_else(|| panic!("no link {id}: {doc}"))
}

#[tokio::test]
async fn close_link_ends_the_link_and_answers_the_status_after_the_change() {
    let rig = Rig::new(1, None).await;
    assert_eq!(
        row(&rig.call("status", json!({})).await.unwrap(), 1)["state"],
        "up"
    );
    let doc = rig.call("close-link", json!({"link": 1})).await.unwrap();
    assert_eq!(doc["ok"], true, "{doc}");
    assert_eq!(doc["verb"], "close-link", "{doc}");
    assert_eq!(doc["fingerprint"], rig.hub.status()["fingerprint"], "{doc}");
    assert_eq!(
        row(&doc, 1)["state"],
        "ended",
        "the answer is the status after the change: {doc}"
    );
    // The far end sees the end of the link.
    let ended = tokio::time::timeout(Duration::from_secs(5), rig.far[0].closed()).await;
    assert!(ended.is_ok(), "the far end of the link did not end");
    // A second close of the same link is not an error.
    let again = rig.call("close-link", json!({"link": 1})).await.unwrap();
    assert_eq!(row(&again, 1)["state"], "ended");
}

#[tokio::test]
async fn close_link_leaves_the_other_links_up() {
    let rig = Rig::new(2, None).await;
    let doc = rig.call("close-link", json!({"link": 2})).await.unwrap();
    assert_eq!(row(&doc, 1)["state"], "up", "{doc}");
    assert_eq!(row(&doc, 2)["state"], "ended", "{doc}");
}

#[tokio::test]
async fn a_write_verb_with_a_bad_token_exits_77_and_changes_nothing() {
    let rig = Rig::new(1, None).await;
    for (verb, args) in [
        ("close-link", json!({"link": 1})),
        ("close-stream", json!({"link": 1, "stream": 1})),
        ("select", json!({"link": 1})),
    ] {
        let mut client = Client::connect(&rig.path).await.unwrap();
        let err = client.call(&"0".repeat(64), verb, args).await.unwrap_err();
        assert_eq!(code_of(&err), 77, "{verb}: {err:#}");
        assert!(!format!("{err:#}").contains(&rig.token()));
    }
    let doc = rig.call("status", json!({})).await.unwrap();
    assert_eq!(
        row(&doc, 1)["state"],
        "up",
        "a refused verb changed the link: {doc}"
    );
    assert_eq!(doc["selected"], Value::Null, "{doc}");
}

#[tokio::test]
async fn a_write_verb_from_a_user_that_the_gate_refuses_exits_77() {
    let rig = Rig::new(1, Some(|_uid| false)).await;
    let err = rig
        .call("close-link", json!({"link": 1}))
        .await
        .unwrap_err();
    assert_eq!(code_of(&err), 77, "{err:#}");
    assert_eq!(rig.slots.len(), 1);
    let up = rig.hub.status();
    assert_eq!(row(&up, 1)["state"], "up", "{up}");
}

#[tokio::test]
async fn a_bad_link_or_stream_id_exits_64() {
    let rig = Rig::new(1, None).await;
    for (verb, args) in [
        ("close-link", json!({})),
        ("close-link", json!({"link": "one"})),
        ("close-link", json!({"link": 99})),
        ("close-stream", json!({"link": 1})),
        ("close-stream", json!({"link": 99, "stream": 1})),
        ("close-stream", json!({"link": 1, "stream": 4242})),
        ("select", json!({"link": 99})),
        ("select", json!({"link": "one"})),
        ("select", json!({})),
    ] {
        let err = rig.call(verb, args.clone()).await.unwrap_err();
        assert_eq!(code_of(&err), 64, "{verb} {args}: {err:#}");
    }
}

#[tokio::test]
async fn an_unknown_verb_exits_64_and_lists_the_verbs() {
    let rig = Rig::new(1, None).await;
    let err = rig.call("format-disk", json!({})).await.unwrap_err();
    assert_eq!(code_of(&err), 64);
    let text = format!("{err:#}");
    for verb in [
        "close-stream",
        "close-link",
        "select",
        "reconnect",
        "rotate",
        "dump",
    ] {
        assert!(
            text.contains(verb),
            "the list of verbs names {verb}: {text}"
        );
    }
}

#[tokio::test]
async fn a_listener_refuses_rotate_and_reconnect() {
    let rig = Rig::new(1, None).await;
    for verb in ["rotate", "reconnect"] {
        let err = rig.call(verb, json!({"link": 1})).await.unwrap_err();
        assert_eq!(code_of(&err), 69, "{verb}: {err:#}");
        assert!(
            format!("{err:#}").contains("a listener does not dial"),
            "{err:#}"
        );
    }
    let doc = rig.call("status", json!({})).await.unwrap();
    assert_eq!(row(&doc, 1)["state"], "up", "the refusal changed nothing");
}

#[tokio::test]
async fn close_stream_resets_one_stream_and_leaves_the_others() {
    let mut rig = Rig::new(1, None).await;
    let (mut conn_a, _, mut far_a) = rig.open().await;
    let (mut conn_b, _, mut far_b) = rig.open().await;
    let (id_a, id_b) = (u64::from(far_a.id()), u64::from(far_b.id()));
    let doc = rig.call("status", json!({})).await.unwrap();
    let ids = row(&doc, 1)["stream_ids"].clone();
    assert!(ids.to_string().contains(&id_a.to_string()), "{doc}");
    assert!(ids.to_string().contains(&id_b.to_string()), "{doc}");

    let doc = rig
        .call("close-stream", json!({"link": 1, "stream": id_a}))
        .await
        .unwrap();
    assert_eq!(doc["verb"], "close-stream", "{doc}");
    let ids = row(&doc, 1)["stream_ids"].as_array().unwrap().clone();
    assert!(
        !ids.contains(&json!(id_a)),
        "the stream is out of the table: {doc}"
    );
    assert!(ids.contains(&json!(id_b)), "{doc}");

    // The far end of the closed stream fails; the local connection ends.
    let mut buf = [0u8; 8];
    let read = tokio::time::timeout(Duration::from_secs(5), far_a.read(&mut buf))
        .await
        .expect("the far end reads the reset");
    assert!(
        read.is_err(),
        "a reset is an error at the far end: {read:?}"
    );
    let end = tokio::time::timeout(Duration::from_secs(5), conn_a.read(&mut buf))
        .await
        .expect("the local connection ends");
    assert!(matches!(end, Ok(0) | Err(_)), "{end:?}");

    // The other stream still carries bytes in both directions.
    conn_b.write_all(b"ping").await.unwrap();
    let mut got = [0u8; 4];
    tokio::time::timeout(Duration::from_secs(5), far_b.read_exact(&mut got))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&got, b"ping");
    far_b.write_all(b"pong").await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), conn_b.read_exact(&mut got))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&got, b"pong");
    // The link is still up.
    assert_eq!(row(&doc, 1)["state"], "up");
}

#[tokio::test]
async fn select_moves_new_streams_only() {
    let mut rig = Rig::new(2, None).await;
    // Auto: the first link that is up serves the first stream.
    let (mut conn_1, index, mut far_1) = rig.open().await;
    assert_eq!(index, 0, "the first link serves a stream before any select");

    let doc = rig.call("select", json!({"link": 2})).await.unwrap();
    assert_eq!(doc["verb"], "select", "{doc}");
    assert_eq!(doc["selected"], 2, "{doc}");

    // A new stream goes to link 2.
    let (_conn_2, index, _far_2) = rig.open().await;
    assert_eq!(index, 1, "a new stream follows the select");

    // The old stream stays on link 1 and still works.
    let doc = rig.call("status", json!({})).await.unwrap();
    assert_eq!(row(&doc, 1)["streams"], 1, "{doc}");
    conn_1.write_all(b"old").await.unwrap();
    let mut got = [0u8; 3];
    tokio::time::timeout(Duration::from_secs(5), far_1.read_exact(&mut got))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&got, b"old");

    // null returns to auto: the first link again.
    let doc = rig.call("select", json!({"link": null})).await.unwrap();
    assert_eq!(doc["selected"], Value::Null, "{doc}");
    let (_conn_3, index, _far_3) = rig.open().await;
    assert_eq!(index, 0);
}

#[tokio::test]
async fn select_refuses_a_link_that_has_ended_and_a_pin_falls_back_when_it_ends() {
    let rig = Rig::new(2, None).await;
    rig.call("close-link", json!({"link": 2})).await.unwrap();
    let err = rig.call("select", json!({"link": 2})).await.unwrap_err();
    assert_eq!(code_of(&err), 69, "{err:#}");
    rig.call("select", json!({"link": 1})).await.unwrap();
    rig.call("close-link", json!({"link": 1})).await.unwrap();
    // No link is up: the open exits 69, as before.
    let err = Client::connect(&rig.path)
        .await
        .unwrap()
        .open(rig.control.token(), &rig.peer, "raw")
        .await
        .unwrap_err();
    assert_eq!(code_of(&err), 69, "{err:#}");
}

// The commands: a listener process holds the streams of a dialer process.

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
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

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

struct Proc(Child);

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// A detached listener `name` at `sock` that admits the `allowed`
/// fingerprints and sends each stream to an echo origin. Its control socket.
fn listener(server: &Host, allowed: &[&str], name: &str, sock: &Path) -> PathBuf {
    let allow = server.path(&format!("allow-{name}"));
    std::fs::write(&allow, allowed.join("\n")).unwrap();
    let origin = server.path("origin.sock");
    if !origin.exists() {
        echo(&origin);
    }
    let to = format!("unix:{}", origin.display());
    let out = server.run(&[
        "link",
        "listen",
        sock.to_str().unwrap(),
        "--allow",
        allow.to_str().unwrap(),
        "--detach",
        "--name",
        name,
        "--to",
        &to,
    ]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let ready: Value = serde_json::from_str(text(&out.stdout).lines().last().unwrap()).unwrap();
    PathBuf::from(ready["control"].as_str().unwrap())
}

/// The live status of the instance `name` until its first link shows a
/// stream.
fn wait_for_stream(server: &Host, name: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let out = server.run(&["link", "status", name, "--json"]);
        let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
        let row = doc["links"][0]["live"]["links"][0].clone();
        if row["stream_ids"].as_array().is_some_and(|s| !s.is_empty()) || Instant::now() > deadline
        {
            return row;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn close_stream_ends_the_stream_of_a_peer_of_a_listener_process() {
    let server = Host::new();
    let sfp = server.fingerprint();
    // This process is the dialer: it holds its own key and reads its end of
    // the stream.
    let dir = tempfile::tempdir().unwrap();
    let me = install(dir.path(), "inproc");
    let sock = server.path("m.sock");
    let control = listener(&server, &[&me.fingerprint()], "lm", &sock);

    let carrier = podnet::carriers::open("unix", &podnet::carriers::Options::default()).unwrap();
    let link = link::dial(&*carrier, sock.to_str().unwrap(), Settings::default())
        .await
        .unwrap();
    hello::dial(&link, &me, AllowList::parse(&sfp).unwrap())
        .await
        .unwrap();
    let mut stream = link.open().unwrap();
    link_service::request(&mut stream, "raw").await.unwrap();
    stream.write_all(b"hello").await.unwrap();
    let mut got = [0u8; 5];
    stream.read_exact(&mut got).await.unwrap();
    assert_eq!(&got, b"hello", "the origin answers through the stream");

    let id = u64::from(stream.id());
    let row = tokio::task::block_in_place(|| wait_for_stream(&server, "lm"));
    assert!(
        row["stream_ids"].to_string().contains(&id.to_string()),
        "the listener tracks the stream {id}: {row}"
    );
    let token = std::fs::read_to_string(format!("{}.token", control.display())).unwrap();
    let doc = Client::connect(&control)
        .await
        .unwrap()
        .call(
            token.trim(),
            "close-stream",
            json!({"link": row["id"], "stream": id}),
        )
        .await
        .unwrap();
    assert_eq!(doc["verb"], "close-stream", "{doc}");
    assert_eq!(doc["links"][0]["state"], "up", "{doc}");
    assert!(
        !doc["links"][0]["stream_ids"]
            .to_string()
            .contains(&id.to_string()),
        "{doc}"
    );
    // The stream of the peer ends with a reset; the link stays.
    let mut buf = [0u8; 8];
    let end = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buf))
        .await
        .expect("the reset reaches the peer");
    assert!(end.is_err(), "a reset is an error at the peer: {end:?}");
    assert!(link.ended().is_none(), "the link stays up");
    tokio::task::block_in_place(|| {
        let _ = server.run(&["link", "close", "lm"]);
    });
}

#[test]
fn link_close_uses_the_close_link_verb_when_the_socket_answers() {
    let (server, client) = (Host::new(), Host::new());
    let (sfp, cfp) = (server.fingerprint(), client.fingerprint());
    let sock = server.path("l.sock");
    let control = listener(&server, &[&cfp], "lk", &sock);
    // A dialer process that holds its link up.
    let dial = Proc(
        client
            .command(&["link", "dial", sock.to_str().unwrap(), "--peer", &sfp])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let _ = wait_for_stream(&server, "lk");
    let out = server.run(&["link", "close", "lk"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let shown = text(&out.stdout);
    assert!(
        shown.contains("closed link 1") && shown.contains(&cfp),
        "the command names the link that the verb closed: {shown}"
    );
    assert!(!control.exists(), "the instance stopped");
    drop(dial);
}

/// A detached `link connect` of `client` to the listener at `sock`.
fn connector(client: &Host, peers: &[&str], name: &str, sock: &Path) {
    let mut args = vec!["link", "connect", sock.to_str().unwrap()];
    for peer in peers {
        args.extend(["--peer", peer]);
    }
    args.extend(["--detach", "--name", name]);
    let out = client.run(&args);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
}

/// The first link row of the live status of the instance `name`.
fn live_row(host: &Host, name: &str) -> Value {
    let out = host.run(&["link", "status", name, "--json"]);
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    doc["links"][0]["live"]["links"][0].clone()
}

#[test]
fn reconnect_and_rotate_of_a_connect_process_keep_the_name_and_a_peer_that_changed_exits_77() {
    let (server, client, other) = (Host::new(), Host::new(), Host::new());
    let (sfp, cfp, ofp) = (
        server.fingerprint(),
        client.fingerprint(),
        other.fingerprint(),
    );
    let sock = server.path("c.sock");
    listener(&server, &[&cfp], "ls", &sock);
    // --peer holds both listener keys: the name and the key of the old link
    // are what stop the second one.
    connector(&client, &[&sfp, &ofp], "cn", &sock);
    let first = live_row(&client, "cn");
    assert_eq!(first["id"], 1, "{first}");
    assert_eq!(first["peer"], sfp.as_str(), "{first}");

    let out = client.run(&["link", "reconnect", "cn"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(
        text(&out.stdout).contains("reconnect link 1 -> 2"),
        "{}",
        text(&out.stdout)
    );
    let row = live_row(&client, "cn");
    assert_eq!(row["id"], 2, "{row}");
    assert_eq!(
        row["peer"],
        sfp.as_str(),
        "the name keeps its fingerprint: {row}"
    );

    let out = client.run(&["link", "rotate", "cn"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(
        text(&out.stdout).contains("rotate link 2 -> 3"),
        "{}",
        text(&out.stdout)
    );
    let end = Instant::now() + Duration::from_secs(5);
    let row = loop {
        let row = live_row(&client, "cn");
        if row["id"] == 3 || Instant::now() > end {
            break row;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(row["id"], 3, "the old link left after its streams: {row}");

    // Another listener, with another key, takes the address. The dial passes
    // the allow list, and the check of the old key refuses it. The old link
    // stays up.
    std::fs::rename(&sock, server.path("moved.sock")).unwrap();
    listener(&other, &[&cfp], "lo", &sock);
    let out = client.run(&["link", "rotate", "cn"]);
    assert_eq!(out.status.code(), Some(77), "{}", text(&out.stderr));
    assert!(
        text(&out.stderr).contains("not the key"),
        "{}",
        text(&out.stderr)
    );
    let row = live_row(&client, "cn");
    assert_eq!(
        (&row["id"], &row["peer"]),
        (&json!(3), &json!(sfp)),
        "{row}"
    );
    let _ = server.run(&["link", "close", "ls"]);
    let _ = other.run(&["link", "close", "lo"]);
    let _ = client.run(&["link", "close", "cn"]);
}
