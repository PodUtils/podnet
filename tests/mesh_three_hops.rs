//! The forwarding of a session stream through the mesh (T-287): a stream
//! from A reaches a service of C through B, over unix links in one process.
//! The announcements of the other nodes are forwarded with the hop of each
//! node, the stream follows the best route, and the bounds hold: the hop
//! limit, the loop rule, the retry on `RESOURCE_LIMIT`, a closed middle link.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pod_identity::PublicKey;
use podnet::identity::{self, Identity};
use podnet::link::{self, Link, Settings};
use podnet::link_announce::{self as announce, Dropped, Guard, Mesh};
use podnet::link_forward::{self as forward, Arrived, ForwardError, Header};
use podnet::link_service as service;
use podnet::mesh_announce::{self as wire, Drop as Rule};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// The interval of the announcements in ms. Each one costs a signature check
/// at each node, which is slow in a debug build: a short interval starves the
/// streams. The first announcement of a link goes at once.
const FAST: u64 = 1000;

struct Node {
    id: Arc<Identity>,
    mesh: Arc<Mesh>,
    guards: Vec<Guard>,
}

fn node(dir: &Path, name: &str) -> Node {
    let sub = dir.join(name);
    std::fs::create_dir(&sub).unwrap();
    let id = Arc::new(identity::create(&sub).unwrap());
    let mesh = Mesh::new(id.clone(), vec!["echo".into()], announce::now_ms());
    Node {
        id,
        mesh,
        guards: Vec::new(),
    }
}

