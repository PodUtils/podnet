//! The udp carrier of the link (T-276): KCP over UDP, TLS 1.3 above it with
//! the Ed25519 key of each end pinned in the handshake. The tests with no
//! socket run on each host. A test that needs the loopback prints
//! `skipping, REASON` on a host that denies it, and does not fail. The loss
//! tests run the settings of podnet over the lossy shim of `pod-link`; the
//! gates of row E2-1 in a release build are tests of `pod-link`
//! (`udp_loss`).
#![cfg(feature = "udp")]

#[path = "test_host/mod.rs"]
mod test_host;

#[path = "link_peer/mod.rs"]
mod link_peer;

use std::sync::Arc;
use std::time::Duration;

use link_peer::Case;
use pod_core::Target;
use pod_identity::KeyName;
use pod_link::datagram::lossy::{Faults, Lossy};
use pod_link::datagram::memory::{pair, MemoryDatagrams, DEFAULT_DEPTH, DEFAULT_MTU};
use pod_link::udp::MAX_WINDOW;
use pod_link::{BoxIo, Carrier};
use podnet::identity::AllowList;
use podnet::link_keyed::Tune;
use podnet::link_udp;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::timeout;

fn address_of(key: &str, door: &str) -> String {
    format!("{key}@{door}")
}

fn case(tune: Tune) -> Case {
    Case {
        name: "udp",
        tune,
        address_of,
    }
}

fn nodelay() -> Tune {
    Tune {
        kcp_nodelay: true,
        ..Tune::default()
    }
}

// ---- no socket ----

#[test]
fn the_carrier_is_a_stream_and_kcp_has_the_settings_of_the_query() {
    let dir = tempfile::tempdir().unwrap();
    let me = link_peer::install(dir.path(), "me");
    let carrier = link_udp::carrier(&me, &AllowList::default(), &Tune::default());
    assert_eq!(carrier.name(), "udp");
    let facts = carrier.facts();
    assert!(facts.reliable && facts.ordered && facts.duplex);
    assert_eq!(carrier.alpn, pod_link::tcp::ALPN);
    assert!(
        !carrier.kcp.nodelay,
        "the congestion window of KCP by default"
    );
    assert_eq!(carrier.kcp.mtu, 1200);
    let fast = link_udp::carrier(&me, &AllowList::default(), &nodelay());
    assert!(fast.kcp.nodelay);
    assert_eq!(fast.kcp.window, MAX_WINDOW);
    if let Err(e) = carrier.available() {
        assert!(e.to_string().contains("UDP"), "{e}");
    }
}

#[test]
fn the_registry_row_has_no_key_and_refuses_a_dial() {
    link_peer::the_registry_row_refuses_a_dial("udp", "udp", "x@127.0.0.1:1");
}

#[tokio::test]
async fn a_dial_address_with_no_key_exits_64() {
    link_peer::no_key_exits_64(&case(Tune::default()), "127.0.0.1:7000").await;
    link_peer::no_key_exits_64(&case(Tune::default()), "nokey@127.0.0.1:7000").await;
}

#[test]
fn the_query_keys_of_udp_are_checked_before_a_socket_opens() {
    let dir = tempfile::tempdir().unwrap();
    let run = |address: &str, carrier: &str| {
        link_peer::podnet(
            dir.path(),
            "s",
            &[
                "link",
                "listen",
                address,
                "--carrier",
                carrier,
                "--allow",
                "/nonexistent",
            ],
        )
        .output()
        .unwrap()
    };
    for (address, carrier, want) in [
        ("/tmp/x?kcp-nodelay=1", "unix", "kcp-nodelay"),
        ("127.0.0.1:0?kcp-nodelay=2", "udp", "kcp-nodelay"),
        ("127.0.0.1:0?mtu=1500", "udp", "64 to 1200"),
        ("/tmp/x?mtu=1200", "unix", "stream carrier"),
    ] {
        let out = run(address, carrier);
        assert_eq!(out.status.code(), Some(64), "{address} {carrier}: {out:?}");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(err.contains(want), "{address} {carrier}: {err}");
    }
    // The keys pass: the next step reads the allow list (exit 66).
    let out = run("127.0.0.1:0?kcp-nodelay=1&mtu=576", "udp");
    assert_eq!(out.status.code(), Some(66), "{out:?}");
}

// ---- the lossy shim ----

