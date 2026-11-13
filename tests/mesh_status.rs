//! The local topology and status view of a mesh (T-289): the verb `mesh`
//! of the control socket and `podnet link mesh`. Three nodes A, B and C
//! (A-B and B-C) cross unix links in one process; A has a control socket.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pod_identity::PublicKey;
use podnet::identity::{self, Identity};
use podnet::link::{self, Link, Settings};
use podnet::link_announce::{self as announce, Guard, Mesh, ViewLimits};
use podnet::link_ctl::{Client, Control, Hub};
use podnet::link_service as service;
use serde_json::{json, Value};

const FAST: u64 = 1000;

struct Node {
    id: Arc<Identity>,
    mesh: Arc<Mesh>,
    guards: Vec<Guard>,
}

fn node(dir: &Path, name: &str, services: &[&str]) -> Node {
    let sub = dir.join(name);
    std::fs::create_dir(&sub).unwrap();
    let id = Arc::new(identity::create(&sub).unwrap());
    let services = services.iter().map(|s| s.to_string()).collect();
    let mesh = Mesh::new(id.clone(), services, announce::now_ms());
    Node {
        id,
        mesh,
        guards: Vec::new(),
    }
}

async fn serve(mesh: Arc<Mesh>, link: Arc<Link>, peer: PublicKey, weight: u32) {
    while let Ok(mut stream) = link.accept().await {
        let (mesh, link) = (mesh.clone(), link.clone());
        tokio::spawn(async move {
            match service::accept(&mut stream).await {
                Ok(name) if name == announce::NAME => {
                    announce::receive_stream(mesh, link, peer, Some(weight), &mut stream).await;
                }
                _ => {
                    let _ = service::reply(&mut stream, service::NO_SERVICE).await;
                }
            }
        });
    }
}

async fn join(x: &mut Node, y: &mut Node, cost: u32) {
    let (lx, ly) = link::unix_pair(Settings::default()).unwrap();
    let (lx, ly) = (Arc::new(lx), Arc::new(ly));
    tokio::spawn(serve(x.mesh.clone(), lx.clone(), y.id.public(), cost));
    tokio::spawn(serve(y.mesh.clone(), ly.clone(), x.id.public(), cost));
    x.guards
        .push(Guard::start(&x.mesh, &lx, y.id.public(), Some(cost), FAST));
    y.guards
        .push(Guard::start(&y.mesh, &ly, x.id.public(), Some(cost), FAST));
}

fn fp(n: &Node) -> String {
    n.id.public().display_fingerprint()
}

struct World {
    _dir: tempfile::TempDir,
    a: Node,
    b: Node,
    c: Node,
    socket: PathBuf,
    _control: Control,
}

/// A-B (cost 3) and B-C (cost 5). A serves the control socket. The world
/// waits until A knows C and the claims of B and C.
async fn world() -> World {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, mut b, mut c) = (
        node(dir.path(), "a", &["socks5"]),
        node(dir.path(), "b", &[]),
        node(dir.path(), "c", &["echo", "raw"]),
    );
    join(&mut a, &mut b, 3).await;
    join(&mut b, &mut c, 5).await;
    let hub = Hub::new(&a.id.fingerprint());
    hub.attach_mesh(a.mesh.clone());
    let socket = dir.path().join("ctl.sock");
    let control = Control::start(&socket, hub).unwrap();
    let w = World {
        _dir: dir,
        a,
        b,
        c,
        socket,
        _control: control,
    };
    let end = Instant::now() + Duration::from_secs(15);
    loop {
        let view = w.a.mesh.view(announce::now_ms(), ViewLimits::default());
        let ready = view["nodes"].as_array().unwrap().len() == 3
            && view["nodes"].as_array().unwrap().iter().all(|n| {
                // B claims A and C; C claims B.
                let want = if n["fingerprint"] == fp(&w.b) { 2 } else { 1 };
                n["self"] == true || n["edges"].as_array().unwrap().len() == want
            });
        if ready {
            break;
        }
        assert!(Instant::now() < end, "no mesh: {view}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    w
}

async fn ask(socket: &Path, args: Value) -> anyhow::Result<Value> {
    let token = podnet::link_ctl::read_token(&podnet::link_ctl::token_path(socket)).unwrap();
    Client::connect(socket)
        .await?
        .call(&token, "mesh", args)
        .await
}

fn podnet(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(args)
        .env_remove("RUST_LOG")
        .output()
        .unwrap()
}

async fn cli(args: Vec<String>) -> Output {
    tokio::task::spawn_blocking(move || {
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        podnet(&refs)
    })
    .await
    .unwrap()
}

fn node_of<'a>(view: &'a Value, fp: &str) -> &'a Value {
    view["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["fingerprint"] == fp)
        .unwrap_or_else(|| panic!("no node {fp}: {view}"))
}

