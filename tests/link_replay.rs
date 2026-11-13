//! The replay window and the RESUME record (T-301) over the unix carrier:
//! a replayed record is counted and dropped, a record below the window is
//! dropped, a RESUME makes the peer send again what was lost, and a RESUME
//! is used once.

mod link_record_harness;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pod_e2e::record::{Opened, Settings, Suite};
use podnet::link_record::Options;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use link_record_harness::*;

#[tokio::test]
async fn a_replayed_record_is_refused() {
    let twice: Edit = Box::new(|i, r| if i == 1 { vec![r.clone(), r] } else { vec![r] });
    let (mut d, mut l, _) = chain(Options::default(), Options::default(), twice, pass()).await;
    for word in [&b"one"[..], b"two", b"three"] {
        d.write_all(word).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    d.shutdown().await.unwrap();
    let mut out = Vec::new();
    l.read_to_end(&mut out).await.unwrap();
    assert_eq!(out, b"onetwothree", "the repeat adds no bytes");
    assert_eq!(l.stats().replayed(), 1);
}

#[tokio::test]
async fn a_record_below_the_window_is_dropped_and_counted() {
    // The relay keeps the first record and sends it again after 1100 more.
    let held = Arc::new(Mutex::new(None::<Vec<u8>>));
    let h = held.clone();
    let late: Edit = Box::new(move |i, r| {
        if i == 0 {
            *h.lock().unwrap() = Some(r.clone());
        }
        if i == 1100 {
            return vec![r, h.lock().unwrap().clone().unwrap()];
        }
        vec![r]
    });
    let (mut d, mut l, _) = chain(Options::default(), Options::default(), late, pass()).await;
    let reader = tokio::spawn(async move {
        let mut out = Vec::new();
        l.read_to_end(&mut out).await.unwrap();
        (out.len(), l.stats().replayed())
    });
    for _ in 0..1101 {
        d.write_all(b"z").await.unwrap();
        d.flush().await.unwrap();
        tokio::task::yield_now().await;
    }
    d.shutdown().await.unwrap();
    let (n, replayed) = reader.await.unwrap();
    assert!(n <= 1101 && n > 0);
    assert_eq!(replayed, 1);
}

#[tokio::test]
async fn resume_sends_again_what_the_peer_did_not_get_and_is_used_once() {
    // C -> A loses its records after the first, until `lose` is cleared.
    let lose = Arc::new(AtomicBool::new(true));
    let l2 = lose.clone();
    let to_a: Edit = Box::new(move |i, r| {
        if i >= 1 && l2.load(Ordering::SeqCst) {
            vec![]
        } else {
            vec![r]
        }
    });
    // A -> C: the RESUME record (type 0xff) comes twice.
    let to_c: Edit = Box::new(|_, r| {
        if header(&r).kind == 0xff {
            vec![r.clone(), r]
        } else {
            vec![r]
        }
    });
    let (mut a, mut c, seen) = chain(Options::default(), Options::default(), to_c, to_a).await;
    let (done, refused) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let mut buf = [0u8; 64];
        loop {
            match c.read(&mut buf).await {
                Ok(n) if n > 0 => c.write_all(&buf[..n]).await.unwrap(),
                _ => break,
            }
        }
        let _ = done.send(c.stats().refused());
    });
    let mut got = [0u8; 3];
    a.write_all(b"one").await.unwrap();
    a.read_exact(&mut got).await.unwrap();
    assert_eq!(&got, b"one");
    a.write_all(b"two").await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    lose.store(false, Ordering::SeqCst);
    a.resume().unwrap();
    a.read_exact(&mut got).await.unwrap();
    assert_eq!(&got, b"two", "sent again after the RESUME");
    // The relay sent the RESUME twice: A sent one, and C refused the copy.
    let sent = seen.to_c.lock().unwrap().clone();
    assert_eq!(sent.iter().filter(|r| header(r).kind == 0xff).count(), 1);
    a.shutdown().await.unwrap();
    assert_eq!(refused.await.unwrap(), 1, "a RESUME is used once");
}

#[tokio::test]
async fn a_resume_that_comes_twice_is_refused_the_second_time() {
    // Records level, over a real handshake: the same RESUME opens once.
    let (a, b) = tokio::io::duplex(64 * 1024);
    let dir = tempfile::tempdir().unwrap();
    let (ia, ib) = (install(dir.path(), "a"), install(dir.path(), "b"));
    let init = tokio::spawn(async move {
        let mut ch = pod_e2e::initiate(a, ia.key())
            .await
            .unwrap()
            .accept(None)
            .await
            .unwrap();
        ch.agree(&Suite::ALL).await.unwrap();
        ch.exporter()
            .unwrap()
            .records(Suite::ChaCha20Poly1305, Settings::DEFAULT, Instant::now())
            .unwrap()
    });
    let mut ch = pod_e2e::respond(b, ib.key())
        .await
        .unwrap()
        .admit()
        .await
        .unwrap();
    ch.agree(&Suite::ALL).await.unwrap();
    let mut theirs = ch
        .exporter()
        .unwrap()
        .records(Suite::ChaCha20Poly1305, Settings::DEFAULT, Instant::now())
        .unwrap();
    let mut mine = init.await.unwrap();
    let resume = mine.resume(&mut pod_core::entropy::OsEntropy).unwrap();
    let now = Instant::now();
    assert!(matches!(
        theirs.open(now, &resume),
        Ok(Opened::Resume { next: 0 })
    ));
    assert!(theirs.open(now, &resume).is_err(), "the token is used once");
}
