//! The state dump of the link control socket (T-309, GitHub #476, row
//! E10-4): the verb `dump` gives the carrier, the peer fingerprint, the
//! counters, the stream table, the settings, the last 20 errors and the
//! build features. It holds no seed, no token and no secret of a peer; each
//! string passes the redaction of `report` and the strip of control
//! characters. The cap is 256 KiB, and a larger table is cut with a count.
//! The tests use unix sockets and memory pairs only: they bind no TCP port.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use podnet::exit::code_of;
use podnet::hello;
use podnet::identity::{self, AllowList, Identity};
use podnet::link::{self, Link, Settings, Stream};
use podnet::link_ctl::{Client, Control, Hub, Slot, DUMP_LIMIT, RESPONSE_LIMIT};
use serde_json::{json, Value};

fn install(dir: &Path, name: &str) -> Identity {
    let sub = dir.join(name);
    std::fs::create_dir(&sub).unwrap();
    identity::create(&sub).unwrap()
}

struct Rig {
    control: Control,
    hub: Arc<Hub>,
    path: PathBuf,
    me: Identity,
    peer: Identity,
    slot: Slot,
    near: Arc<Link>,
    _far: Link,
    _dir: tempfile::TempDir,
}

impl Rig {
    /// A hub with one link to a peer. `settings` is for both ends.
    async fn new(settings: Settings) -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let (me, peer) = (install(dir.path(), "me"), install(dir.path(), "peer"));
        let list = |k: &Identity| AllowList::parse(&k.fingerprint()).unwrap();
        let hub = Hub::new(&me.fingerprint());
        let path = dir.path().join("ctl.sock");
        let control = Control::start(&path, hub.clone()).unwrap();
        let (far, near) = link::memory_pair(1 << 20, settings).unwrap();
        let (d, l) = tokio::join!(
            hello::dial(&far, &peer, list(&me)),
            hello::accept(&near, &me, list(&peer))
        );
        let (_, peer_key) = (d.unwrap(), l.unwrap());
        let near = Arc::new(near);
        let slot = hub.hold(near.clone(), peer_key, "memory", "pair0", "listener");
        Rig {
            control,
            hub,
            path,
            me,
            peer,
            slot,
            near,
            _far: far,
            _dir: dir,
        }
    }

    async fn dump(&self) -> anyhow::Result<Value> {
        Client::connect(&self.path)
            .await?
            .call(self.control.token(), "dump", json!({}))
            .await
    }

    /// Open and track `n` streams of the link.
    fn streams(&self, n: usize) -> Vec<(Stream, podnet::link_ctl::Tracked)> {
        (0..n)
            .map(|_| {
                let stream = self.near.open().unwrap();
                let tracked = self.slot.track(&stream);
                (stream, tracked)
            })
            .collect()
    }
}