/// The two ends of the carrier of podnet with `tune` over a memory pair
/// with `faults` in each direction, and the shims.
async fn ends(faults: Faults, tune: &Tune) -> (BoxIo, BoxIo, Arc<Lossy<MemoryDatagrams>>) {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (
        link_peer::install(dir.path(), "a"),
        link_peer::install(dir.path(), "b"),
    );
    let listening = link_udp::carrier(&b, &link_peer::list_of(&[&a]), tune);
    let dialing = link_udp::carrier(&a, &AllowList::default(), tune);
    let (near, far) = pair(DEFAULT_MTU, DEFAULT_DEPTH);
    let (near, far) = (
        Arc::new(Lossy::new(near, faults, 1)),
        Arc::new(Lossy::new(far, faults, 2)),
    );
    let target = Target::Addr("192.0.2.1:7000".parse().unwrap());
    let key = KeyName::Key(b.public());
    let (dialed, accepted) = timeout(link_peer::LIMIT, async {
        tokio::join!(
            dialing.connect_over(near.clone(), &target, &key),
            listening.accept_over(far.clone(), &"the peer")
        )
    })
    .await
    .expect("the handshake in time");
    let (dialed, accepted) = (dialed.unwrap(), accepted.unwrap());
    assert_eq!(dialed.1, b.public());
    assert_eq!(accepted.1, a.public());
    (dialed.0, accepted.0, near)
}

/// `size` bytes each way at once, whole and in order, in `limit`.
async fn cross(near: BoxIo, far: BoxIo, size: usize, limit: Duration) {
    let (mut near_read, mut near_write) = tokio::io::split(near);
    let (mut far_read, mut far_write) = tokio::io::split(far);
    let up: Vec<u8> = (0..size).map(|i| (i % 253) as u8).collect();
    let down: Vec<u8> = (0..size).map(|i| (i % 241) as u8).collect();
    let all = async {
        tokio::join!(
            async {
                near_write.write_all(&up).await.unwrap();
                near_write.shutdown().await.unwrap();
            },
            async {
                far_write.write_all(&down).await.unwrap();
                far_write.shutdown().await.unwrap();
            },
            async {
                let mut got = Vec::new();
                far_read.read_to_end(&mut got).await.unwrap();
                got
            },
            async {
                let mut got = Vec::new();
                near_read.read_to_end(&mut got).await.unwrap();
                got
            },
        )
    };
    let (_, _, got_up, got_down) = timeout(limit, all).await.expect("the transfer in time");
    assert!(got_up == up, "up: {} of {} bytes", got_up.len(), up.len());
    assert!(
        got_down == down,
        "down: {} of {} bytes",
        got_down.len(),
        down.len()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_mib_crosses_each_way_at_five_percent_loss_with_the_defaults() {
    let faults = Faults {
        loss: 0.05,
        reorder: 0.01,
        duplicate: 0.0,
    };
    let (near, far, shim) = ends(faults, &Tune::default()).await;
    cross(near, far, 1 << 20, Duration::from_secs(240)).await;
    assert!(shim.stats().lost > 0, "{:?}", shim.stats());
}

/// The round trip of row E2-1 (8 MiB at 5 % drop and 1 % reorder) with
/// `kcp-nodelay=1`: the "nodelay" of KCP and a window of 1024 segments.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn eight_mib_cross_each_way_at_five_percent_loss_with_kcp_nodelay() {
    let faults = Faults {
        loss: 0.05,
        reorder: 0.01,
        duplicate: 0.0,
    };
    let (near, far, shim) = ends(faults, &nodelay()).await;
    cross(near, far, 8 << 20, Duration::from_secs(240)).await;
    assert!(shim.stats().lost > 0, "{:?}", shim.stats());
}

// ---- UDP loopback ----

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_link_crosses_udp_and_each_end_learns_the_key_of_the_other() {
    test_host::need_loopback!();
    link_peer::a_link_crosses(&case(Tune::default()), 256 * 1024).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_link_crosses_udp_with_kcp_nodelay_and_a_small_mtu() {
    test_host::need_loopback!();
    let tune = Tune {
        kcp_nodelay: true,
        mtu: Some(576),
        ..Tune::default()
    };
    link_peer::a_link_crosses(&case(tune), 256 * 1024).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_dialer_that_the_listener_does_not_allow_exits_77_and_the_door_stays_open() {
    test_host::need_loopback!();
    link_peer::a_stranger_exits_77(&case(Tune::default())).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_listener_with_another_key_than_the_pinned_one_exits_77() {
    test_host::need_loopback!();
    link_peer::another_key_exits_77(&case(Tune::default())).await;
}

// ---- the commands ----

#[test]
fn link_dial_check_meets_link_listen_over_udp_and_a_stranger_exits_77() {
    test_host::need_loopback!();
    link_peer::cli_check(
        "udp",
        "127.0.0.1:0?kcp-nodelay=1",
        &[],
        |key, door| format!("{key}@{door}?kcp-nodelay=1"),
        &[],
    );
}
