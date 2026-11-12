//! The tests of the TCP path, the screen and the wires of a guest (T-248).

use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::sync::Notify;

use super::kit::*;
use super::pump::{screen, Verdict};
use super::wire::{self, Opened, WireSpec};
use super::*;

const DST: &str = "93.184.216.34";

fn text(wire: &str, net: &str, via: &str, extra: &str) -> String {
    format!(
        "networks:\n  a: direct\nguests:\n  - wire: {wire}\n    net: {net}\n    via: {via}\n{extra}"
    )
}

#[test]
fn guest_wire_config_names_the_key_that_is_wrong() {
    let ok = text("unix:/x", "10.66.0.0/24", "a", "");
    let config = crate::router::parse(&ok).unwrap();
    assert!(config.doors.is_empty(), "a guest alone is a router");
    assert_eq!(config.guests.len(), 1);
    let cases = [
        (text("tcp:/x", "10.66.0.0/24", "a", ""), "guests[0].wire"),
        (text("tun:", "10.66.0.0/24", "a", ""), "guests[0].wire"),
        (
            text("tun:a-name-of-sixteen", "10.66.0.0/24", "a", ""),
            "guests[0].wire",
        ),
        (text("unix:/x", "10.66.0.0/31", "a", ""), "guests[0].net"),
        (text("unix:/x", "10.66.0.0", "a", ""), "guests[0].net"),
        (
            text("unix:/x", "10.66.0.0/24", "nowhere", ""),
            "guests[0].via",
        ),
        (
            text("unix:/x", "10.66.0.0/24", "a", "    mtu: 100\n"),
            "guests[0].mtu",
        ),
        (
            text("unix:/x", "10.66.0.0/24", "a", "    port: 1\n"),
            "port",
        ),
        (
            format!("{ok}  - wire: unix:/x\n    net: 10.67.0.0/24\n    via: a\n"),
            "guests[1].wire",
        ),
        ("networks:\n  a: direct\n".to_string(), "doors"),
    ];
    for (file, key) in cases {
        let err = format!("{:#}", crate::router::parse(&file).expect_err(key));
        assert!(err.contains(key), "{key}: {err}");
    }
}

#[test]
fn guest_wire_net_and_wire_parse() {
    let net: GuestNet = "10.66.4.77/24".parse().unwrap();
    assert_eq!(net.network.to_string(), "10.66.4.0");
    assert_eq!(net.gateway().to_string(), "10.66.4.1");
    assert!(net.contains("10.66.4.200".parse().unwrap()));
    assert!(!net.contains("10.66.5.1".parse().unwrap()));
    assert_eq!(
        "unix:/run/g.sock".parse::<WireSpec>().unwrap().to_string(),
        "unix:/run/g.sock"
    );
    assert_eq!(
        "tun:g0".parse::<WireSpec>().unwrap(),
        WireSpec::Tun("g0".into())
    );
}

/// The screen: only IPv4 TCP and UDP from the net go on, to a unicast
/// destination outside the net.
#[tokio::test]
async fn guest_wire_screen_drops_what_the_router_does_not_route() {
    let (_dir, text) = rules_file();
    let rig = Rig::new(&text, Fake::new("A"), Fake::new("B")).await;
    let guest = &rig.router.guests[0];
    let packet = |proto: u8, src: [u8; 4], dst: [u8; 4], flags: u8, frag: u16| {
        let mut p = vec![0u8; 40];
        p[0] = 0x45;
        p[2..4].copy_from_slice(&40u16.to_be_bytes());
        p[6..8].copy_from_slice(&frag.to_be_bytes());
        p[9] = proto;
        p[12..16].copy_from_slice(&src);
        p[16..20].copy_from_slice(&dst);
        p[20..22].copy_from_slice(&40000u16.to_be_bytes());
        p[22..24].copy_from_slice(&443u16.to_be_bytes());
        p[33] = flags;
        p
    };
    let me = [10, 66, 0, 4];
    let far = [93, 184, 216, 34];
    let syn = packet(6, me, far, 0x02, 0);
    assert_eq!(
        screen(guest, &syn),
        Verdict::Syn((
            "10.66.0.4".parse().unwrap(),
            40000,
            "93.184.216.34".parse().unwrap(),
            443
        ))
    );
    assert_eq!(screen(guest, &packet(6, me, far, 0x10, 0)), Verdict::Pass);
    assert_eq!(screen(guest, &packet(17, me, far, 0, 0)), Verdict::Udp(443));
    assert_eq!(screen(guest, &packet(17, me, far, 0, 185)), Verdict::Pass);
    assert_eq!(screen(guest, &packet(1, me, far, 0, 0)), Verdict::Drop);
    assert_eq!(
        screen(guest, &packet(6, [10, 67, 0, 4], far, 0x02, 0)),
        Verdict::Drop
    );
    for dst in [
        [10, 66, 0, 1],
        [10, 66, 0, 9],
        [127, 0, 0, 1],
        [224, 0, 0, 1],
        [255, 255, 255, 255],
        [0, 0, 0, 0],
    ] {
        assert_eq!(
            screen(guest, &packet(6, me, dst, 0x02, 0)),
            Verdict::Drop,
            "{dst:?}"
        );
    }
    let mut v6 = syn.clone();
    v6[0] = 0x60;
    assert_eq!(screen(guest, &v6), Verdict::Drop);
    assert_eq!(screen(guest, &syn[..30]), Verdict::Drop);
}

