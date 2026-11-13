//! The long-poll carrier of the link (T-271): POSTs up and a held POST down, HTTP/1.1 over TLS 1.3 with the Ed25519 key of each end pinned in the handshake.
//! The tests with no socket run on each host. A test that needs the
//! loopback prints `skipping, REASON` on a host that denies it, and does
//! not fail.
#![cfg(feature = "poll")]

#[path = "test_host/mod.rs"]
mod test_host;

#[path = "link_peer/mod.rs"]
mod link_peer;

use link_peer::Case;
use pod_link::Carrier;
use podnet::identity::AllowList;
use podnet::link_keyed::Tune;
use podnet::link_poll;

fn address_of(key: &str, door: &str) -> String {
    format!("{key}@https://{door}/link")
}

fn case(name: &'static str) -> Case {
    Case {
        name,
        tune: Tune::default(),
        address_of,
    }
}

/// The overhead class of `name` in `link carriers --json`.
fn overhead_of(name: &str) -> String {
    let dir = tempfile::tempdir().unwrap();
    let out = link_peer::podnet(dir.path(), "s", &["link", "carriers", "--json"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let row = doc["carriers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == name)
        .unwrap()
        .clone();
    assert_eq!(row["feature"], "poll");
    row["overhead"].as_str().unwrap().to_string()
}

// ---- no socket ----

#[test]
fn the_query_key_poll_sets_the_hold_of_a_long_poll() {
    let dir = tempfile::tempdir().unwrap();
    let me = link_peer::install(dir.path(), "me");
    let form = link_poll::form_of("poll").unwrap();
    let base = link_poll::carrier(&me, &AllowList::default(), form, &Tune::default());
    assert_eq!(base.name(), "poll");
    assert_eq!(base.hold, pod_link::poll::HOLD);
    let tune = Tune {
        hold: Some(60),
        ..Tune::default()
    };
    let long = link_poll::carrier(&me, &AllowList::default(), form, &tune);
    assert_eq!(long.hold.as_secs(), 60);
    assert!(
        long.idle > long.hold,
        "the idle time is longer than the hold"
    );
    assert!(long.check().is_ok());
}

#[test]
fn the_query_key_poll_is_for_the_poll_carriers_only() {
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
    let out = run("/tmp/x?poll=5", "unix");
    assert_eq!(out.status.code(), Some(64), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("poll"),
        "{out:?}"
    );
    // The key passes: the next step reads the allow list (exit 66).
    let out = run("127.0.0.1:0?poll=5", "poll");
    assert_eq!(out.status.code(), Some(66), "{out:?}");
}

#[test]
fn the_long_poll_has_the_overhead_class_medium() {
    assert_eq!(overhead_of("poll"), "medium");
}

#[tokio::test]
async fn a_dial_address_with_no_key_or_a_path_with_a_query_exits_64() {
    link_peer::no_key_exits_64(&case("poll"), "https://127.0.0.1:7000/link").await;
    link_peer::no_key_exits_64(&case("poll"), "nokey@https://127.0.0.1:7000/link").await;
    link_peer::no_key_exits_64(&case("poll"), "x@http://127.0.0.1:7000/link").await;
}

// ---- TCP loopback ----

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_link_crosses_poll_with_a_short_hold() {
    test_host::need_loopback!();
    let mut short = case("poll");
    short.tune.hold = Some(1);
    link_peer::a_link_crosses(&short, 64 * 1024).await;
}

#[test]
fn the_registry_row_of_poll_has_no_key_and_refuses_a_dial() {
    link_peer::the_registry_row_refuses_a_dial("poll", "poll", "x@https://127.0.0.1:1/link");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_link_crosses_poll_and_each_end_learns_the_key_of_the_other() {
    test_host::need_loopback!();
    link_peer::a_link_crosses(&case("poll"), 512 * 1024).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_dialer_of_poll_that_the_listener_does_not_allow_exits_77_and_the_door_stays_open() {
    test_host::need_loopback!();
    link_peer::a_stranger_exits_77(&case("poll")).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_listener_with_another_key_than_the_pinned_one_exits_77_on_poll() {
    test_host::need_loopback!();
    link_peer::another_key_exits_77(&case("poll")).await;
}

#[test]
fn link_dial_check_meets_link_listen_over_poll_and_a_stranger_exits_77() {
    test_host::need_loopback!();
    link_peer::cli_check("poll", "127.0.0.1:0", &[], address_of, &[]);
}
