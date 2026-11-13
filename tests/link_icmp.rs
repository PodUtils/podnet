//! The icmp carrier of the link (T-282): the frames in the data of ICMP
//! echo messages on a raw ICMPv4 socket, the ARQ of `pod-link` and TLS 1.3
//! with the Ed25519 key of each end pinned above. The tests with no socket
//! and the tests of a refusal run on each host. A test that needs the raw
//! socket prints `skipping, REASON` on a host that refuses it (no
//! `CAP_NET_RAW`), and does not fail.
#![cfg(feature = "icmp")]

#[path = "test_host/mod.rs"]
mod test_host;

#[path = "link_peer/mod.rs"]
mod link_peer;

use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Command;

use link_peer::Case;
use pod_link::Carrier;
use podnet::exit::code_of;
use podnet::identity::AllowList;
use podnet::link_icmp;
use podnet::link_keyed::Tune;

/// The uid and the gid of a user with no privilege, for a root host.
const NOBODY: u32 = 65534;

fn address_of(key: &str, door: &str) -> String {
    format!("{key}@{door}")
}

fn case() -> Case {
    Case {
        name: "icmp",
        tune: Tune::default(),
        address_of,
    }
}

/// `Err(reason)` when this host refuses a raw ICMP socket to this process.
fn raw_socket() -> Result<(), String> {
    let dir = tempfile::tempdir().unwrap();
    let me = link_peer::install(dir.path(), "me");
    link_icmp::carrier(&me, &AllowList::default())
        .available()
        .map_err(|e| format!("this host refuses a raw ICMP socket: {e}"))
}

/// One test with a raw socket at a time: each listener on one address
/// takes each echo request to it, so two listeners of two tests would both
/// answer one dialer.
static RAW: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Skip the test (return from it) when [`raw_socket`] fails.
macro_rules! need_raw_socket {
    () => {
        test_host::need_loopback!();
        if let Err(why) = raw_socket() {
            test_host::skip_line(&why);
            return;
        }
    };
}

/// A command of podnet with no privilege for a raw socket: as the user
/// [`NOBODY`] on a root host, as this user on another host. `None` when
/// this host gives the privilege to a user that is not root.
fn without_raw(dir: &Path, state: &str, args: &[&str]) -> Option<Command> {
    let mut cmd = link_peer::podnet(dir, state, args);
    // SAFETY: `geteuid` reads the effective uid; it has no failure.
    if unsafe { libc::geteuid() } == 0 {
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o777)).unwrap();
        // The allow list of a command must be the file of its user.
        std::os::unix::fs::chown(dir.join("allow"), Some(NOBODY), Some(NOBODY)).unwrap();
        cmd.uid(NOBODY).gid(NOBODY);
        return Some(cmd);
    }
    raw_socket().is_err().then_some(cmd)
}

// ---- no socket ----

#[test]
fn the_carrier_is_a_stream_of_high_overhead_with_the_alpn_of_tcp() {
    let dir = tempfile::tempdir().unwrap();
    let me = link_peer::install(dir.path(), "me");
    let carrier = link_icmp::carrier(&me, &AllowList::default());
    assert_eq!(carrier.name(), "icmp");
    let facts = carrier.facts();
    assert!(facts.reliable && facts.ordered && facts.duplex);
    assert_eq!(facts.mtu, None);
    assert_eq!(facts.cost, pod_link::Cost::High);
    assert_eq!(carrier.alpn, pod_link::tcp::ALPN);
    assert_eq!(carrier.icmp.payload, pod_link::icmp::PAYLOAD);
    if let Err(e) = carrier.available() {
        assert!(e.to_string().contains("CAP_NET_RAW"), "{e}");
    }
}

#[test]
fn the_registry_row_has_no_key_and_refuses_a_dial() {
    link_peer::the_registry_row_refuses_a_dial("icmp", "icmp", "x@127.0.0.1");
    let report = podnet::carriers::report(&podnet::carriers::Options::default());
    let row = report.iter().find(|r| r.name == "icmp").unwrap();
    assert!(row.needs_privilege);
    assert!(row.about.contains("--allow-raw icmp"), "{}", row.about);
    if let Err(reason) = &row.available {
        assert!(reason.contains("CAP_NET_RAW"), "{reason}");
    }
}

#[tokio::test]
async fn a_dial_address_with_no_key_exits_64() {
    link_peer::no_key_exits_64(&case(), "127.0.0.1").await;
    link_peer::no_key_exits_64(&case(), "nokey@127.0.0.1").await;
}

