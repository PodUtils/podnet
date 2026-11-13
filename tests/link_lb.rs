//! Load balancing across link groups (T-288, GitHub #470 row E4-4): a link
//! is a member of a group of the router engine. The tests use memory
//! carriers and one control socket, so they bind no TCP port.
//!
//! The process variable `PODNET_LINK_CONTROL` names the one control socket
//! that the members ask. One test runs at a time (`SERIAL`).

use std::sync::Arc;
use std::time::Duration;

use pod_identity::PublicKey;
use podnet::hello;
use podnet::identity::{self, AllowList, Identity};
use podnet::link::{self, Link, Settings};
use podnet::link_ctl::{Control, Hub, Slot, CONTROL_VARIABLE};
use podnet::link_group::{self, LinkGroup};
use podnet::link_service as service;
use podnet::network::{Class, Dial as _, Failure, Health, Member, Policy, Selection};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::{timeout, Instant};

const LIMIT: Duration = Duration::from_secs(20);
static SERIAL: std::sync::LazyLock<tokio::sync::Mutex<()>> =
    std::sync::LazyLock::new(|| tokio::sync::Mutex::new(()));

fn install(dir: &std::path::Path, name: &str) -> Identity {
    let sub = dir.join(name);
    std::fs::create_dir(&sub).unwrap();
    identity::create(&sub).unwrap()
}

fn list(key: &Identity) -> AllowList {
    AllowList::parse(&key.fingerprint()).unwrap()
}

/// The far side of a link: the service `echo` writes the tag of the peer
/// (one byte) and echoes. The service `bad` is not served.
async fn far(link: Arc<Link>, tag: u8) {
    while let Ok(mut stream) = link.accept().await {
        tokio::spawn(async move {
            let Ok(name) = service::accept(&mut stream).await else {
                return;
            };
            if name == "echo" {
                service::reply(&mut stream, service::OPEN).await.unwrap();
                stream.write_all(&[tag]).await.unwrap();
                let (mut r, mut w) = tokio::io::split(stream);
                let _ = tokio::io::copy(&mut r, &mut w).await;
            } else {
                let _ = service::reply(&mut stream, service::NO_SERVICE).await;
            }
        });
    }
}

struct Peer {
    fingerprint: String,
    public: PublicKey,
    slot: Option<Slot>,
    near: Arc<Link>,
}

struct World {
    _dir: tempfile::TempDir,
    _control: Control,
    hub: Arc<Hub>,
    peers: Vec<Peer>,
}

/// A near instance that holds one link to each of `n` far peers. Peer `i`
/// has the tag `i`.
async fn world(n: u8) -> World {
    let dir = tempfile::tempdir().unwrap();
    let near = install(dir.path(), "near");
    let hub = Hub::new(&near.fingerprint());
    let mut peers = Vec::new();
    for i in 0..n {
        let key = install(dir.path(), &format!("far{i}"));
        let (dialer, listener) = link::memory_pair(1 << 20, Settings::default()).unwrap();
        let (d, l): (_, anyhow::Result<PublicKey>) = tokio::join!(
            hello::dial(&dialer, &near, list(&key)),
            hello::accept(&listener, &key, list(&near))
        );
        let public = d.unwrap();
        l.unwrap();
        tokio::spawn(far(Arc::new(listener), i));
        let dialer = Arc::new(dialer);
        let slot = hub.hold(dialer.clone(), public, "memory", "pair", "dialer");
        peers.push(Peer {
            fingerprint: key.fingerprint(),
            public,
            slot: Some(slot),
            near: dialer,
        });
    }
    let socket = dir.path().join("ctl.sock");
    let control = Control::start(&socket, hub.clone()).unwrap();
    std::env::set_var(CONTROL_VARIABLE, &socket);
    World {
        _dir: dir,
        _control: control,
        hub,
        peers,
    }
}

impl World {
    /// Hold the link to peer `i` again.
    fn restore(&mut self, i: usize) {
        let p = &mut self.peers[i];
        p.slot = Some(
            self.hub
                .hold(p.near.clone(), p.public, "memory", "pair", "dialer"),
        );
    }

    fn spec(&self, i: usize, service: &str) -> String {
        format!("peer:{}/{service}", self.peers[i].fingerprint)
    }
}