async fn until(what: &str, mut ok: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(10);
    while !ok() {
        assert!(Instant::now() < end, "timeout: {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// What a node does with the streams that a peer opens: the announcements,
/// the forwarded streams, and the service `echo`.
async fn serve(mesh: Arc<Mesh>, link: Arc<Link>, peer: PublicKey, weight: u32) {
    while let Ok(mut stream) = link.accept().await {
        let (mesh, link) = (mesh.clone(), link.clone());
        tokio::spawn(async move {
            let Ok(mut name) = service::accept(&mut stream).await else {
                return;
            };
            if name == announce::NAME {
                announce::receive_stream(mesh, link, peer, Some(weight), &mut stream).await;
                return;
            }
            if name == forward::NAME {
                match forward::accept(&mesh, &mut stream).await {
                    Ok(Arrived::Local(local)) => name = local,
                    _ => return,
                }
            }
            if name != "echo" {
                let _ = service::reply(&mut stream, service::NO_SERVICE).await;
                return;
            }
            let _ = service::reply(&mut stream, service::OPEN).await;
            let (mut r, mut w) = tokio::io::split(stream);
            let _ = tokio::io::copy(&mut r, &mut w).await;
            let _ = w.shutdown().await;
        });
    }
}

/// A link between two nodes. Each end has the cost `cost` for it.
async fn join(x: &mut Node, y: &mut Node, cost: u32) -> (Arc<Link>, Arc<Link>) {
    let (lx, ly) = link::unix_pair(Settings::default()).unwrap();
    let (lx, ly) = (Arc::new(lx), Arc::new(ly));
    tokio::spawn(serve(x.mesh.clone(), lx.clone(), y.id.public(), cost));
    tokio::spawn(serve(y.mesh.clone(), ly.clone(), x.id.public(), cost));
    x.guards
        .push(Guard::start(&x.mesh, &lx, y.id.public(), Some(cost), FAST));
    y.guards
        .push(Guard::start(&y.mesh, &ly, x.id.public(), Some(cost), FAST));
    (lx, ly)
}

fn route(mesh: &Mesh, to: &Node) -> Option<serde_json::Value> {
    let doc = mesh.routes(announce::now_ms());
    doc["routes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["origin"] == to.id.public().display_fingerprint())
        .cloned()
}

async fn has_route(from: &Node, to: &Node, hops: u64) {
    until("a route", || {
        route(&from.mesh, to).is_some_and(|r| r["hops"] == hops)
    })
    .await;
}

async fn echo_roundtrip(stream: link::Stream, data: Vec<u8>) -> Vec<u8> {
    let (mut r, mut w) = tokio::io::split(stream);
    let send = tokio::spawn(async move {
        w.write_all(&data).await.unwrap();
        w.shutdown().await.unwrap();
    });
    let mut back = Vec::new();
    r.read_to_end(&mut back).await.unwrap();
    send.await.unwrap();
    back
}

#[tokio::test]
async fn a_reaches_a_service_of_c_through_b() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, mut b, mut c) = (
        node(dir.path(), "a"),
        node(dir.path(), "b"),
        node(dir.path(), "c"),
    );
    join(&mut a, &mut b, 3).await;
    join(&mut b, &mut c, 5).await;
    has_route(&a, &c, 2).await;
    has_route(&c, &a, 2).await;
    let r = route(&a.mesh, &c).unwrap();
    assert_eq!(r["next_hop"], b.id.public().display_fingerprint());
    // The cost: the link A-B (3) and the signed cost of the hop of B (5).
    assert_eq!(r["cost"], 8, "{r}");
    // B forwarded the announcements of both ends.
    assert!(
        b.mesh.routes(announce::now_ms())["forwarded"]
            .as_u64()
            .unwrap()
            > 0
    );
    // 1 MiB goes through B and comes back equal.
    let mut data = vec![0u8; 1 << 20];
    let mut x = 0x2545_f491_4f6c_dd1du64;
    for byte in &mut data {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *byte = (x >> 24) as u8;
    }
    let stream = forward::open(&a.mesh, &c.id.public(), "echo", forward::DEFAULT_HOPS)
        .await
        .unwrap();
    let back = echo_roundtrip(stream, data.clone()).await;
    assert!(back == data, "the bytes differ");
    // A service that C does not have: the answer comes back through B.
    let err = forward::open(&a.mesh, &c.id.public(), "nope", 8)
        .await
        .unwrap_err();
    assert_eq!(err, ForwardError::NoService);
    // A direct neighbour gets the plain request.
    let stream = forward::open(&a.mesh, &b.id.public(), "echo", 1)
        .await
        .unwrap();
    assert_eq!(echo_roundtrip(stream, b"hi".to_vec()).await, b"hi");
}

#[tokio::test]
async fn the_hop_limit_refuses_a_longer_path() {
    let dir = tempfile::tempdir().unwrap();
    let mut n: Vec<Node> = ["a", "b", "c", "d", "e"]
        .iter()
        .map(|name| node(dir.path(), name))
        .collect();
    for i in 0..4 {
        let (left, right) = n.split_at_mut(i + 1);
        join(&mut left[i], &mut right[0], 2).await;
    }
    has_route(&n[0], &n[4], 4).await;
    let dest = n[4].id.public();
    for hops in [0, 1, 3] {
        let err = forward::open(&n[0].mesh, &dest, "echo", hops)
            .await
            .unwrap_err();
        assert_eq!(err, ForwardError::HopsExceeded, "hops {hops}");
    }
    let stream = forward::open(&n[0].mesh, &dest, "echo", 4).await.unwrap();
    assert_eq!(echo_roundtrip(stream, b"far".to_vec()).await, b"far");
    assert_eq!(forward::MAX_HOPS, 16);
    assert_eq!(forward::DEFAULT_HOPS, 8);
}

#[tokio::test]
async fn the_hop_limit_bounds_the_forwarding_of_announcements() {
    let dir = tempfile::tempdir().unwrap();
    let mut n: Vec<Node> = ["a", "b", "c", "d"]
        .iter()
        .map(|name| node(dir.path(), name))
        .collect();
    // B forwards only a path that has 2 nodes after its hop.
    n[1].mesh.set_hop_limit(2);
    for i in 0..3 {
        let (left, right) = n.split_at_mut(i + 1);
        join(&mut left[i], &mut right[0], 2).await;
    }
    has_route(&n[0], &n[2], 2).await;
    // D is 3 hops from A. It needs B to send on a path of 3 nodes.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(route(&n[0].mesh, &n[3]).is_none());
    // C and D have the routes that B does not cut.
    has_route(&n[2], &n[0], 2).await;
}

#[tokio::test]
async fn a_loop_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, mut b, mut c) = (
        node(dir.path(), "a"),
        node(dir.path(), "b"),
        node(dir.path(), "c"),
    );
    let (la, _lb) = join(&mut a, &mut b, 3).await;
    join(&mut b, &mut c, 5).await;
    has_route(&a, &c, 2).await;
    let ask = |visited: Vec<PublicKey>| {
        let la = la.clone();
        let dest = c.id.public();
        async move {
            let mut stream = la.open().unwrap();
            let header = Header {
                dest,
                hops: 8,
                service: "echo".into(),
                visited,
            };
            let line = format!("{}{}\n{}", "PNSVC1 ", forward::NAME, header.line());
            stream.write_all(line.as_bytes()).await.unwrap();
            let mut answer = [0u8; 1];
            stream.read_exact(&mut answer).await.unwrap();
            answer[0]
        }
    };
    // B is in the list of visited nodes: a loop.
    assert_eq!(
        ask(vec![a.id.public(), b.id.public()]).await,
        service::NO_ROUTE
    );
    // The next hop (C) is in the list: B has no other route.
    assert_eq!(
        ask(vec![a.id.public(), c.id.public()]).await,
        service::NO_ROUTE
    );
    // A bad header.
    let mut stream = la.open().unwrap();
    stream
        .write_all(format!("PNSVC1 {}\nnot a header\n", forward::NAME).as_bytes())
        .await
        .unwrap();
    let mut answer = [0u8; 1];
    stream.read_exact(&mut answer).await.unwrap();
    assert_eq!(answer[0], service::NO_ROUTE);
    // The clean list works.
    assert_eq!(ask(vec![a.id.public()]).await, service::OPEN);
}