#[tokio::test]
async fn a_listen_address_with_a_port_exits_64_before_the_socket() {
    let dir = tempfile::tempdir().unwrap();
    let me = link_peer::install(dir.path(), "me");
    let allow = link_peer::list_of(&[&me]);
    for address in ["127.0.0.1:0", "::1", "localhost"] {
        let e = podnet::link_keyed::listen("icmp", &Tune::default(), &me, &allow, address)
            .await
            .err()
            .unwrap();
        assert_eq!(code_of(&e), 64, "{address}: {e:#}");
        assert!(format!("{e:#}").contains("IPv4"), "{e:#}");
    }
}

// ---- the commands, with no raw socket ----

#[test]
fn without_allow_raw_the_icmp_carrier_exits_64_and_names_the_flag() {
    let dir = tempfile::tempdir().unwrap();
    for args in [
        &[
            "link",
            "listen",
            "127.0.0.1",
            "--carrier",
            "icmp",
            "--allow",
            "/nonexistent",
        ][..],
        &[
            "link",
            "dial",
            "x@127.0.0.1",
            "--carrier",
            "icmp",
            "--check",
        ][..],
        &[
            "link",
            "listen",
            "127.0.0.1?carrier=icmp",
            "--allow",
            "/nonexistent",
        ][..],
    ] {
        let out = link_peer::podnet(dir.path(), "s", args).output().unwrap();
        assert_eq!(out.status.code(), Some(64), "{args:?}: {out:?}");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(err.contains("--allow-raw icmp"), "{args:?}: {err}");
    }
    let out = link_peer::podnet(
        dir.path(),
        "s",
        &[
            "link",
            "listen",
            "127.0.0.1",
            "--carrier",
            "icmp",
            "--allow-raw",
            "udp",
            "--allow",
            "/nonexistent",
        ],
    )
    .output()
    .unwrap();
    assert_eq!(out.status.code(), Some(64), "{out:?}");
}

#[test]
fn without_cap_net_raw_the_carrier_exits_69_and_the_row_says_why() {
    let dir = tempfile::tempdir().unwrap();
    let me = link_peer::install(dir.path(), "a");
    let allow = link_peer::write_allow(dir.path(), "allow", &[&me]);
    std::fs::set_permissions(&allow, std::fs::Permissions::from_mode(0o644)).unwrap();
    let allow = allow.to_str().unwrap();
    let to = format!("{}@127.0.0.1", me.fingerprint());
    let runs: [&[&str]; 2] = [
        &[
            "link",
            "listen",
            "127.0.0.1",
            "--carrier",
            "icmp",
            "--allow-raw",
            "icmp",
            "--allow",
            allow,
        ],
        &[
            "link",
            "dial",
            &to,
            "--carrier",
            "icmp",
            "--allow-raw",
            "icmp",
            "--allow",
            allow,
            "--check",
        ],
    ];
    for args in runs {
        let Some(mut cmd) = without_raw(dir.path(), "n", args) else {
            test_host::skip_line("this host gives CAP_NET_RAW to a user that is not root");
            return;
        };
        let out = cmd.output().unwrap();
        assert_eq!(out.status.code(), Some(69), "{args:?}: {out:?}");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(err.contains("CAP_NET_RAW"), "{args:?}: {err}");
    }
    let mut cmd = without_raw(dir.path(), "n", &["link", "carriers", "--json"]).unwrap();
    let out = cmd.output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let row = doc["carriers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == "icmp")
        .unwrap()
        .clone();
    assert_eq!(row["available"], false, "{row}");
    assert!(
        row["reason"].as_str().unwrap().contains("CAP_NET_RAW"),
        "{row}"
    );
}

// ---- a raw ICMP socket on the loopback ----

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_link_crosses_icmp_and_each_end_learns_the_key_of_the_other() {
    need_raw_socket!();
    let _one = RAW.lock().await;
    link_peer::a_link_crosses(&case(), 64 * 1024).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_dialer_that_the_listener_does_not_allow_exits_77_and_the_door_stays_open() {
    need_raw_socket!();
    let _one = RAW.lock().await;
    link_peer::a_stranger_exits_77(&case()).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_listener_with_another_key_than_the_pinned_one_exits_77() {
    need_raw_socket!();
    let _one = RAW.lock().await;
    link_peer::another_key_exits_77(&case()).await;
}

#[test]
fn link_dial_check_meets_link_listen_over_icmp_and_a_stranger_exits_77() {
    need_raw_socket!();
    let _one = RAW.blocking_lock();
    let raw = ["--allow-raw", "icmp"];
    link_peer::cli_check("icmp", "127.0.0.1", &raw, address_of, &raw);
}
