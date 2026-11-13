//! The HTTP poll simplex carrier of the link (T-280): POSTs up and a GET poll down, HTTP/1.1 over TLS 1.3 with the Ed25519 key of each end pinned; the highest overhead class.
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
fn the_simplex_poll_reports_the_overhead_class_highest() {
    assert_eq!(overhead_of("httppoll"), "highest");
    let dir = tempfile::tempdir().unwrap();
    let me = link_peer::install(dir.path(), "me");
    let form = link_poll::form_of("httppoll").unwrap();
    let carrier = link_poll::carrier(&me, &AllowList::default(), form, &Tune::default());
    assert_eq!(carrier.facts().cost, pod_link::Cost::Highest);
    assert_eq!(form.batch(), 64 * 1024);
}

// ---- TCP loopback ----

#[test]
fn the_registry_row_of_httppoll_has_no_key_and_refuses_a_dial() {
    link_peer::the_registry_row_refuses_a_dial("httppoll", "poll", "x@https://127.0.0.1:1/link");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_link_crosses_httppoll_and_each_end_learns_the_key_of_the_other() {
    test_host::need_loopback!();
    link_peer::a_link_crosses(&case("httppoll"), 512 * 1024).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_dialer_of_httppoll_that_the_listener_does_not_allow_exits_77_and_the_door_stays_open() {
    test_host::need_loopback!();
    link_peer::a_stranger_exits_77(&case("httppoll")).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_listener_with_another_key_than_the_pinned_one_exits_77_on_httppoll() {
    test_host::need_loopback!();
    link_peer::another_key_exits_77(&case("httppoll")).await;
}

#[test]
fn link_dial_check_meets_link_listen_over_httppoll_and_a_stranger_exits_77() {
    test_host::need_loopback!();
    link_peer::cli_check("httppoll", "127.0.0.1:0", &[], address_of, &[]);
}