#[tokio::test]
async fn a_closed_middle_link_fails_one_stream_not_the_process() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, mut b, mut c) = (
        node(dir.path(), "a"),
        node(dir.path(), "b"),
        node(dir.path(), "c"),
    );
    join(&mut a, &mut b, 3).await;
    let (lbc, _lcb) = join(&mut b, &mut c, 5).await;
    has_route(&a, &c, 2).await;
    let dest = c.id.public();
    let mut first = forward::open(&a.mesh, &dest, "echo", 8).await.unwrap();
    first.write_all(b"one").await.unwrap();
    let mut buf = [0u8; 3];
    first.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"one");
    // The middle link closes: the open stream ends.
    lbc.close().await;
    let mut rest = Vec::new();
    let ended = tokio::time::timeout(Duration::from_secs(5), first.read_to_end(&mut rest)).await;
    assert!(ended.is_ok(), "the stream did not end");
    // The link A-B and the nodes live: B still serves a stream of A.
    let stream = forward::open(&a.mesh, &b.id.public(), "echo", 1)
        .await
        .unwrap();
    assert_eq!(echo_roundtrip(stream, b"alive".to_vec()).await, b"alive");
    // A new stream to C fails with an answer, not a hang.
    let end = Instant::now() + Duration::from_secs(10);
    let failure = loop {
        match forward::open(&a.mesh, &dest, "echo", 8).await {
            Err(why) => break why,
            Ok(stream) => drop(stream),
        }
        assert!(Instant::now() < end, "the stream to C never failed");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert!(matches!(
        failure,
        ForwardError::NoRoute | ForwardError::ResourceLimit
    ));
}

