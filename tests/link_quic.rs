//! The quic carrier of the link (T-273): one stream of a QUIC connection,
//! TLS 1.3 with the Ed25519 key of each end pinned in the handshake. The
//! tests with no socket run on each host. A test that needs the loopback
//! prints `skipping, REASON` on a host that denies it, and does not fail.
#![cfg(feature = "quic")]

#[path = "test_host/mod.rs"]
mod test_host;

#[path = "link_peer/mod.rs"]
mod link_peer;

use link_peer::Case;
use pod_link::Carrier;
use podnet::identity::AllowList;
use podnet::link_keyed::Tune;
use podnet::link_quic;

fn address_of(key: &str, door: &str) -> String {
    format!("{key}@{door}")
}

fn case() -> Case {
    Case {
        name: "quic",
        tune: Tune::default(),
        address_of,
    }
}

// ---- no socket ----

#[test]
fn the_carrier_is_a_stream_with_the_alpn_of_tcp() {
    let dir = tempfile::tempdir().unwrap();
    let me = link_peer::install(dir.path(), "me");
    let carrier = link_quic::carrier(&me, &AllowList::default());
    assert_eq!(carrier.name(), "quic");
    let facts = carrier.facts();
    assert!(facts.reliable && facts.ordered && facts.duplex);
    assert_eq!(facts.mtu, None);
    assert_eq!(carrier.alpn, pod_link::tcp::ALPN);
    if let Err(e) = carrier.available() {
        assert!(e.to_string().contains("UDP"), "{e}");
    }
}

#[test]
fn the_registry_row_has_no_key_and_refuses_a_dial() {
    link_peer::the_registry_row_refuses_a_dial("quic", "quic", "x@127.0.0.1:1");
}

#[tokio::test]
async fn a_dial_address_with_no_key_exits_64() {
    link_peer::no_key_exits_64(&case(), "127.0.0.1:7000").await;
    link_peer::no_key_exits_64(&case(), "nokey@127.0.0.1:7000").await;
}

// ---- UDP loopback ----

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_link_crosses_quic_and_each_end_learns_the_key_of_the_other() {
    test_host::need_loopback!();
    link_peer::a_link_crosses(&case(), 256 * 1024).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_dialer_that_the_listener_does_not_allow_exits_77_and_the_door_stays_open() {
    test_host::need_loopback!();
    link_peer::a_stranger_exits_77(&case()).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_listener_with_another_key_than_the_pinned_one_exits_77() {
    test_host::need_loopback!();
    link_peer::another_key_exits_77(&case()).await;
}

// ---- the commands ----

#[test]
fn link_dial_check_meets_link_listen_over_quic_and_a_stranger_exits_77() {
    test_host::need_loopback!();
    link_peer::cli_check("quic", "127.0.0.1:0", &[], address_of, &[]);
}

#[test]
fn a_query_key_of_another_carrier_exits_64() {
    let dir = tempfile::tempdir().unwrap();
    let out = link_peer::podnet(
        dir.path(),
        "s",
        &[
            "link",
            "listen",
            "127.0.0.1:0?poll=5",
            "--carrier",
            "quic",
            "--allow",
            "/nonexistent",
        ],
    )
    .output()
    .unwrap();
    assert_eq!(out.status.code(), Some(64), "{out:?}");
}
