//! T-176: the constraints of a guest-wire router on `pod-netstack`.
//!
//! The router is a stack with `any_ip` and `listen_any` / `bind_udp_any`. The
//! guest is a second stack. Each test measures one constraint of
//! `docs/research/podnet/guest-wire-router.md` and pins the behavior of the
//! version of `pod-netstack` in `Cargo.lock`. A test that names a gap of
//! `pod-netstack` fails when the crate closes the gap: then change the test
//! and the note.

mod wire;

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use pod_netstack::{NetStack, PacketQueue, StackConfig};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use wire::*;

const DST: &str = "93.184.216.34";

fn pair(router: StackConfig, guest: StackConfig) -> (NetStack, NetStack, Wire) {
    let router = NetStack::over_link(router);
    let guest = NetStack::over_link(guest);
    let wire = Wire::join(&router, &guest);
    (router, guest, wire)
}

async fn read_all(stream: &mut pod_netstack::TcpStream) -> usize {
    let mut total = 0;
    let mut buf = vec![0u8; 65536];
    loop {
        match tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buf)).await {
            Ok(Ok(0)) => return total,
            Ok(Ok(n)) => total += n,
            other => panic!("read: {other:?} after {total} bytes"),
        }
    }
}

/// Constraint 1, case A: a flow that closed on both sides leaves no tuple.
/// The stack reads `TimeWait` as closed, so a new SYN for the tuple at once
/// has its handshake with no wait (no 10 s `TimeWait` delay).
#[tokio::test]
async fn guest_wire_closed_tuple_is_free_at_once() {
    let (router, guest, _wire) = pair(router_config(), guest_config());
    let mut listener = router.listen_any(443).await.unwrap();
    let dst = at(DST, 443);
    for round in 0..3 {
        let started = Instant::now();
        let mut c = guest.connect_from(dst, 40000).await.unwrap();
        within(5, "handshake", c.established()).await.unwrap();
        let mut s = within(5, "accept", listener.accept()).await.unwrap();
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "round {round}: {:?}",
            started.elapsed()
        );
        c.write_all(b"x").await.unwrap();
        let mut byte = [0u8; 1];
        s.read_exact(&mut byte).await.unwrap();
        c.shutdown();
        assert_eq!(read_all(&mut s).await, 0);
        s.shutdown();
        assert_eq!(read_all(&mut c).await, 0);
    }
}

/// Constraint 1, case B: the guest forgot a flow (a reboot) and sends a SYN
/// for the tuple while the router still holds the flow. The old socket
/// answers the SYN with an ACK, the guest answers it with a RST, and the SYN
/// that the guest sends again (1 s after the first) opens the new flow. The
/// router app of the old flow reads an end of file, not an error.
#[tokio::test]
async fn guest_wire_stale_tuple_is_released_by_a_new_syn() {
    let router = NetStack::over_link(router_config());
    let mut guest = NetStack::over_link(guest_config());
    let wire = Wire::join(&router, &guest);
    let mut listener = router.listen_any(443).await.unwrap();
    let dst = at(DST, 443);
    let mut old = guest.connect_from(dst, 40000).await.unwrap();
    within(5, "handshake", old.established()).await.unwrap();
    let mut stale = within(5, "accept", listener.accept()).await.unwrap();
    old.write_all(b"x").await.unwrap();
    let mut byte = [0u8; 1];
    stale.read_exact(&mut byte).await.unwrap();
    // The guest goes away with no FIN and no RST; a new guest has the same
    // address and the same source port.
    drop(wire);
    guest.shutdown().await;
    drop(old);
    let guest = NetStack::over_link(guest_config());
    let _wire = Wire::join(&router, &guest);
    let started = Instant::now();
    let mut new = guest.connect_from(dst, 40000).await.unwrap();
    within(10, "handshake of the new flow", new.established())
        .await
        .unwrap();
    let took = started.elapsed();
    assert!(
        took >= Duration::from_millis(900),
        "the first SYN got an answer: {took:?}"
    );
    assert!(took < Duration::from_secs(3), "{took:?}");
    let fresh = within(5, "accept of the new flow", listener.accept())
        .await
        .unwrap();
    assert_eq!(fresh.local_addr(), dst);
    assert_eq!(
        read_all(&mut stale).await,
        0,
        "the old flow ends as an end of file"
    );
}

