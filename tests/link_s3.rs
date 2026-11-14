//! The object-storage carrier of the link (T-279): TLS 1.3 with the
//! Ed25519 key of each end pinned, over the objects of a bucket of the S3
//! API. The bucket is a stand-in on the loopback (`link_s3_standin`); no
//! test calls a real service. A test that needs the loopback prints
//! `skipping, REASON` on a host that denies it, and does not fail.
#![cfg(feature = "s3")]

#[path = "test_host/mod.rs"]
mod test_host;

#[path = "link_peer/mod.rs"]
mod link_peer;

#[path = "link_s3_standin/mod.rs"]
mod standin;

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::Duration;

use link_peer::{install, list_of, Case};
use pod_link::s3::{Credentials, S3};
use pod_link::{Carrier, Cost, Settings};
use podnet::exit::code_of;
use podnet::identity::{AllowList, Identity};
use podnet::link_keyed::Tune;
use podnet::link_s3;
use standin::StandIn;

const BUCKET: &str = "links";

fn address_of(key: &str, listen: &str) -> String {
    format!("{key}@{listen}")
}

/// A profile file with the key of the stand-in, in `dir`.
fn credentials_file(dir: &Path) -> PathBuf {
    let file = dir.join("credentials");
    std::fs::write(&file, standin::profile_file()).unwrap();
    file
}

/// The tune of the carrier on `standin`, with the key in a profile file.
fn tune(standin: &StandIn, dir: &Path) -> Tune {
    Tune {
        s3_endpoint: Some(standin.endpoint()),
        s3_credentials: Some(credentials_file(dir)),
        ..Tune::default()
    }
}

/// The carrier of `me` on `standin` with the key `key`, short polls and
/// small objects, so that a stream spans many.
fn carrier(me: &Identity, allow: &AllowList, standin: &StandIn, key: Credentials) -> S3 {
    let tune = Tune {
        s3_endpoint: Some(standin.endpoint()),
        ..Tune::default()
    };
    let mut s3 = link_s3::carrier_with(me, allow, &tune, key).unwrap();
    (s3.object, s3.poll_min, s3.poll_max) = (
        16 << 10,
        Duration::from_millis(10),
        Duration::from_millis(200),
    );
    s3
}

// ---- no socket ----

#[test]
fn the_registry_row_has_no_key_and_refuses_a_dial() {
    link_peer::the_registry_row_refuses_a_dial("s3", "s3", "x@links");
}

#[test]
fn the_carrier_is_a_stream_of_the_highest_overhead_and_takes_the_flags() {
    let dir = tempfile::tempdir().unwrap();
    let me = install(dir.path(), "me");
    let tune = Tune {
        s3_endpoint: Some("http://127.0.0.1:9/base".into()),
        s3_region: Some("eu-central-1".into()),
        s3_credentials: Some(credentials_file(dir.path())),
        s3_prune: true,
        ..Tune::default()
    };
    let s3 = link_s3::carrier(&me, &AllowList::default(), &tune).unwrap();
    assert_eq!(s3.name(), "s3");
    assert_eq!(s3.region, "eu-central-1");
    assert_eq!(s3.endpoint.base, "/base");
    assert_eq!(s3.credentials, standin::key());
    assert!(s3.prune);
    let facts = s3.facts();
    assert!(facts.reliable && facts.ordered && facts.duplex);
    assert_eq!(facts.cost, Cost::Highest);
}

#[test]
fn the_cli_refuses_no_endpoint_no_key_and_the_flags_for_another_carrier() {
    let dir = tempfile::tempdir().unwrap();
    let allow = link_peer::write_allow(dir.path(), "allow", &[]);
    let run = |args: &[&str]| {
        link_peer::podnet(dir.path(), "s", args)
            .env("HOME", dir.path())
            .env_remove(link_s3::ACCESS_KEY_VARIABLE)
            .env_remove(link_s3::SECRET_KEY_VARIABLE)
            .env_remove(link_s3::SESSION_TOKEN_VARIABLE)
            .env_remove(link_s3::CREDENTIALS_FILE_VARIABLE)
            .env_remove(link_s3::PROFILE_VARIABLE)
            .output()
            .unwrap()
    };
    let allow = allow.to_str().unwrap();
    let base = [
        "link",
        "listen",
        "links/x",
        "--carrier",
        "s3",
        "--allow",
        allow,
    ];
    let out = run(&base);
    assert_eq!(out.status.code(), Some(64), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stderr).contains("--s3-endpoint"));
    // No key in the environment, no profile file: exit 77, and no value.
    let mut args = base.to_vec();
    args.extend(["--s3-endpoint", "http://127.0.0.1:9"]);
    let out = run(&args);
    assert_eq!(out.status.code(), Some(77), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stderr).contains("no key of the bucket"));
    let out = run(&["link", "listen", "/tmp/x", "--s3-prune", "--allow", allow]);
    assert_eq!(out.status.code(), Some(64), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stderr).contains("s3 carrier"));
}