async fn tag(group: &impl podnet::network::Dial) -> Result<u8, Failure> {
    let mut conn = group
        .dial(&podnet::network::peer("lb"), Instant::now() + LIMIT)
        .await?;
    let mut one = [0u8; 1];
    timeout(LIMIT, conn.io.read_exact(&mut one))
        .await
        .unwrap()
        .unwrap();
    Ok(one[0])
}

fn members(w: &World, weights: &[u32]) -> Vec<(String, u32)> {
    weights
        .iter()
        .enumerate()
        .map(|(i, wt)| (w.spec(i, "echo"), *wt))
        .collect()
}

fn group(w: &World, weights: &[u32], policy: Policy) -> LinkGroup {
    LinkGroup::new(
        "links",
        &members(w, weights),
        policy,
        link_group::thresholds(1),
    )
    .unwrap()
}

#[tokio::test]
async fn link_lb_weighted_pick_pins_each_stream_to_one_link() {
    let _g = SERIAL.lock().await;
    let w = world(2).await;
    let lg = group(&w, &[3, 1], Policy::Weighted);
    let mut counts = [0u32; 2];
    for _ in 0..200 {
        counts[tag(lg.group().as_ref()).await.unwrap() as usize] += 1;
    }
    // 3:1 within a margin of 200 streams.
    assert!((130..=170).contains(&counts[0]), "{counts:?}");
    assert_eq!(counts[0] + counts[1], 200);
    let flows: Vec<u64> = lg.group().members().iter().map(Member::flows).collect();
    assert_eq!(flows, vec![counts[0] as u64, counts[1] as u64]);
    // Each stream took its link at the open: the link counted the open.
    assert_eq!(
        w.hub.status()["links"][0]["opened"].as_u64().unwrap()
            + w.hub.status()["links"][1]["opened"].as_u64().unwrap(),
        200
    );
    // The group is a peer way.
    assert_eq!(
        lg.group().way().addressing,
        podnet::network::Addressing::Peer
    );
    // A stream stays on its member for its whole life: the echo of a
    // stream comes from the peer that gave its tag.
    let mut conn = lg
        .group()
        .dial(&podnet::network::peer("lb"), Instant::now() + LIMIT)
        .await
        .unwrap();
    let mut one = [0u8; 1];
    conn.io.read_exact(&mut one).await.unwrap();
    for _ in 0..20 {
        conn.io.write_all(&[one[0]]).await.unwrap();
        let mut back = [0u8; 1];
        conn.io.read_exact(&mut back).await.unwrap();
        assert_eq!(back[0], one[0]);
    }
}

#[tokio::test]
async fn link_lb_rr_and_leastload_policies_spread_streams() {
    let _g = SERIAL.lock().await;
    let w = world(3).await;
    let lg = group(&w, &[1, 1, 1], Policy::Rr);
    let mut seen = Vec::new();
    for _ in 0..6 {
        seen.push(tag(lg.group().as_ref()).await.unwrap());
    }
    let mut each = [0; 3];
    for t in &seen {
        each[*t as usize] += 1;
    }
    assert_eq!(each, [2, 2, 2], "{seen:?}");
    let lg = group(&w, &[1, 1, 1], Policy::LeastLoad);
    // Hold two streams open: the next stream takes the third link.
    let mut held = Vec::new();
    let mut tags = Vec::new();
    for _ in 0..3 {
        let mut conn = lg
            .group()
            .dial(&podnet::network::peer("lb"), Instant::now() + LIMIT)
            .await
            .unwrap();
        let mut one = [0u8; 1];
        timeout(LIMIT, conn.io.read_exact(&mut one))
            .await
            .unwrap()
            .unwrap();
        tags.push(one[0]);
        held.push(conn);
    }
    tags.sort();
    assert_eq!(tags, [0, 1, 2], "each open stream is on its own link");
}

#[tokio::test]
async fn link_lb_control_probe_drops_a_member_at_once_and_it_comes_back() {
    let _g = SERIAL.lock().await;
    let mut w = world(2).await;
    let lg = group(&w, &[1, 1], Policy::Weighted);
    lg.probe_round().await;
    assert!(lg
        .group()
        .members()
        .iter()
        .all(|m| m.health() == Health::Up));
    // The link to peer 1 ends: one control probe drops the member.
    w.peers[1].slot = None;
    lg.probe_round().await;
    assert_eq!(lg.group().members()[1].health(), Health::Down);
    assert_eq!(lg.group().members()[0].health(), Health::Up);
    for _ in 0..30 {
        assert_eq!(tag(lg.group().as_ref()).await.unwrap(), 0);
    }
    // The link comes back: the data probe gives probation, then up.
    w.restore(1);
    lg.probe_round().await;
    assert_eq!(lg.group().members()[1].health(), Health::Probation);
    for _ in 0..2 {
        lg.probe_round().await;
    }
    assert_eq!(lg.group().members()[1].health(), Health::Up);
}