#[tokio::test]
async fn the_dump_holds_the_carrier_the_peer_the_counters_the_streams_and_the_build() {
    let rig = Rig::new(Settings::default()).await;
    let _held = rig.streams(3);
    rig.slot.count_accepted();
    let doc = rig.dump().await.unwrap();
    assert_eq!(doc["ok"], true, "{doc}");
    assert_eq!(doc["verb"], "dump");
    assert_eq!(doc["fingerprint"], rig.me.fingerprint().as_str());
    assert!(doc["pid"].is_u64(), "{doc}");
    let link = &doc["links"][0];
    assert_eq!(link["carrier"], "memory", "{doc}");
    assert_eq!(link["address"], "pair0");
    assert_eq!(link["role"], "listener");
    assert_eq!(link["peer"], rig.peer.fingerprint().as_str());
    assert_eq!(link["state"], "up");
    assert_eq!(link["streams"], 3);
    assert_eq!(link["accepted"], 1);
    let table = link["stream_table"].as_array().unwrap();
    assert_eq!(table.len(), 3, "{doc}");
    for row in table {
        assert!(row["id"].is_u64(), "{row}");
        // The hub side of this link is the listener: it opened even ids.
        assert!(row["owner"] == "local" || row["owner"] == "peer", "{row}");
    }
    assert_eq!(link["streams_cut"], 0);
    assert_eq!(doc["totals"]["links"], 1, "{doc}");
    assert_eq!(doc["totals"]["streams"], 3);
    assert_eq!(doc["totals"]["accepted"], 1);
    // The build: the version, the optional features, the carriers.
    assert_eq!(doc["build"]["version"], env!("CARGO_PKG_VERSION"));
    let enabled: Vec<&str> = podnet::build_features::enabled();
    assert_eq!(doc["build"]["features"], json!(enabled), "{doc}");
    let names: Vec<&str> = doc["build"]["carriers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, podnet::carriers::names());
    // The settings and the limits; the route table waits for T-286.
    assert_eq!(doc["settings"]["selected"], Value::Null, "{doc}");
    assert_eq!(doc["settings"]["limits"]["response"], RESPONSE_LIMIT);
    assert_eq!(doc["settings"]["limits"]["dump"], DUMP_LIMIT);
    assert_eq!(doc["routes"]["built"], false, "{doc}");
    assert_eq!(doc["errors"], json!([]));
}

#[tokio::test]
async fn the_dump_needs_the_token_and_the_user_check() {
    let rig = Rig::new(Settings::default()).await;
    let err = Client::connect(&rig.path)
        .await
        .unwrap()
        .call(&"0".repeat(64), "dump", json!({}))
        .await
        .unwrap_err();
    assert_eq!(code_of(&err), 77, "{err:#}");
    // A refused user, with the right token.
    let dir = tempfile::tempdir().unwrap();
    let me = install(dir.path(), "me");
    let hub = Hub::new(&me.fingerprint());
    let path = dir.path().join("gate.sock");
    let control = Control::start_gated(&path, hub, Arc::new(|_| false)).unwrap();
    let err = Client::connect(&path)
        .await
        .unwrap()
        .call(control.token(), "dump", json!({}))
        .await
        .unwrap_err();
    assert_eq!(code_of(&err), 77, "{err:#}");
}

#[tokio::test]
async fn the_dump_holds_no_seed_no_token_and_no_secret() {
    let rig = Rig::new(Settings::default()).await;
    let seed_me = std::fs::read_to_string(rig.me.path()).unwrap();
    let seed_peer = std::fs::read_to_string(rig.peer.path()).unwrap();
    let token = rig.control.token().to_string();
    // Text that came from outside lands in the errors: it must not leak.
    rig.hub
        .note_error(&format!("the key file said {}", seed_me.trim()));
    rig.hub
        .note_error(&format!("the peer file said {}", seed_peer.trim()));
    rig.hub.note_error(&format!("the token {token} was seen"));
    rig.hub
        .note_error("proxy https://user:hunter2@proxy.example:3128 refused");
    rig.hub.note_error("Authorization: Bearer abcdef0123456789");
    rig.hub
        .note_error(&format!("a hex run {}", "ab".repeat(40)));
    let doc = rig.dump().await.unwrap();
    let shown = doc.to_string();
    for secret in [
        seed_me.trim(),
        seed_peer.trim(),
        token.as_str(),
        "hunter2",
        "abcdef0123456789",
    ] {
        assert!(
            !shown.contains(secret),
            "the dump holds {secret:?}: {shown}"
        );
    }
    assert!(!shown.contains(&"ab".repeat(40)));
    assert!(shown.contains("<hidden>"), "{shown}");
    assert!(!shown.contains("token\":\""), "no token field: {shown}");
}

#[tokio::test]
async fn every_string_loses_its_control_characters() {
    let rig = Rig::new(Settings::default()).await;
    rig.hub
        .note_error("\u{1b}[31mred\u{1b}[0m and \u{7}bell\r\nline");
    let doc = rig.dump().await.unwrap();
    let text = doc["errors"][0]["text"].as_str().unwrap();
    assert!(
        !text
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t'),
        "{text:?}"
    );
    assert!(text.contains("red") && text.contains("bell"), "{text:?}");
    assert!(!doc.to_string().contains("\\u001b"), "{doc}");
}

#[tokio::test]
async fn the_dump_keeps_the_last_20_errors_in_order() {
    let rig = Rig::new(Settings::default()).await;
    for n in 0..30 {
        rig.hub.note_error(&format!("error {n}"));
    }
    let doc = rig.dump().await.unwrap();
    let errors = doc["errors"].as_array().unwrap();
    assert_eq!(errors.len(), 20, "{doc}");
    assert_eq!(errors[0]["text"], "error 10");
    assert_eq!(errors[19]["text"], "error 29");
    assert!(errors[0]["at"].is_u64());
    // A long text is cut.
    rig.hub.note_error(&"x".repeat(5000));
    let doc = rig.dump().await.unwrap();
    let last = doc["errors"][19]["text"].as_str().unwrap();
    assert!(last.len() < 1000, "{}", last.len());
}

#[tokio::test]
async fn a_table_over_the_cap_is_cut_with_a_count() {
    let rig = Rig::new(Settings::default()).await;
    let _held = rig.streams(200);
    let whole = rig.hub.dump_within(usize::MAX);
    assert_eq!(
        whole["links"][0]["stream_table"].as_array().unwrap().len(),
        200
    );
    assert_eq!(whole["cut"]["stream_rows"], 0);
    let cap = 3000;
    let doc = rig.hub.dump_within(cap);
    let text = doc.to_string();
    assert!(text.len() <= cap, "{} > {cap}", text.len());
    let shown = doc["links"][0]["stream_table"].as_array().unwrap().len();
    let cut = doc["links"][0]["streams_cut"].as_u64().unwrap();
    assert!(shown < 200, "{shown}");
    assert_eq!(shown as u64 + cut, 200, "{doc}");
    assert_eq!(doc["cut"]["stream_rows"], cut, "{doc}");
    // The counters are whole: the cut takes rows, not counts.
    assert_eq!(doc["links"][0]["streams"], 200);
    assert_eq!(doc["totals"]["streams"], 200);
}

#[tokio::test]
async fn an_answer_over_64_kib_and_under_the_cap_reaches_the_client() {
    let settings = Settings {
        max_streams: 4096,
        ..Settings::default()
    };
    let rig = Rig::new(settings).await;
    let _held = rig.streams(4000);
    let doc = rig.dump().await.unwrap();
    let size = doc.to_string().len();
    assert!(
        size > RESPONSE_LIMIT && size < DUMP_LIMIT,
        "the dump is {size} bytes"
    );
    assert_eq!(
        doc["links"][0]["stream_table"].as_array().unwrap().len(),
        4000
    );
    // Every other verb keeps the limit of 64 KiB.
    let status = Client::connect(&rig.path)
        .await
        .unwrap()
        .call(rig.control.token(), "status", json!({}))
        .await
        .unwrap();
    assert!(status.to_string().len() < RESPONSE_LIMIT);
}

// The command: `podnet bug-report --include-link` asks the control socket of
// each running instance for its dump.

fn host_run(dir: &Path, args: &[&str]) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(args)
        .env("PODNET_STATE_DIR", dir.join("state"))
        .env("PODNET_RUN_DIR", dir.join("run"))
        .env_remove("RUST_LOG")
        .output()
        .unwrap()
}