// ---- TCP loopback: the stand-in of the S3 API ----

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_link_crosses_s3_and_each_end_learns_the_key_of_the_other() {
    test_host::need_loopback!();
    let standin = StandIn::start(&[BUCKET]);
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (install(dir.path(), "a"), install(dir.path(), "b"));
    let far_carrier = carrier(&b, &list_of(&[&a]), &standin, standin::key());
    let near_carrier = carrier(&a, &list_of(&[&b]), &standin, standin::key());
    let mut door = link_s3::listen(&far_carrier, "links/t1").await.unwrap();
    assert_eq!(door.address(), "links/t1");
    let to = address_of(&b.fingerprint(), &door.address());
    let allow = list_of(&[&b]);
    let (dialed, accepted) = tokio::join!(
        link_s3::dial(&near_carrier, &allow, &to, Settings::default()),
        link_peer::quiet(&mut door)
    );
    let ((near, seen_by_a), (far, seen_by_b)) = (dialed.unwrap(), accepted.unwrap());
    assert_eq!(seen_by_a, b.public());
    assert_eq!(seen_by_b, Some(a.public()));
    let msg: Vec<u8> = (0..100_000).map(|i| (i % 251) as u8).collect();
    link_peer::echo(&near, &far, &msg).await;
    // No --s3-prune: the objects stay in the bucket.
    assert!(!standin.keys(BUCKET, "t1/up/").is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_glue_of_link_keyed_carries_a_link_and_refuses_strangers() {
    test_host::need_loopback!();
    let standin = StandIn::start(&[BUCKET]);
    let dir = tempfile::tempdir().unwrap();
    let case = Case {
        name: "s3",
        tune: tune(&standin, dir.path()),
        address_of,
    };
    let (a, b) = (install(dir.path(), "a"), install(dir.path(), "b"));
    let mut door = podnet::link_keyed::listen("s3", &case.tune, &b, &list_of(&[&a]), BUCKET)
        .await
        .unwrap();
    let to = address_of(&b.fingerprint(), &door.address());
    let (dialed, accepted) = case.meet(&a, list_of(&[&b]), &to, &mut door).await;
    let ((near, _), (far, seen_by_b)) = (dialed.unwrap(), accepted.unwrap());
    assert_eq!(seen_by_b, Some(a.public()));
    link_peer::echo(&near, &far, &[5u8; 4096]).await;
    // Another key than the pinned one exits 77 at the dialer.
    let c = install(dir.path(), "c");
    let wrong = address_of(&c.fingerprint(), &door.address());
    let e = case
        .dial(&a, &list_of(&[&b, &c]), &wrong)
        .await
        .err()
        .unwrap();
    assert_eq!(code_of(&e), 77, "{e:#}");
    // An address with no key exits 64.
    link_peer::no_key_exits_64(&case, BUCKET).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_key_that_the_bucket_refuses_exits_77_and_a_missing_bucket_69() {
    test_host::need_loopback!();
    let standin = StandIn::start(&[BUCKET]);
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (install(dir.path(), "a"), install(dir.path(), "b"));
    let wrong = Credentials::new("PODNETTESTKEY", "another-secret");
    let s3 = carrier(&b, &list_of(&[&a]), &standin, wrong.clone());
    let Err(e) = link_s3::listen(&s3, BUCKET).await else {
        panic!("the bucket refuses the key")
    };
    assert_eq!(code_of(&e), 77, "{e:#}");
    let s3 = carrier(&a, &list_of(&[&b]), &standin, wrong);
    let to = address_of(&b.fingerprint(), BUCKET);
    let allow = list_of(&[&b]);
    let Err(e) = link_s3::dial(&s3, &allow, &to, Settings::default()).await else {
        panic!("the bucket refuses the key")
    };
    assert_eq!(code_of(&e), 77, "{e:#}");
    assert!(standin.refused.load(Ordering::SeqCst) >= 2);
    let s3 = carrier(&b, &list_of(&[&a]), &standin, standin::key());
    let Err(e) = link_s3::listen(&s3, "nobucket").await else {
        panic!("no such bucket")
    };
    assert_eq!(code_of(&e), 69, "{e:#}");
}

#[test]
fn link_listen_and_dial_check_meet_over_s3() {
    test_host::need_loopback!();
    let standin = StandIn::start(&[BUCKET]);
    let dir = tempfile::tempdir().unwrap();
    let file = credentials_file(dir.path());
    let (endpoint, file) = (standin.endpoint(), file.to_str().unwrap().to_string());
    let flags = [
        "--s3-endpoint",
        endpoint.as_str(),
        "--s3-credentials",
        file.as_str(),
    ];
    link_peer::cli_check("s3", "links/cli", &flags, address_of, &flags);
}