/// The Prove of T-248: the rules and the groups of the router choose the
/// way of each TCP flow of the guest, and a rule that rejects refuses it.
#[tokio::test]
async fn guest_wire_routes_tcp_by_the_rules() {
    let (_dir, text) = rules_file();
    let (a, b) = (Fake::new("A"), Fake::new("B"));
    let rig = Rig::new(&text, a.clone(), b.clone()).await;
    // A rule sends 93.184.216.0/24 to the network a.
    let mut c = rig.guest.connect(at(DST, 443)).await.unwrap();
    within(5, "handshake", c.established()).await.unwrap();
    c.write_all(b"ping").await.unwrap();
    let mut got = [0u8; 6];
    within(5, "echo", c.read_exact(&mut got)).await.unwrap();
    assert_eq!(&got, b"A|ping");
    // No rule matches 1.1.1.1: the group g, whose member is b.
    let mut c2 = rig.guest.connect(at("1.1.1.1", 8443)).await.unwrap();
    within(5, "handshake", c2.established()).await.unwrap();
    c2.write_all(b"pong").await.unwrap();
    within(5, "echo", c2.read_exact(&mut got)).await.unwrap();
    assert_eq!(&got, b"B|pong");
    assert_eq!(a.dialed(), [format!("{DST}:443")]);
    assert_eq!(b.dialed(), ["1.1.1.1:8443".to_string()]);
    assert_eq!(rig.router.status()["groups"][0]["members"][0]["flows"], 1);
    // The dry run names the same ways.
    let to = |s: &str| s.parse::<Target>().unwrap();
    assert_eq!(
        rig.router
            .decide_guest(0, &to(&format!("{DST}:443")))
            .way
            .as_deref(),
        Some("a")
    );
    assert_eq!(
        rig.router
            .decide_guest(0, &to("1.1.1.1:443"))
            .way
            .as_deref(),
        Some("g")
    );
    assert_eq!(
        rig.router.decide_guest(0, &to("198.51.100.7:443")).way,
        None
    );
    // A rule that rejects: the connect of the guest is refused and nothing is dialed.
    let mut refused = rig.guest.connect(at("198.51.100.7", 443)).await.unwrap();
    let err = within(5, "refusal", refused.established())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::ConnectionRefused);
    assert_eq!(rig.stats().tcp_refused.load(Ordering::Relaxed), 1);
    assert_eq!(a.dialed().len() + b.dialed().len(), 2);
    assert_eq!(rig.stats().tcp_dialed.load(Ordering::Relaxed), 3);
    assert_eq!(rig.stats().tcp_open.load(Ordering::Relaxed), 2);
    let metrics = rig.router.metrics();
    assert!(
        metrics.contains("podnet_route_guest_tcp_dialed_total{guest=\"unix:/unused\"} 3"),
        "{metrics}"
    );
    assert!(
        metrics.contains("podnet_route_guest_tcp_refused_total{guest=\"unix:/unused\"} 1"),
        "{metrics}"
    );
}

/// The dial comes first (T-176, constraint 2): the guest has no handshake
/// while the dial runs, the copies of its SYN make one dial, and the
/// handshake ends after the dial.
#[tokio::test]
async fn guest_wire_dials_before_the_handshake() {
    let (_dir, text) = rules_file();
    let gate = Arc::new(Notify::new());
    let a = Fake::with("A", false, Some(gate.clone()));
    let rig = Rig::new(&text, a.clone(), Fake::new("B")).await;
    let mut c = rig.guest.connect(at(DST, 443)).await.unwrap();
    // The guest sends its SYN again at 1 s: wait for the copy.
    let early = tokio::time::timeout(Duration::from_millis(1400), c.established()).await;
    assert!(early.is_err(), "the handshake ended before the dial");
    assert_eq!(a.dialed().len(), 1, "a copy of the SYN made a second dial");
    assert_eq!(
        rig.router.status()["guests"][0]["counters"]["tcp_dialed"],
        1
    );
    gate.notify_one();
    within(3, "handshake after the dial", c.established())
        .await
        .unwrap();
    c.write_all(b"x").await.unwrap();
    let mut got = [0u8; 3];
    within(5, "echo", c.read_exact(&mut got)).await.unwrap();
    assert_eq!(&got, b"A|x");
    assert_eq!(a.dialed().len(), 1);
}

