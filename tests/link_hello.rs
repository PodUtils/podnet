//! The hello of a link (T-265): each end proves its key before the link
//! offers a service. The allow list of each end is default deny, and a peer
//! that it does not hold exits 77. The tests bind no port.

use std::path::Path;

use pod_identity::PublicKey;
use podnet::exit::code_of;
use podnet::hello;
use podnet::identity::{self, AllowList, Identity};
use podnet::link::{self, Link, Settings};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn install(dir: &Path, name: &str) -> Identity {
    let sub = dir.join(name);
    std::fs::create_dir(&sub).unwrap();
    identity::create(&sub).unwrap()
}

fn list_of(keys: &[&Identity]) -> AllowList {
    let text: Vec<String> = keys.iter().map(|k| k.fingerprint()).collect();
    AllowList::parse(&text.join("\n")).unwrap()
}

fn pair() -> (Link, Link) {
    link::memory_pair(1 << 20, Settings::default()).unwrap()
}

#[tokio::test]
async fn two_installs_that_allow_each_other_meet() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (install(dir.path(), "a"), install(dir.path(), "b"));
    let (dialer, listener) = pair();
    let (d, l) = tokio::join!(
        hello::dial(&dialer, &a, list_of(&[&b])),
        hello::accept(&listener, &b, list_of(&[&a]))
    );
    assert_eq!(d.unwrap(), b.public());
    assert_eq!(l.unwrap(), a.public());
}

#[tokio::test]
async fn a_dialer_that_the_listener_does_not_allow_exits_77_at_both_ends() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b, c) = (
        install(dir.path(), "a"),
        install(dir.path(), "b"),
        install(dir.path(), "c"),
    );
    let (dialer, listener) = pair();
    let (d, l) = tokio::join!(
        hello::dial(&dialer, &a, list_of(&[&b])),
        hello::accept(&listener, &b, list_of(&[&c]))
    );
    let (d, l) = (d.unwrap_err(), l.unwrap_err());
    assert_eq!(code_of(&l), 77, "{l:#}");
    assert!(format!("{l:#}").contains(&a.fingerprint()), "{l:#}");
    assert_eq!(code_of(&d), 77, "{d:#}");
}

#[tokio::test]
async fn an_empty_allow_list_refuses_each_peer() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (install(dir.path(), "a"), install(dir.path(), "b"));
    let (dialer, listener) = pair();
    let (d, l) = tokio::join!(
        hello::dial(&dialer, &a, list_of(&[&b])),
        hello::accept(&listener, &b, AllowList::default())
    );
    assert_eq!(code_of(&l.unwrap_err()), 77);
    assert_eq!(code_of(&d.unwrap_err()), 77);
}

#[tokio::test]
async fn a_listener_that_the_dialer_does_not_allow_exits_77() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b, c) = (
        install(dir.path(), "a"),
        install(dir.path(), "b"),
        install(dir.path(), "c"),
    );
    let (dialer, listener) = pair();
    let (d, _l) = tokio::join!(
        hello::dial(&dialer, &a, list_of(&[&c])),
        hello::accept(&listener, &b, list_of(&[&a]))
    );
    let d = d.unwrap_err();
    assert_eq!(code_of(&d), 77, "{d:#}");
    assert!(format!("{d:#}").contains(&b.fingerprint()), "{d:#}");
}

/// A dialer that announces the key of an allowed install, and signs with
/// nothing, gets no access.
#[tokio::test]
async fn a_dialer_that_cannot_sign_with_the_key_that_it_names_exits_77() {
    let dir = tempfile::tempdir().unwrap();
    let (victim, b) = (install(dir.path(), "a"), install(dir.path(), "b"));
    let (dialer, listener) = pair();
    let forger = async {
        let mut s = dialer.open().unwrap();
        let PublicKey(key) = victim.public();
        let mut hello = b"PNLINK01".to_vec();
        hello.extend_from_slice(&key);
        hello.extend_from_slice(&[7u8; 32]);
        s.write_all(&hello).await.unwrap();
        let mut reply = [0u8; 128];
        s.read_exact(&mut reply).await.unwrap();
        s.write_all(&[0u8; 64]).await.unwrap();
        let mut verdict = [9u8; 1];
        s.read_exact(&mut verdict).await.unwrap();
        verdict[0]
    };
    let (verdict, l) = tokio::join!(forger, hello::accept(&listener, &b, list_of(&[&victim])));
    assert_eq!(verdict, 1);
    let l = l.unwrap_err();
    assert_eq!(code_of(&l), 77, "{l:#}");
    assert!(format!("{l:#}").contains("signature"), "{l:#}");
}

#[tokio::test]
async fn the_hello_shows_no_seed() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (install(dir.path(), "a"), install(dir.path(), "b"));
    let seed = std::fs::read_to_string(a.path())
        .unwrap()
        .trim()
        .to_string();
    let (dialer, listener) = pair();
    let (d, _l) = tokio::join!(
        hello::dial(&dialer, &a, AllowList::default()),
        hello::accept(&listener, &b, list_of(&[&a]))
    );
    let text = format!("{:#} {:?}", d.unwrap_err(), a);
    assert!(!text.contains(&seed), "{text}");
}
