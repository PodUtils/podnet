//! The replay window and the RESUME record (T-301) over the unix carrier:
//! a replayed record is counted and dropped, a record below the window is
//! dropped, a RESUME makes the peer send again what was lost, and a RESUME
//! is used once. A sealed stream moves to a new carrier with no lost byte,
//! and the end of the resume window is `ResumeExpired` (exit 69).

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

// ---- the RESUME record on a new carrier, and its end (T-301) ----

const MIB: usize = 1 << 20;

fn sha256(bytes: &[u8]) -> Vec<u8> {
    ring::digest::digest(&ring::digest::SHA256, bytes)
        .as_ref()
        .to_vec()
}

/// Copy until `left` bytes (both ways together) have passed.
async fn copy_until<R, W>(from: &mut R, to: &mut W, left: &std::sync::atomic::AtomicI64)
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    let mut buf = vec![0u8; 16 * 1024];
    loop {
        let n = match from.read(&mut buf).await {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        let before = left.fetch_sub(n as i64, Ordering::SeqCst);
        if before <= n as i64 {
            let _ = to.write_all(&buf[..before.max(0) as usize]).await;
            return;
        }
        if to.write_all(&buf[..n]).await.is_err() {
            return;
        }
    }
}

/// A carrier through a stand-in host that drops both ends, with no word,
/// after `cut` bytes (`None`: never). The receiver hears the cut.
fn carrier(
    cut: Option<usize>,
) -> (
    tokio::io::DuplexStream,
    tokio::io::DuplexStream,
    tokio::sync::oneshot::Receiver<()>,
) {
    let (near, relay_near) = tokio::io::duplex(64 * 1024);
    let (relay_far, far) = tokio::io::duplex(64 * 1024);
    let (done, cut_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let left = std::sync::atomic::AtomicI64::new(cut.map_or(i64::MAX, |c| c as i64));
        let (mut nr, mut nw) = tokio::io::split(relay_near);
        let (mut fr, mut fw) = tokio::io::split(relay_far);
        tokio::select! {
            _ = copy_until(&mut nr, &mut fw, &left) => {}
            _ = copy_until(&mut fr, &mut nw, &left) => {}
        }
        drop((nr, nw, fr, fw));
        let _ = done.send(());
    });
    (near, far, cut_rx)
}

/// Two sealed ends over `carrier(cut)`, each with `options`.
async fn sealed_over(
    cut: Option<usize>,
    options: Options,
) -> (
    podnet::link_record::Sealed,
    podnet::link_record::Sealed,
    tokio::sync::oneshot::Receiver<()>,
) {
    let dir = tempfile::tempdir().unwrap();
    let (a, c) = (install(dir.path(), "a"), install(dir.path(), "c"));
    let (near, far, cut) = carrier(cut);
    let (cp, list) = (c.public(), allow(&[&a]));
    let (d, l) = tokio::join!(
        podnet::link_record::dial(near, &a, Some(&cp), options.clone()),
        podnet::link_record::accept(far, &c, &list, options)
    );
    (d.unwrap(), l.unwrap(), cut)
}

/// Write `data` and read to the end, on one sealed stream.
async fn both_ways(s: &mut podnet::link_record::Sealed, data: Vec<u8>) -> Vec<u8> {
    let (mut r, mut w) = tokio::io::split(s);
    let send = async {
        w.write_all(&data).await.unwrap();
        w.shutdown().await.unwrap();
    };
    let mut got = Vec::new();
    let (_, read) = tokio::join!(send, r.read_to_end(&mut got));
    read.unwrap();
    got
}