/// A peer of A that answers each forwarded stream with `answer`, or, with
/// `None`, opens it and echoes. It counts the streams.
fn stub(link: Arc<Link>, answer: Option<u8>, count: Arc<AtomicUsize>) {
    tokio::spawn(async move {
        while let Ok(mut stream) = link.accept().await {
            let count = count.clone();
            tokio::spawn(async move {
                let Ok(name) = service::accept(&mut stream).await else {
                    return;
                };
                assert_eq!(name, forward::NAME);
                count.fetch_add(1, Ordering::SeqCst);
                if let Some(byte) = answer {
                    let _ = service::reply(&mut stream, byte).await;
                    return;
                }
                let mut byte = [0u8; 1];
                while byte[0] != b'\n' {
                    stream.read_exact(&mut byte).await.unwrap();
                }
                let _ = service::reply(&mut stream, service::OPEN).await;
                let (mut r, mut w) = tokio::io::split(stream);
                let _ = tokio::io::copy(&mut r, &mut w).await;
                let _ = w.shutdown().await;
            });
        }
    });
}

#[tokio::test]
async fn a_hop_without_a_stream_makes_the_sender_try_the_next_route() {
    let dir = tempfile::tempdir().unwrap();
    let a = node(dir.path(), "a");
    let (b, c, d) = (
        node(dir.path(), "b"),
        node(dir.path(), "c"),
        node(dir.path(), "d"),
    );
    // A has two next hops to D: B (cheap) and C. B has no stream to give.
    let now = announce::now_ms();
    let (la_b, lb) = link::unix_pair(Settings::default()).unwrap();
    let (la_c, lc) = link::unix_pair(Settings::default()).unwrap();
    let (asked_b, asked_c) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    stub(Arc::new(lb), Some(service::RESOURCE_LIMIT), asked_b.clone());
    stub(Arc::new(lc), None, asked_c.clone());
    let (tx, _rx) = tokio::sync::mpsc::channel(1);
    a.mesh
        .attach_link(b.id.public(), Arc::new(la_b), tx.clone());
    a.mesh.attach_link(c.id.public(), Arc::new(la_c), tx);
    let via_c = wire::append_hop(&d.mesh.make(now).unwrap(), c.id.key(), 1).unwrap();
    let via_b = wire::append_hop(&d.mesh.make(now).unwrap(), b.id.key(), 1).unwrap();
    a.mesh.receive(&c.id.public(), 10, &via_c, now).unwrap();
    a.mesh.receive(&b.id.public(), 1, &via_b, now).unwrap();
    let stream = forward::open(&a.mesh, &d.id.public(), "echo", 8)
        .await
        .unwrap();
    assert_eq!(echo_roundtrip(stream, b"retry".to_vec()).await, b"retry");
    assert_eq!(asked_b.load(Ordering::SeqCst), 1, "B was tried first");
    assert_eq!(asked_c.load(Ordering::SeqCst), 1, "C was tried second");
}

#[test]
fn a_multi_hop_announcement_needs_a_valid_chain() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b, c) = (
        node(dir.path(), "a"),
        node(dir.path(), "b"),
        node(dir.path(), "c"),
    );
    let now = announce::now_ms();
    // C announces; B forwards with its hop (cost 5); A takes it from B.
    let from_c = c.mesh.make(now).unwrap();
    let via_b = wire::append_hop(&from_c, b.id.key(), 5).unwrap();
    assert!(a.mesh.receive(&b.id.public(), 3, &via_b, now).is_ok());
    let row = route(&a.mesh, &c).unwrap();
    assert_eq!(row["cost"], 8);
    assert_eq!(row["hops"], 2);
    // The last node of the path must be the sender.
    let from_c = c.mesh.make(now).unwrap();
    let via_b = wire::append_hop(&from_c, b.id.key(), 5).unwrap();
    assert_eq!(
        a.mesh.receive(&c.id.public(), 3, &via_b, now).unwrap_err(),
        Dropped::NotDirect
    );
    // A lowered cost of the hop fails the chain.
    let mut low = via_b.clone();
    let at = low.len() - 68;
    low[at..at + 4].copy_from_slice(&1u32.to_be_bytes());
    assert_eq!(
        a.mesh.receive(&b.id.public(), 3, &low, now).unwrap_err(),
        Dropped::Announcement(Rule::BadHopSignature)
    );
}