/// Constraint 2: the stack ends the handshake before the app sees the flow.
/// The guest has its handshake with no `accept`, so no dial can come first.
#[tokio::test]
async fn guest_wire_handshake_ends_before_the_app_accepts() {
    let (router, guest, _wire) = pair(router_config(), guest_config());
    let mut listener = router.listen_any(443).await.unwrap();
    let mut c = guest.connect(at(DST, 443)).await.unwrap();
    within(3, "handshake with no accept", c.established())
        .await
        .unwrap();
    let s = within(3, "accept", listener.accept()).await.unwrap();
    assert_eq!(s.local_addr(), at(DST, 443));
}

/// Constraint 2: a hold of the SYN on the wire (the `HeldSyn` of the netns
/// backend) puts the dial first. The guest has no handshake while the pump
/// holds the SYN; the guest sends the SYN again (the pump holds each copy);
/// after the release the handshake ends at once.
#[tokio::test]
async fn guest_wire_held_syn_dials_before_the_handshake() {
    let (router, guest, wire) = pair(router_config(), guest_config());
    let mut listener = router.listen_any(443).await.unwrap();
    wire.hold.lock().unwrap().on = true;
    let mut c = guest.connect(at(DST, 443)).await.unwrap();
    let early = tokio::time::timeout(Duration::from_millis(500), c.established()).await;
    assert!(
        early.is_err(),
        "the handshake ended while the pump held the SYN"
    );
    assert!(wire.held() >= 1);
    // The dial takes 1.5 s: the guest sends its SYN again at 1 s.
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert!(
        wire.held() >= 2,
        "the guest did not send its SYN again: {}",
        wire.held()
    );
    wire.hold.lock().unwrap().on = false;
    let released = Instant::now();
    wire.release();
    within(3, "handshake after the release", c.established())
        .await
        .unwrap();
    assert!(released.elapsed() < Duration::from_millis(500));
    let s = within(3, "accept", listener.accept()).await.unwrap();
    assert_eq!(s.local_addr(), at(DST, 443));
    assert_eq!(
        router.stats().accepted,
        1,
        "the copies of the SYN made one flow"
    );
}

/// Constraint 2: a dial that fails after the handshake cannot reset the
/// guest. `TcpStream::abort` sends no RST (the reactor removes the socket
/// in the same pass), so the guest keeps an open flow that nothing serves.
/// Gap of `pod-netstack`; the ask is in the note.
#[tokio::test]
async fn guest_wire_abort_after_the_handshake_sends_no_reset() {
    let (router, guest, wire) = pair(router_config(), guest_config());
    let mut listener = router.listen_any(443).await.unwrap();
    let mut c = guest.connect(at(DST, 443)).await.unwrap();
    within(3, "handshake", c.established()).await.unwrap();
    let s = within(3, "accept", listener.accept()).await.unwrap();
    s.abort();
    let mut byte = [0u8; 1];
    let read = tokio::time::timeout(Duration::from_secs(1), c.read(&mut byte)).await;
    assert!(read.is_err(), "the guest saw the end of the flow: {read:?}");
    assert_eq!(wire.resets.down.load(Ordering::Relaxed), 0);
    assert!(
        c.write_all(b"zz").await.is_ok(),
        "the guest writes to a flow that is gone"
    );
}