#[test]
fn bug_report_include_link_adds_the_dump_of_a_running_instance_and_no_token() {
    let dir = tempfile::tempdir().unwrap();
    let status = host_run(dir.path(), &["link", "status", "--create", "--json"]);
    let doc: Value = serde_json::from_slice(&status.stdout).unwrap();
    let fingerprint = doc["identity"]["fingerprint"].as_str().unwrap().to_string();
    let allow = dir.path().join("allow");
    std::fs::write(&allow, &fingerprint).unwrap();
    let sock = dir.path().join("l.sock");
    let out = host_run(
        dir.path(),
        &[
            "link",
            "listen",
            sock.to_str().unwrap(),
            "--allow",
            allow.to_str().unwrap(),
            "--detach",
            "--name",
            "lk",
        ],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let ready: Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).lines().last().unwrap()).unwrap();
    let control = ready["control"].as_str().unwrap().to_string();
    let token = std::fs::read_to_string(format!("{control}.token")).unwrap();

    let without = host_run(dir.path(), &["bug-report"]);
    let without = String::from_utf8_lossy(&without.stdout).into_owned();
    assert!(
        !without.contains("podnet link dump"),
        "no section without the flag"
    );

    let out = host_run(dir.path(), &["bug-report", "--include-link"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(text.contains("## podnet link dump"), "{text}");
    assert!(
        text.contains(&fingerprint),
        "the dump names this install: {text}"
    );
    assert!(text.contains("\"verb\": \"dump\""), "{text}");
    assert!(text.contains("\"features\""), "{text}");
    assert!(!text.contains(token.trim()), "the token is in the report");
    let seed = std::fs::read_to_string(dir.path().join("state").join("link-identity.key")).unwrap();
    assert!(!text.contains(seed.trim()), "the seed is in the report");

    let _ = host_run(dir.path(), &["link", "close", "lk"]);
    let out = host_run(dir.path(), &["bug-report", "--include-link"]);
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(text.contains("(no link instance runs)"), "{text}");
}

/// A mesh with one learned origin, named with text that must not leak.
fn learned_mesh(dir: &Path, name: &str) -> (Arc<podnet::link_announce::Mesh>, Identity) {
    use podnet::link_announce::{now_ms, Mesh};
    let a = Arc::new(install(dir, "mesh-a"));
    let b = Arc::new(install(dir, "mesh-b"));
    let ma = Mesh::new(a.clone(), vec!["socks5".into()], now_ms());
    let mb = Mesh::new(b.clone(), vec!["raw".into()], now_ms());
    mb.neighbour_up(a.public(), 3);
    ma.neighbour_up(b.public(), 3);
    let wire = mb.make(now_ms()).unwrap();
    ma.receive(&b.public(), 3, &wire, now_ms()).unwrap();
    assert!(ma.set_name(b.public(), name));
    (ma, identity::open(&dir.join("mesh-b")).unwrap())
}

#[tokio::test]
async fn the_dump_of_an_instance_with_announce_holds_the_route_table() {
    let rig = Rig::new(Settings::default()).await;
    let dir = tempfile::tempdir().unwrap();
    let (mesh, origin) = learned_mesh(dir.path(), "edge-node");
    rig.hub.attach_mesh(mesh);
    let doc = rig.dump().await.unwrap();
    let routes = &doc["routes"];
    assert_eq!(routes["built"], true, "{doc}");
    assert!(routes.get("reason").is_none(), "{routes}");
    assert!(routes.get("verb").is_none(), "{routes}");
    let nodes = routes["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 2, "{routes}");
    assert_eq!(nodes[0]["self"], true);
    let far = &nodes[1];
    assert_eq!(far["fingerprint"], origin.fingerprint().as_str(), "{far}");
    assert_eq!(far["name"], "edge-node");
    assert_eq!(far["hops"], 1);
    assert_eq!(far["cost"], 3);
    assert_eq!(far["state"], "up");
    assert_eq!(routes["counters"]["announcements_received"], 1);
    assert_eq!(routes["truncated"]["nodes"], 0);
}

#[tokio::test]
async fn the_dump_without_announce_says_why_the_routes_are_not_built() {
    let rig = Rig::new(Settings::default()).await;
    let doc = rig.dump().await.unwrap();
    assert_eq!(doc["routes"]["built"], false, "{doc}");
    let reason = doc["routes"]["reason"].as_str().unwrap();
    assert!(reason.contains("--announce"), "{reason}");
    assert!(doc["routes"].get("nodes").is_none());
}

#[tokio::test]
async fn the_route_table_of_the_dump_is_redacted() {
    let rig = Rig::new(Settings::default()).await;
    let dir = tempfile::tempdir().unwrap();
    let hex = "ab".repeat(32);
    let (mesh, _) = learned_mesh(dir.path(), &hex);
    rig.hub.attach_mesh(mesh);
    let seed = std::fs::read_to_string(dir.path().join("mesh-a").join("link-identity.key"))
        .unwrap_or_default();
    let shown = rig.dump().await.unwrap().to_string();
    assert!(!shown.contains(&hex), "{shown}");
    assert!(shown.contains("<hidden>"), "{shown}");
    if !seed.trim().is_empty() {
        assert!(!shown.contains(seed.trim()));
    }
}