#[tokio::test]
async fn link_lb_data_probe_drops_a_member_after_k_misses_and_no_member_refuses() {
    let _g = SERIAL.lock().await;
    let w = world(2).await;
    // Member 1 serves no `bad`: its control probe passes, its data probe
    // fails each time.
    let specs = vec![(w.spec(0, "echo"), 1), (w.spec(1, "bad"), 1)];
    let lg = LinkGroup::new("links", &specs, Policy::Weighted, link_group::thresholds(1)).unwrap();
    let health = |i: usize| lg.group().members()[i].health();
    lg.probe_round().await;
    assert_eq!(health(1), Health::Cooldown, "a data miss is not down yet");
    for _ in 0..2 {
        lg.probe_round().await;
        assert_eq!(health(1), Health::Cooldown);
    }
    lg.probe_round().await;
    assert_eq!(health(1), Health::Down, "K = 4 misses in a row");
    assert_eq!(health(0), Health::Up);
    // Each member down: the stream is refused, never sent direct.
    lg.group().set_health(0, Health::Down);
    assert!(matches!(lg.group().pick(), Selection::None));
    let err = tag(lg.group().as_ref()).await.err().unwrap();
    assert_eq!(err.class, Class::Policy);
    assert!(err.text().contains("no healthy member"), "{}", err.text());
}

#[tokio::test]
async fn link_lb_limits_are_16_members_weights_1_to_100_interval_1_to_60() {
    let _g = SERIAL.lock().await;
    let w = world(1).await;
    let fp = &w.peers[0].fingerprint;
    let make = |n: usize, weight: u32, secs: u64| {
        let specs: Vec<(String, u32)> = (0..n)
            .map(|i| (format!("peer:{fp}/s{i}"), weight))
            .collect();
        LinkGroup::new("g", &specs, Policy::Weighted, link_group::thresholds(secs))
    };
    assert!(make(16, 100, 1).is_ok());
    assert!(make(16, 1, 60).is_ok());
    for (n, weight, secs) in [
        (17, 1, 5),
        (0, 1, 5),
        (2, 101, 5),
        (2, 0, 5),
        (2, 1, 0),
        (2, 1, 61),
    ] {
        let err = make(n, weight, secs).err().expect("refused");
        assert_eq!(err.class, Class::Usage, "{n} {weight} {secs}");
    }
    // A member that is no peer is refused.
    let err = LinkGroup::new(
        "g",
        &[("direct".to_string(), 1)],
        Policy::Weighted,
        link_group::thresholds(5),
    )
    .err()
    .unwrap();
    assert_eq!(err.class, Class::Usage);
}

#[tokio::test]
async fn link_lb_the_router_file_takes_a_group_of_links() {
    let _g = SERIAL.lock().await;
    let w = world(2).await;
    let text = format!(
        "networks:\n  a: {}\n  b: {}\ngroups:\n  links: a*3, b*1\ndoors:\n  - listen: unix:{}/door.sock\n    via: links\n",
        w.spec(0, "echo"),
        w.spec(1, "echo"),
        w._dir.path().display()
    );
    let config = podnet::router::parse(&text).unwrap();
    let router = podnet::router::Router::up(&config).await.unwrap();
    let (_, group) = &router.groups[0];
    let mut counts = [0u32; 2];
    for _ in 0..100 {
        counts[tag(group.as_ref()).await.unwrap() as usize] += 1;
    }
    assert!(counts[0] > counts[1] && counts[1] > 0, "{counts:?}");
    // A weight over 100 is refused for a group of links.
    let bad = text.replace("a*3", "a*101");
    let config = podnet::router::parse(&bad).unwrap();
    let err = podnet::router::Router::up(&config).await.err().unwrap();
    assert!(format!("{err:#}").contains("groups.links"), "{err:#}");
}