/// A dial that fails answers the SYN with a RST at once: `connect` of the
/// guest is refused (T-176: not proven before).
#[tokio::test]
async fn guest_wire_a_failed_dial_refuses_the_connect() {
    let (_dir, text) = rules_file();
    let a = Fake::with("A", true, None);
    let rig = Rig::new(&text, a.clone(), Fake::new("B")).await;
    let started = std::time::Instant::now();
    let mut c = rig.guest.connect(at(DST, 443)).await.unwrap();
    let err = within(5, "refusal", c.established()).await.unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::ConnectionRefused);
    assert!(
        started.elapsed() < Duration::from_millis(900),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(rig.stats().tcp_refused.load(Ordering::Relaxed), 1);
    assert_eq!(rig.stats().tcp_open.load(Ordering::Relaxed), 0);
}

/// A flow carries a large payload both ways and ends in order: the FIN of the
/// guest reaches the upstream, and the end of the upstream reaches the guest.
#[tokio::test]
async fn guest_wire_a_flow_carries_bytes_and_ends_both_ways() {
    let (_dir, text) = rules_file();
    let rig = Rig::new(&text, Fake::new("A"), Fake::new("B")).await;
    let mut c = rig.guest.connect(at(DST, 443)).await.unwrap();
    within(5, "handshake", c.established()).await.unwrap();
    let data: Vec<u8> = (0..1_000_000u32)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8)
        .collect();
    let (mut read, mut write) = tokio::io::split(c);
    let copy = data.clone();
    let writer = tokio::spawn(async move {
        write.write_all(&copy).await.unwrap();
        write.shutdown().await.unwrap();
    });
    let mut got = Vec::new();
    within(30, "the echo", read.read_to_end(&mut got))
        .await
        .unwrap();
    writer.await.unwrap();
    assert_eq!(&got[..2], b"A|");
    assert!(
        got[2..] == data[..],
        "the echo changed ({} bytes)",
        got.len()
    );
    // The flow is over: the counter of open flows goes back to 0.
    for _ in 0..50 {
        if rig.stats().tcp_open.load(Ordering::Relaxed) == 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the flow did not end");
}

/// The guest wire as a unix socket: the router listens, a guest connects,
/// and a flow is routed. A new connection replaces the old one.
#[tokio::test]
async fn guest_wire_unix_socket_serves_a_guest_and_the_next() {
    let (dir, text) = rules_file();
    let wire_path = dir.path().join("g.sock");
    let text = text.replace("unix:/unused", &format!("unix:{}", wire_path.display()));
    let config = crate::router::parse(&text).unwrap();
    let mut ways = std::collections::HashMap::new();
    let (a, b) = (Fake::new("A"), Fake::new("B"));
    ways.insert("a".to_string(), a.way());
    ways.insert("b".to_string(), b.way());
    let router = crate::router::Router::build(&config, ways).unwrap();
    let opened = router.guests[0].open(false).unwrap();
    assert!(matches!(opened, Opened::Listener(_)));
    let mode = std::os::unix::fs::PermissionsExt::mode(
        &std::fs::metadata(&wire_path).unwrap().permissions(),
    );
    assert_eq!(mode & 0o777, 0o600);
    let served = tokio::spawn(router.guests[0].clone().serve(opened));
    for round in 0..2 {
        let guest = pod_netstack::NetStack::over_link(guest_config());
        let fd = wire::connect_seqpacket(&wire_path).unwrap();
        let _tasks = join(&guest, fd);
        let mut c = guest.connect(at(DST, 443)).await.unwrap();
        within(5, &format!("handshake of round {round}"), c.established())
            .await
            .unwrap();
        c.write_all(b"hi").await.unwrap();
        let mut got = [0u8; 4];
        within(5, &format!("echo of round {round}"), c.read_exact(&mut got))
            .await
            .unwrap();
        assert_eq!(&got, b"A|hi");
    }
    served.abort();
}

/// A TUN device needs `CAP_NET_ADMIN`. The test skips when the host refuses
/// it; the data path of a TUN is that of the socket pair (`PacketFd`).
#[test]
fn guest_wire_tun_opens_or_the_host_refuses_it() {
    match wire::open_tun("podnet-gtest0") {
        Ok(fd) => {
            let mut byte = [0u8; 1];
            // SAFETY: a read of one byte from a descriptor that this test owns.
            let n = unsafe {
                libc::read(
                    std::os::fd::AsRawFd::as_raw_fd(&fd),
                    byte.as_mut_ptr().cast(),
                    1,
                )
            };
            assert!(n < 0, "an idle TUN has no packet");
        }
        Err(e) => eprintln!("skipped: the host refuses a TUN device ({e})"),
    }
}
