//! The record layer of the link (T-298) over the unix carrier: a sealed
//! stream echoes with each suite, a suite mismatch exits 78, a key that is
//! not expected exits 77, a tampered record or header is refused, a cut is
//! not an end, and the key changes at its limit. Each test binds no port.

mod link_record_harness;

use pod_e2e::record::{Settings, Suite};
use podnet::exit::code_of;
use podnet::link_record::{self as record, Options};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use link_record_harness::*;

#[tokio::test]
async fn a_sealed_stream_echoes() {
    let (d, l) = pair(Options::default(), Options::default()).await;
    let (d, l) = (d.unwrap(), l.unwrap());
    assert_eq!(d.suite(), Suite::ChaCha20Poly1305, "the default suite");
    assert_eq!(d.suite(), l.suite());
    tokio::spawn(echo(l));
    let data = noise(300_000);
    let (back, d) = round(d, data.clone()).await;
    assert_eq!(back, data);
    assert!(d.stats().sent() > 9 && d.stats().received() > 9);
    assert_eq!(d.stats().replayed(), 0);
}

#[tokio::test]
async fn each_suite_carries_the_stream() {
    for suite in Suite::ALL {
        let opts = Options {
            suites: vec![suite],
            ..Options::default()
        };
        let (d, l) = pair(opts.clone(), opts).await;
        let (d, l) = (d.unwrap(), l.unwrap());
        assert_eq!(d.suite(), suite);
        tokio::spawn(echo(l));
        let (back, _) = round(d, b"suite".repeat(5000)).await;
        assert_eq!(back, b"suite".repeat(5000), "{suite}");
    }
}

#[tokio::test]
async fn a_suite_mismatch_exits_78() {
    let aes = Options {
        suites: vec![Suite::Aes256Gcm],
        ..Options::default()
    };
    let chacha = Options {
        suites: vec![Suite::ChaCha20Poly1305],
        ..Options::default()
    };
    let (d, l) = pair(aes, chacha).await;
    assert_eq!(code_of(&d.err().unwrap()), 78);
    assert_eq!(code_of(&l.err().unwrap()), 78);
}

#[tokio::test]
async fn a_peer_that_is_not_the_expected_key_exits_77() {
    let dir = tempfile::tempdir().unwrap();
    let (a, c, x) = (
        install(dir.path(), "a"),
        install(dir.path(), "c"),
        install(dir.path(), "x"),
    );
    let (s, t, _links) = streams().await;
    let (xp, list) = (x.public(), allow(&[&a]));
    let (d, l) = tokio::join!(
        record::dial(s, &a, Some(&xp), Options::default()),
        record::accept(t, &c, &list, Options::default())
    );
    assert_eq!(code_of(&d.err().unwrap()), 77);
    assert!(l.is_err());
    let (s, t, _links2) = streams().await;
    let list = allow(&[&x]);
    let (d, l) = tokio::join!(
        record::dial(s, &a, None, Options::default()),
        record::accept(t, &c, &list, Options::default())
    );
    assert_eq!(code_of(&l.err().unwrap()), 77, "a key that the list lacks");
    assert!(d.is_err());
}

#[tokio::test]
async fn a_tampered_record_is_refused() {
    let flip: Edit = Box::new(|i, mut r| {
        if i == 1 {
            let n = r.len();
            r[n - 20] ^= 1;
        }
        vec![r]
    });
    let (mut d, mut l, _) = chain(Options::default(), Options::default(), flip, pass()).await;
    d.write_all(b"first").await.unwrap();
    let mut got = [0u8; 5];
    l.read_exact(&mut got).await.unwrap();
    assert_eq!(&got, b"first");
    d.write_all(b"second").await.unwrap();
    let err = l.read(&mut [0u8; 16]).await.unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData, "{err}");
    assert!(err.to_string().contains("check"), "{err}");
}

#[tokio::test]
async fn a_changed_header_is_refused() {
    let flip: Edit = Box::new(|i, mut r| {
        if i == 0 {
            r[5] ^= 1; // the stream number
        }
        vec![r]
    });
    let (mut d, mut l, _) = chain(Options::default(), Options::default(), flip, pass()).await;
    d.write_all(b"first").await.unwrap();
    let err = l.read(&mut [0u8; 16]).await.unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData, "{err}");
}

#[tokio::test]
async fn a_cut_stream_is_not_a_clean_end() {
    let cut: Edit = Box::new(|i, r| if i >= 1 { vec![] } else { vec![r] });
    let (mut d, mut l, _) = chain(Options::default(), Options::default(), cut, pass()).await;
    d.write_all(b"one").await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    d.shutdown().await.unwrap();
    let mut out = Vec::new();
    // The relay drops the FIN record: the end is a cut, not an end.
    let err = l.read_to_end(&mut out).await.unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::UnexpectedEof, "{err}");
    assert_eq!(out, b"one");
}

#[tokio::test]
async fn the_key_changes_at_the_limit_and_the_stream_goes_on() {
    let opts = Options {
        records: Settings {
            rekey_records: 3,
            ..Settings::DEFAULT
        },
        ..Options::default()
    };
    let (mut d, mut l, seen) = chain(opts.clone(), opts, pass(), pass()).await;
    for i in 0..10u8 {
        d.write_all(&[i; 100]).await.unwrap();
        let mut got = [0u8; 100];
        l.read_exact(&mut got).await.unwrap();
        assert_eq!(got, [i; 100]);
    }
    assert!(d.epochs().0 >= 3, "{:?}", d.epochs());
    assert_eq!(l.epochs().1, d.epochs().0);
    let phases: Vec<u16> = seen
        .to_c
        .lock()
        .unwrap()
        .iter()
        .map(|r| header(r).flags & 1)
        .collect();
    assert!(phases.contains(&0) && phases.contains(&1), "{phases:?}");
}