#[tokio::test(flavor = "multi_thread")]
async fn mesh_json_has_the_nodes_and_the_edges() {
    let w = world().await;
    let view = ask(&w.socket, json!({})).await.unwrap();
    assert_eq!(view["ok"], true);
    assert_eq!(view["verb"], "mesh");
    assert_eq!(view["node"], fp(&w.a));
    assert_eq!(view["limits"]["nodes"], 4096);
    assert_eq!(view["limits"]["edges_per_node"], 64);
    assert_eq!(view["limits"]["bytes"], 256 * 1024 - 1);
    let nodes = view["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 3, "{view}");
    // This node is first.
    assert_eq!(nodes[0]["fingerprint"], fp(&w.a));
    assert_eq!(nodes[0]["self"], true);
    assert_eq!(nodes[0]["hops"], 0);
    assert_eq!(nodes[0]["services"], json!(["socks5"]));
    let own = nodes[0]["edges"].as_array().unwrap();
    assert_eq!(own.len(), 1);
    assert_eq!(own[0]["peer"], fp(&w.b));
    assert_eq!(own[0]["cost"], 3);
    assert_eq!(own[0]["state"], "up");
    assert!(own[0]["last_probe_ms"].is_u64(), "{view}");
    // B: one hop, the cost of the link, and two claimed neighbours.
    let b = node_of(&view, &fp(&w.b));
    assert_eq!(b["hops"], 1);
    assert_eq!(b["cost"], 3);
    assert_eq!(b["state"], "up");
    assert_eq!(b["name"], Value::Null);
    assert!(b["seq"].as_u64().unwrap() > 0);
    assert_eq!(b["next_hop"], fp(&w.b));
    let claimed: Vec<&str> = b["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["peer"].as_str().unwrap())
        .collect();
    assert!(claimed.contains(&fp(&w.a).as_str()) || claimed.contains(&fp(&w.c).as_str()));
    assert!(b["edges"]
        .as_array()
        .unwrap()
        .iter()
        .all(|e| e["state"] == "claimed" && e["cost"].is_u64()));
    // C: two hops, through B, with its services.
    let c = node_of(&view, &fp(&w.c));
    assert_eq!(c["hops"], 2);
    assert_eq!(c["cost"], 8);
    assert_eq!(c["next_hop"], fp(&w.b));
    assert_eq!(c["services"], json!(["echo", "raw"]));
    assert_eq!(c["edges"].as_array().unwrap()[0]["peer"], fp(&w.b));
    assert_eq!(
        view["truncated"],
        json!({"nodes": 0, "edges": 0, "bytes": 0})
    );
    assert_eq!(view["counters"]["nodes"], 3);
    assert_eq!(view["counters"]["neighbours"], 1);
    // A local name shows in the view.
    assert!(w.a.mesh.set_name(w.c.id.public(), "node-c"));
    assert!(!w.a.mesh.set_name(w.c.id.public(), ""));
    let view = ask(&w.socket, json!({})).await.unwrap();
    assert_eq!(node_of(&view, &fp(&w.c))["name"], "node-c");
    let _ = &w.b;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_command_prints_json_text_and_metrics_from_a_socket_of_mode_0600() {
    let w = world().await;
    let mode = std::fs::metadata(&w.socket).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    let control = format!("unix:{}", w.socket.display());
    let out = cli(vec![
        "link".into(),
        "mesh".into(),
        "--control".into(),
        control.clone(),
        "--json".into(),
    ])
    .await;
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let view: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(view["verb"], "mesh");
    assert_eq!(view["nodes"].as_array().unwrap().len(), 3);
    assert!(view["counters"]["announcements_received"].as_u64().unwrap() >= 2);
    let out = cli(vec![
        "link".into(),
        "mesh".into(),
        "--control".into(),
        control.clone(),
        "--metrics".into(),
    ])
    .await;
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(
        text.contains("# TYPE podnet_mesh_nodes gauge\npodnet_mesh_nodes 3\n"),
        "{text}"
    );
    assert!(text.contains("podnet_mesh_neighbours 1\n"), "{text}");
    assert!(text.contains("# TYPE podnet_mesh_announcements_received_total counter\n"));
    assert!(text.contains("podnet_mesh_announcements_dropped_total 0\n"));
    assert!(text.contains("podnet_mesh_truncated_nodes 0\n"));
    assert!(text
        .lines()
        .all(|l| l.starts_with('#') || l.split(' ').count() == 2));
    let out = cli(vec![
        "link".into(),
        "mesh".into(),
        "--control".into(),
        control,
    ])
    .await;
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains(&format!("node  {}", fp(&w.c))), "{text}");
    assert!(text.contains("edge  "), "{text}");
    // --json and --metrics together, and a control that is no unix socket.
    let both = cli(vec![
        "link".into(),
        "mesh".into(),
        "--control".into(),
        "unix:/x".into(),
        "--json".into(),
        "--metrics".into(),
    ])
    .await;
    assert!(!both.status.success());
    let bad = cli(vec![
        "link".into(),
        "mesh".into(),
        "--control".into(),
        "127.0.0.1:1".into(),
    ])
    .await;
    assert_eq!(bad.status.code(), Some(64));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_view_over_the_limits_is_cut_with_a_count() {
    let w = world().await;
    // Two nodes of three.
    let view = ask(&w.socket, json!({"max_nodes": 2})).await.unwrap();
    assert_eq!(view["nodes"].as_array().unwrap().len(), 2);
    assert_eq!(view["truncated"]["nodes"], 1);
    assert_eq!(view["limits"]["nodes"], 2);
    assert_eq!(view["counters"]["nodes"], 3);
    assert_eq!(view["nodes"][0]["self"], true);
    // One edge for each node. A and C have one edge each. B claims two.
    let full = ask(&w.socket, json!({})).await.unwrap();
    let total: usize = full["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["edges"].as_array().unwrap().len())
        .sum();
    let view = ask(&w.socket, json!({"max_edges": 1})).await.unwrap();
    assert!(view["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .all(|n| n["edges"].as_array().unwrap().len() <= 1));
    let kept: usize = view["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["edges"].as_array().unwrap().len())
        .sum();
    assert_eq!(
        view["truncated"]["edges"].as_u64().unwrap() as usize,
        total - kept
    );
    assert!(total > kept, "B claims two edges: {full}");
    // A request cannot raise a limit.
    let view = ask(&w.socket, json!({"max_nodes": 100000, "max_edges": 100000}))
        .await
        .unwrap();
    assert_eq!(view["limits"]["nodes"], 4096);
    assert_eq!(view["limits"]["edges_per_node"], 64);
    // A bad value exits 64.
    let err = ask(&w.socket, json!({"max_nodes": "many"}))
        .await
        .unwrap_err();
    assert_eq!(podnet::exit::code_of(&err), podnet::exit::USAGE);
    // The command takes the same limits.
    let out = cli(vec![
        "link".into(),
        "mesh".into(),
        "--control".into(),
        format!("unix:{}", w.socket.display()),
        "--json".into(),
        "--max-nodes".into(),
        "1".into(),
    ])
    .await;
    let view: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(view["nodes"].as_array().unwrap().len(), 1);
    assert_eq!(view["truncated"]["nodes"], 2);
    // The size limit: the document fits and the count says what was cut.
    let small = ViewLimits {
        bytes: 1500,
        ..ViewLimits::default()
    };
    let view = w.a.mesh.view(announce::now_ms(), small);
    assert!(
        view.to_string().len() <= small.bytes,
        "{} bytes",
        view.to_string().len()
    );
    assert!(view["truncated"]["bytes"].as_u64().unwrap() >= 1, "{view}");
    assert!(view["nodes"].as_array().unwrap().len() < 3);
    assert_eq!(view["nodes"][0]["self"], true);
}

#[tokio::test(flavor = "multi_thread")]
async fn without_announce_the_verb_exits_69() {
    let dir = tempfile::tempdir().unwrap();
    let id = identity::create(dir.path()).unwrap();
    let hub = Hub::new(&id.fingerprint());
    let socket = dir.path().join("ctl.sock");
    let _control = Control::start(&socket, hub).unwrap();
    let err = ask(&socket, json!({})).await.unwrap_err();
    assert_eq!(podnet::exit::code_of(&err), podnet::exit::UNAVAILABLE);
    assert!(format!("{err:#}").contains("--announce"));
}