/// Constraint 2: the accept queue holds four times the pool (32 flows at
/// the default). A flow past it has its handshake and is dropped with no
/// RST: the guest has an open flow, the router counts `dropped`.
#[tokio::test]
async fn guest_wire_accept_queue_drops_a_flow_after_the_handshake() {
    let (router, guest, wire) = pair(router_config(), guest_config());
    let mut listener = router.listen_any(443).await.unwrap();
    let mut flows = Vec::new();
    for _ in 0..60 {
        flows.push(guest.connect(at(DST, 443)).await.unwrap());
    }
    let mut established = 0;
    for flow in flows.iter_mut() {
        if tokio::time::timeout(Duration::from_secs(5), flow.established())
            .await
            .is_ok_and(|r| r.is_ok())
        {
            established += 1;
        }
    }
    assert_eq!(established, 60);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let mut accepted = 0;
    while tokio::time::timeout(Duration::from_millis(100), listener.accept())
        .await
        .is_ok()
    {
        accepted += 1;
    }
    let stats = router.stats();
    assert_eq!(accepted, 32);
    assert_eq!(stats.dropped, 28);
    assert_eq!(wire.resets.down.load(Ordering::Relaxed), 0);
}

/// Constraint 3: a burst of datagrams with no reader. The router keeps the
/// first ones (the channel of 64 and the buffer of 64 packets), loses the
/// rest, keeps the order, and counts none: `dropped` and `rx_dropped` stay 0.
#[tokio::test]
async fn guest_wire_udp_has_a_bounded_queue() {
    let (router, guest, _wire) = pair(router_config(), guest_config());
    let mut flows = router.bind_udp_any(5353).await.unwrap();
    let sender = guest.bind_udp(at("10.66.0.4", 40000)).await.unwrap();
    for i in 0..500u32 {
        sender
            .send_to(&i.to_be_bytes(), at("1.1.1.1", 5353))
            .await
            .unwrap();
    }
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let mut got = Vec::new();
    let mut buf = [0u8; 64];
    while let Ok(Ok((n, _, _))) =
        tokio::time::timeout(Duration::from_millis(300), flows.recv_from_to(&mut buf)).await
    {
        got.push(u32::from_be_bytes(buf[..n].try_into().unwrap()));
    }
    assert!(got.len() >= 64, "{}", got.len());
    assert!(got.len() < 250, "the queue did not bound: {}", got.len());
    assert!(got.windows(2).all(|w| w[0] < w[1]), "the order changed");
    assert_eq!(router.stats().dropped, 0);
    assert_eq!(router.device_stats().rx_dropped, 0);
    assert_eq!(
        router.device_stats().rx_packets,
        500,
        "the wire gave each datagram to the stack"
    );
}

/// Constraint 3: a datagram over `mtu - 28` bytes is lost, up and down, with
/// `Ok` for the sender and no counter. The smoltcp of podnet has no IPv4
/// fragmentation, so the size of 65535 bytes of the note does not arise
/// before the features `proto-ipv4-fragmentation` and
/// `reassembly-buffer-size-65536` (measured, see the note).
#[tokio::test]
async fn guest_wire_udp_over_the_mtu_is_lost_without_fragmentation() {
    for (mtu, limit) in [(1280usize, 1252usize), (1500, 1472)] {
        let with_mtu = |c: StackConfig| StackConfig { mtu, ..c };
        let (router, guest, _wire) = pair(with_mtu(router_config()), with_mtu(guest_config()));
        let mut flows = router.bind_udp_any(5353).await.unwrap();
        let mut guest_socket = guest.bind_udp(at("10.66.0.4", 40000)).await.unwrap();
        let mut buf = vec![0u8; 70000];
        for (size, arrives) in [(limit, true), (limit + 1, false), (65507, false)] {
            let sent = guest_socket
                .send_to(&vec![7u8; size], at("1.1.1.1", 5353))
                .await;
            assert!(
                sent.is_ok(),
                "mtu {mtu} size {size}: the sender sees no error"
            );
            let got =
                tokio::time::timeout(Duration::from_millis(600), flows.recv_from_to(&mut buf))
                    .await;
            assert_eq!(got.is_ok(), arrives, "mtu {mtu} up size {size}");
        }
        assert_eq!(
            guest.stats().datagrams,
            3,
            "the stack counts a datagram that it could not send"
        );
        guest_socket
            .send_to(b"hello", at("1.1.1.1", 5353))
            .await
            .unwrap();
        let (_, from, to) = within(3, "datagram", flows.recv_from_to(&mut buf))
            .await
            .unwrap();
        for (size, arrives) in [(limit, true), (limit + 1, false), (70000, false)] {
            let sent = flows.send_to_from(&vec![1u8; size], from, to.ip()).await;
            assert!(
                sent.is_ok(),
                "mtu {mtu} size {size}: the sender sees no error"
            );
            let got =
                tokio::time::timeout(Duration::from_millis(600), guest_socket.recv_from(&mut buf))
                    .await;
            assert_eq!(got.is_ok(), arrives, "mtu {mtu} down size {size}");
        }
    }
}