#[test]
fn the_record_layer_and_the_session_share_one_window() {
    let window = podnet::session::RESUME_WINDOW;
    assert_eq!(window, Duration::from_secs(600));
    assert_eq!(Options::default().records.resume_window, window);
    assert_eq!(podnet::link_record::SETTINGS.resume_window, window);
    assert_eq!(podnet::session::settings(window).resume_deadline, window);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_sealed_stream_moves_to_a_new_carrier_with_no_lost_byte() {
    let options = Options {
        carry_over: true,
        ..Options::default()
    };
    // The first carrier is cut after 300 kB, in the middle of both
    // directions.
    let (mut a, mut c, cut) = sealed_over(Some(300_000), options).await;
    let (to_c, to_a) = (noise(MIB), noise(MIB + 17)[17..].to_vec());
    let (move_a, move_c) = (a.rebinder(), c.rebinder());
    let rebind = async move {
        cut.await.unwrap();
        let (near, far, _) = carrier(None);
        move_a.rebind(near).unwrap();
        move_c.rebind(far).unwrap();
    };
    let run = async {
        tokio::join!(
            both_ways(&mut a, to_c.clone()),
            both_ways(&mut c, to_a.clone()),
            rebind
        )
    };
    let (at_a, at_c, ()) = tokio::time::timeout(Duration::from_secs(60), run)
        .await
        .expect("the bytes came through");
    assert_eq!(at_c.len(), MIB);
    assert_eq!(at_a.len(), MIB);
    assert_eq!(sha256(&at_c), sha256(&to_c), "A to C differs");
    assert_eq!(sha256(&at_a), sha256(&to_a), "C to A differs");
    // Each end refused no RESUME, and no record came twice.
    for end in [&a, &c] {
        assert_eq!(end.stats().refused(), 0);
        assert_eq!(end.stats().replayed(), 0);
    }
}

#[tokio::test]
async fn three_failed_resume_records_end_the_stream_with_exit_69() {
    // A -> C: the relay changes each RESUME record of A.
    let spoil: Edit = Box::new(|_, mut r| {
        if header(&r).kind == 0xff {
            let at = r.len() - 1;
            r[at] ^= 1;
        }
        vec![r]
    });
    let (mut a, mut c, _) = chain(Options::default(), Options::default(), spoil, pass()).await;
    a.write_all(b"one").await.unwrap();
    let mut got = [0u8; 3];
    c.read_exact(&mut got).await.unwrap();
    for _ in 0..3 {
        a.resume().unwrap();
    }
    let mut rest = Vec::new();
    let e = tokio::time::timeout(Duration::from_secs(30), c.read_to_end(&mut rest))
        .await
        .expect("the stream ends")
        .unwrap_err();
    assert_eq!(c.stats().refused(), 3, "{e}");
    let error = podnet::link_record::ended(e);
    assert_eq!(podnet::exit::code_of(&error), podnet::exit::UNAVAILABLE);
    let expired = error
        .chain()
        .find_map(|e| e.downcast_ref::<podnet::session::ResumeExpired>())
        .unwrap_or_else(|| panic!("not ResumeExpired: {error:#}"));
    assert_eq!(expired.window, podnet::session::RESUME_WINDOW);
}

#[tokio::test]
async fn no_new_carrier_in_the_window_ends_with_resume_expired() {
    let options = Options {
        carry_over: true,
        records: Settings {
            resume_window: Duration::from_secs(1),
            ..podnet::link_record::SETTINGS
        },
        ..Options::default()
    };
    let (mut a, _c, cut) = sealed_over(Some(20_000), options).await;
    let (_, mut w) = tokio::io::split(&mut a);
    let _ = w.write_all(&noise(100_000)).await;
    drop(w);
    cut.await.unwrap();
    let mut rest = Vec::new();
    let e = tokio::time::timeout(Duration::from_secs(30), a.read_to_end(&mut rest))
        .await
        .expect("the window ends")
        .unwrap_err();
    let error = podnet::link_record::ended(e);
    assert_eq!(podnet::exit::code_of(&error), podnet::exit::UNAVAILABLE);
    let expired = error
        .chain()
        .find_map(|e| e.downcast_ref::<podnet::session::ResumeExpired>())
        .unwrap_or_else(|| panic!("not ResumeExpired: {error:#}"));
    assert_eq!(expired.window, Duration::from_secs(1));
}
