//! The tests of the UDP path of a guest wire (T-248).

use std::sync::atomic::Ordering;
use std::time::Duration;

use super::kit::*;
use super::udp::QUEUE;

const DST: &str = "93.184.216.34";

async fn rig() -> (tempfile::TempDir, Rig) {
    let (dir, text) = rules_file();
    let rig = Rig::new(&text, Fake::new("A"), Fake::new("B")).await;
    (dir, rig)
}

async fn count(what: &str, counter: &std::sync::atomic::AtomicU64, want: u64) {
    for _ in 0..100 {
        if counter.load(Ordering::Relaxed) >= want {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("{what}: {} of {want}", counter.load(Ordering::Relaxed));
}

/// Each UDP flow goes over the way that the rules choose for its
/// destination, and the answer comes back from that destination.
#[tokio::test]
async fn guest_wire_udp_routes_by_the_rules() {
    let (_dir, rig) = rig().await;
    let mut socket = rig.guest.bind_udp(at(GUEST, 40000)).await.unwrap();
    let mut buf = vec![0u8; 2048];
    for (dst, via) in [(at(DST, 5353), &rig.a), (at("1.1.1.1", 5353), &rig.b)] {
        socket.send_to(b"query", dst).await.unwrap();
        let (n, from) = within(5, "answer", socket.recv_from(&mut buf))
            .await
            .unwrap();
        assert_eq!((&buf[..n], from), (&b"query"[..], dst));
        assert_eq!(via.udp.sent(), [(dst, b"query".to_vec())]);
    }
    // A rule that rejects: no answer, and the datagram is counted.
    socket
        .send_to(b"query", at("198.51.100.7", 5353))
        .await
        .unwrap();
    count("refused", &rig.stats().udp_refused, 1).await;
    let none = tokio::time::timeout(Duration::from_millis(300), socket.recv_from(&mut buf)).await;
    assert!(none.is_err(), "a rejected target got an answer");
    let stats = rig.stats();
    assert_eq!(stats.udp_flows.load(Ordering::Relaxed), 3);
    assert_eq!(stats.udp_up.load(Ordering::Relaxed), 2);
    assert_eq!(stats.udp_down.load(Ordering::Relaxed), 2);
    // The flow of the guest and a target keeps its way and its socket.
    socket.send_to(b"again", at(DST, 5353)).await.unwrap();
    within(5, "answer", socket.recv_from(&mut buf))
        .await
        .unwrap();
    assert_eq!(stats.udp_flows.load(Ordering::Relaxed), 3);
}

/// The Prove of T-248: a flow whose way is slow has a queue of `QUEUE`
/// datagrams. The router reads the socket of the stack at once, drops the
/// datagrams past the queue, counts them, and keeps the order of the rest.
#[tokio::test]
async fn guest_wire_udp_has_a_bounded_queue() {
    let (_dir, rig) = rig().await;
    rig.a.udp.block(true);
    let sender = rig.guest.bind_udp(at(GUEST, 40000)).await.unwrap();
    const SENT: u32 = 500;
    for i in 0..SENT {
        sender
            .send_to(&i.to_be_bytes(), at(DST, 5353))
            .await
            .unwrap();
    }
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let stats = rig.stats();
    assert!(rig.a.udp.sent().is_empty(), "a blocked way took a datagram");
    rig.a.udp.block(false);
    tokio::time::sleep(Duration::from_millis(800)).await;
    let got: Vec<u32> = rig
        .a
        .udp
        .sent()
        .iter()
        .map(|(_, d)| u32::from_be_bytes(d[..4].try_into().unwrap()))
        .collect();
    let dropped = stats.udp_queue_dropped.load(Ordering::Relaxed);
    // The queue, and the datagram in the hand of the flow.
    assert!(
        got.len() <= QUEUE + 1,
        "the queue did not bound: {}",
        got.len()
    );
    assert!(
        got.len() >= QUEUE,
        "the queue lost datagrams it could keep: {}",
        got.len()
    );
    assert!(dropped > 0, "no datagram counted");
    assert!(got.windows(2).all(|w| w[0] < w[1]), "the order changed");
    // Each datagram is in the way, in a counter of the router, or in a counter
    // of the stack (`stack_udp_rx_dropped`, PodUtils/podnet#464, ask 3).
    let in_stack = stats.stack_udp_rx_dropped.load(Ordering::Relaxed);
    assert_eq!(
        u64::from(SENT),
        got.len() as u64 + dropped + in_stack,
        "way {} + router {dropped} + stack {in_stack}",
        got.len()
    );
}

/// IPv4 fragmentation (the features of `smoltcp` in `Cargo.toml`) carries a
/// datagram over the MTU, up and down, to 65507 bytes. A datagram to the
/// guest over that size is dropped and counted.
#[tokio::test]
async fn guest_wire_udp_carries_a_datagram_over_the_mtu() {
    let (_dir, rig) = rig().await;
    let mut socket = rig.guest.bind_udp(at(GUEST, 40000)).await.unwrap();
    let dst = at(DST, 5353);
    let mut buf = vec![0u8; 70000];
    // 1472 is the largest payload without fragmentation at the MTU 1500.
    for size in [1472usize, 1473, 4000, 65507] {
        let payload: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
        socket.send_to(&payload, dst).await.unwrap();
        let (n, from) = within(10, &format!("echo of {size}"), socket.recv_from(&mut buf))
            .await
            .unwrap();
        assert_eq!(from, dst);
        assert!(
            buf[..n] == payload[..],
            "{size} bytes changed ({n} came back)"
        );
    }
    assert_eq!(rig.stats().udp_oversize.load(Ordering::Relaxed), 0);
    rig.a.udp.arrive(dst, vec![7u8; 65507 + 13]);
    count("oversize", &rig.stats().udp_oversize, 1).await;
    let none = tokio::time::timeout(Duration::from_millis(500), socket.recv_from(&mut buf)).await;
    assert!(none.is_err(), "an oversized datagram reached the guest");
}