/// Constraint 3: the stack keeps the byte offset of a partial write. A
/// socket buffer of 4 KiB takes 2 MB with a slow reader; each byte arrives
/// in its place.
#[tokio::test]
async fn guest_wire_partial_write_keeps_the_byte_order() {
    let small = |c: StackConfig| StackConfig {
        tcp_buffer: 4096,
        ..c
    };
    let (router, guest, _wire) = pair(small(router_config()), small(guest_config()));
    let mut listener = router.listen_any(443).await.unwrap();
    let mut c = guest.connect(at(DST, 443)).await.unwrap();
    within(3, "handshake", c.established()).await.unwrap();
    let mut s = within(3, "accept", listener.accept()).await.unwrap();
    let data: Vec<u8> = (0..2_000_000u32)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8)
        .collect();
    let copy = data.clone();
    let writer = tokio::spawn(async move {
        s.write_all(&copy).await.unwrap();
        s.shutdown();
        s
    });
    let mut got = Vec::new();
    let mut buf = [0u8; 700];
    loop {
        let n = within(10, "read", c.read(&mut buf)).await.unwrap();
        if n == 0 {
            break;
        }
        got.extend_from_slice(&buf[..n]);
        if got.len() % 50_000 < 700 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    }
    assert!(got == data, "the stream changed (length {})", got.len());
    let _ = writer.await;
}

/// A drop of the router stream right after `write_all` loses the queued
/// bytes (gap 2 of PodUtils/podnet#457). A `shutdown` and a held handle
/// deliver each byte: the rule for a router.
#[tokio::test]
async fn guest_wire_a_drop_loses_the_tail_and_a_held_shutdown_does_not() {
    let small = |c: StackConfig| StackConfig {
        tcp_buffer: 4096,
        ..c
    };
    let (router, guest, _wire) = pair(small(router_config()), small(guest_config()));
    let mut listener = router.listen_any(443).await.unwrap();
    const TAIL: usize = 1_000_000;
    // A drop.
    let mut c = guest.connect(at(DST, 443)).await.unwrap();
    within(3, "handshake", c.established()).await.unwrap();
    let mut s = within(3, "accept", listener.accept()).await.unwrap();
    s.write_all(&vec![5u8; TAIL]).await.unwrap();
    drop(s);
    let lost = read_all(&mut c).await;
    assert!(lost < TAIL, "the gap is closed: update the note ({lost})");
    // A shutdown, and the handle held to the end of the guest.
    let mut c = guest.connect(at(DST, 443)).await.unwrap();
    within(3, "handshake", c.established()).await.unwrap();
    let mut s = within(3, "accept", listener.accept()).await.unwrap();
    s.write_all(&vec![5u8; TAIL]).await.unwrap();
    s.shutdown();
    assert_eq!(read_all(&mut c).await, TAIL);
    drop(s);
}

/// Constraint 3: the stop-reading signal of the wire is a poll of the
/// queue. A full `PacketQueue` drops the packet, counts it, and says
/// `is_full`; it has no wait for room.
#[tokio::test]
async fn guest_wire_link_queue_says_full_and_has_no_wait_for_room() {
    let queue = PacketQueue::bounded(2);
    assert!(queue.push(vec![1]) && queue.push(vec![2]));
    assert!(queue.is_full());
    assert!(!queue.push(vec![3]));
    assert_eq!(queue.dropped(), 1);
    assert_eq!(queue.pop(), Some(vec![1]));
    assert!(!queue.is_full());
}
