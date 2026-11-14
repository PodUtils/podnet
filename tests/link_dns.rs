//! The DNS simplex carrier of the link (T-278): query names up and TXT
//! answers down, the ARQ of `pod-link` and TLS 1.3 above it with the
//! Ed25519 key of each end pinned. The listener is a local authoritative
//! server of the zone on the UDP loopback. The tests with no socket run on
//! each host. A test that needs the loopback prints `skipping, REASON` on a
//! host that denies it, and does not fail.
#![cfg(feature = "dns")]

#[path = "test_host/mod.rs"]
mod test_host;

#[path = "link_peer/mod.rs"]
mod link_peer;

use link_peer::{install, list_of, Case};
use pod_link::{Carrier, Cost};
use podnet::exit::code_of;
use podnet::identity::{AllowList, Identity};
use podnet::link::Door;
use podnet::link_dns;
use podnet::link_keyed::{self, Tune};

const ZONE: &str = "t.example";

fn address_of(key: &str, _: &str) -> String {
    format!("{key}@{ZONE}")
}

/// The tune of a listener of the zone.
fn zone() -> Tune {
    Tune {
        dns_zone: Some(ZONE.into()),
        ..Tune::default()
    }
}

/// The tune of a dialer that sends its queries to the listener `door`.
fn toward(door: &Door) -> Tune {
    Tune {
        dns_server: Some(door.address().parse().unwrap()),
        ..Tune::default()
    }
}

async fn listen(me: &Identity, allow: &AllowList) -> Door {
    link_keyed::listen("dns", &zone(), me, allow, "127.0.0.1:0")
        .await
        .unwrap()
}

/// The case of the dialer of `door`.
fn case(door: &Door) -> Case {
    Case {
        name: "dns",
        tune: toward(door),
        address_of,
    }
}

// ---- no socket ----

#[test]
fn the_carrier_is_a_stream_of_the_highest_overhead() {
    let dir = tempfile::tempdir().unwrap();
    let me = install(dir.path(), "me");
    let carrier = link_dns::carrier(&me, &AllowList::default(), &zone());
    assert_eq!(carrier.name(), "dns");
    assert_eq!(carrier.zone, ZONE);
    let facts = carrier.facts();
    assert!(facts.reliable && facts.ordered && facts.duplex);
    assert_eq!(facts.cost, Cost::Highest);
    assert_eq!(carrier.alpn, pod_link::tcp::ALPN);
}

#[test]
fn the_registry_row_has_no_key_and_refuses_a_dial() {
    link_peer::the_registry_row_refuses_a_dial("dns", "dns", "x@t.example");
}

#[tokio::test]
async fn a_listener_with_no_zone_and_an_address_with_no_key_exit_64() {
    let dir = tempfile::tempdir().unwrap();
    let me = install(dir.path(), "me");
    let e = link_keyed::listen(
        "dns",
        &Tune::default(),
        &me,
        &list_of(&[&me]),
        "127.0.0.1:0",
    )
    .await
    .err()
    .unwrap();
    assert_eq!(code_of(&e), 64, "{e:#}");
    assert!(format!("{e:#}").contains("--dns-zone"), "{e:#}");
    let to = Case {
        name: "dns",
        tune: Tune {
            dns_server: Some("127.0.0.1:53".parse().unwrap()),
            ..Tune::default()
        },
        address_of,
    };
    link_peer::no_key_exits_64(&to, "t.example").await;
    link_peer::no_key_exits_64(&to, "nokey@t.example").await;
}

#[test]
fn the_dns_flags_are_for_the_dns_carrier() {
    let dir = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| link_peer::podnet(dir.path(), "s", args).output().unwrap();
    let out = run(&[
        "link",
        "listen",
        "/tmp/x",
        "--dns-zone",
        ZONE,
        "--allow",
        "/nonexistent",
    ]);
    assert_eq!(out.status.code(), Some(64), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("dns carrier"),
        "{out:?}"
    );
    // The flags pass: the next step reads the allow list (exit 66).
    let out = run(&[
        "link",
        "listen",
        "127.0.0.1:0",
        "--carrier",
        "dns",
        "--dns-zone",
        ZONE,
        "--allow",
        "/nonexistent",
    ]);
    assert_eq!(out.status.code(), Some(66), "{out:?}");
}

// ---- UDP loopback: a local authoritative server ----

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_link_crosses_dns_and_each_end_learns_the_key_of_the_other() {
    test_host::need_loopback!();
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (install(dir.path(), "a"), install(dir.path(), "b"));
    let mut door = listen(&b, &list_of(&[&a])).await;
    let to = address_of(&b.fingerprint(), "");
    let (dialed, accepted) = case(&door).meet(&a, list_of(&[&b]), &to, &mut door).await;
    let ((near, seen_by_a), (far, seen_by_b)) = (dialed.unwrap(), accepted.unwrap());
    assert_eq!(seen_by_a, b.public());
    assert_eq!(seen_by_b, Some(a.public()));
    link_peer::echo(&near, &far, &[7u8; 4096]).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_dialer_that_the_listener_does_not_allow_exits_77_and_the_door_stays_open() {
    test_host::need_loopback!();
    let dir = tempfile::tempdir().unwrap();
    let (a, b, c) = (
        install(dir.path(), "a"),
        install(dir.path(), "b"),
        install(dir.path(), "c"),
    );
    let mut door = listen(&b, &list_of(&[&a])).await;
    let to = address_of(&b.fingerprint(), "");
    let (dialed, accepted) = case(&door).meet(&c, list_of(&[&b]), &to, &mut door).await;
    let refused = dialed.expect_err("the dial is refused");
    assert_eq!(code_of(&refused), 77, "{refused:#}");
    // T-557: the listener gets the refusal too, with the exit 77.
    let accepted = accepted.expect_err("the listener takes no peer");
    assert_eq!(code_of(&accepted), 77, "{accepted:#}");
    let (dialed, accepted) = case(&door).meet(&a, list_of(&[&b]), &to, &mut door).await;
    dialed.unwrap();
    assert_eq!(accepted.unwrap().1, Some(a.public()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_listener_with_another_key_than_the_pinned_one_exits_77() {
    test_host::need_loopback!();
    let dir = tempfile::tempdir().unwrap();
    let (a, b, c) = (
        install(dir.path(), "a"),
        install(dir.path(), "b"),
        install(dir.path(), "c"),
    );
    let mut door = listen(&b, &list_of(&[&a])).await;
    let to = address_of(&c.fingerprint(), "");
    let (dialed, accepted) = case(&door)
        .meet(&a, list_of(&[&b, &c]), &to, &mut door)
        .await;
    let e = dialed.err().unwrap();
    assert_eq!(code_of(&e), 77, "{e:#}");
    // T-557: the listener gets `KeyRefused`, with the exit 77.
    let e = accepted.err().expect("the listener takes no peer");
    assert_eq!(code_of(&e), 77, "{e:#}");
}

// ---- the commands ----

#[test]
fn link_dial_check_meets_link_listen_over_dns_and_a_stranger_exits_77() {
    test_host::need_loopback!();
    link_peer::cli_check(
        "dns",
        "127.0.0.1:0",
        &["--dns-zone", ZONE],
        address_of,
        &["--dns-server", "{listen}"],
    );
}
