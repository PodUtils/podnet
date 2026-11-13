//! The record layer of the link (T-298, T-299, T-300, T-301) over the unix
//! carrier: a sealed stream echoes, a tampered, replayed or padded record,
//! a RESUME used once, the compression, and the middle hop of a chain that
//! sees no plaintext. Each test binds no port.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pod_e2e::record::{Header, Opened, Settings, Suite, HEADER};
use podnet::exit::code_of;
use podnet::identity::{self, AllowList, Identity};
use podnet::link::{self, Link, Settings as LinkSettings, Stream};
use podnet::link_record::{self as record, Options, Sealed, COMPRESSED};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

fn install(dir: &Path, name: &str) -> Identity {
    let sub = dir.join(name);
    std::fs::create_dir(&sub).unwrap();
    identity::create(&sub).unwrap()
}

fn allow(keys: &[&Identity]) -> AllowList {
    let text: Vec<String> = keys.iter().map(|k| k.fingerprint()).collect();
    AllowList::parse(&text.join("\n")).unwrap()
}

/// A stream that the opener made and the acceptor got, over a unix link.
async fn streams() -> (Stream, Stream, (Link, Link)) {
    let (a, b) = link::unix_pair(LinkSettings::default()).unwrap();
    let opened = a.open().unwrap();
    let mut got = b.accept().await.unwrap();
    // The opener's first bytes open the stream at the acceptor.
    let mut opened = opened;
    opened.write_all(b"x").await.unwrap();
    let mut one = [0u8; 1];
    got.read_exact(&mut one).await.unwrap();
    (opened, got, (a, b))
}

async fn pair(
    dialer: Options,
    listener: Options,
) -> (Result<Sealed, anyhow::Error>, Result<Sealed, anyhow::Error>) {
    let dir = tempfile::tempdir().unwrap();
    let (a, c) = (install(dir.path(), "a"), install(dir.path(), "c"));
    let (s, t, _links) = streams().await;
    let (cp, list) = (c.public(), allow(&[&a]));
    let (d, l) = tokio::join!(
        record::dial(s, &a, Some(&cp), dialer),
        record::accept(t, &c, &list, listener)
    );
    std::mem::forget(_links);
    (d, l)
}

async fn echo(mut s: Sealed) {
    let mut buf = vec![0u8; 8192];
    loop {
        let n = s.read(&mut buf).await.unwrap_or(0);
        if n == 0 {
            break;
        }
        s.write_all(&buf[..n]).await.unwrap();
    }
    let _ = s.shutdown().await;
}

async fn round(mut s: Sealed, data: Vec<u8>) -> (Vec<u8>, Sealed) {
    let (mut r, mut w) = tokio::io::split(&mut s);
    let send = async {
        w.write_all(&data).await.unwrap();
        w.shutdown().await.unwrap();
    };
    let mut back = Vec::new();
    let (_, got) = tokio::join!(send, r.read_to_end(&mut back));
    got.unwrap();
    (back, s)
}

fn noise(n: usize) -> Vec<u8> {
    let mut x = 0x2545_f491_4f6c_dd1du64;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

// ---- the relay of a chain: it reads whole records and may change them ----

type Edit = Box<dyn FnMut(usize, Vec<u8>) -> Vec<Vec<u8>> + Send>;

#[derive(Default, Clone)]
struct Seen {
    /// The records A sent, as the middle hop saw them.
    to_c: Arc<Mutex<Vec<Vec<u8>>>>,
    to_a: Arc<Mutex<Vec<Vec<u8>>>>,
}

async fn skip_handshake<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(r: &mut R, w: &mut W) {
    let mut magic = [0u8; 13];
    r.read_exact(&mut magic).await.unwrap();
    w.write_all(&magic).await.unwrap();
    // The Noise message, the proof or the verdict, and the suites.
    for _ in 0..3 {
        let mut len = [0u8; 2];
        r.read_exact(&mut len).await.unwrap();
        let mut body = vec![0u8; u16::from_be_bytes(len) as usize];
        r.read_exact(&mut body).await.unwrap();
        w.write_all(&len).await.unwrap();
        w.write_all(&body).await.unwrap();
    }
}

async fn relay_one<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    mut r: R,
    mut w: W,
    seen: Arc<Mutex<Vec<Vec<u8>>>>,
    mut edit: Edit,
) {
    skip_handshake(&mut r, &mut w).await;
    let mut index = 0;
    loop {
        let mut rec = vec![0u8; HEADER];
        if r.read_exact(&mut rec).await.is_err() {
            break;
        }
        rec.resize(pod_e2e::record::record_len(&rec).unwrap(), 0);
        if r.read_exact(&mut rec[HEADER..]).await.is_err() {
            break;
        }
        seen.lock().unwrap().push(rec.clone());
        for out in edit(index, rec) {
            if w.write_all(&out).await.is_err() {
                return;
            }
        }
        index += 1;
    }
    let _ = w.shutdown().await;
}

/// A chain A - B - C of two unix links. B copies the records of one stream
/// and applies `to_c` to those of A and `to_a` to those of C.
async fn chain(opts_a: Options, opts_c: Options, to_c: Edit, to_a: Edit) -> (Sealed, Sealed, Seen) {
    let dir = tempfile::tempdir().unwrap();
    let (a, c) = (install(dir.path(), "a"), install(dir.path(), "c"));
    let (ab_a, ab_b) = link::unix_pair(LinkSettings::default()).unwrap();
    let (bc_b, bc_c) = link::unix_pair(LinkSettings::default()).unwrap();
    let mut sa = ab_a.open().unwrap();
    sa.write_all(b"x").await.unwrap();
    let mut sb_in = ab_b.accept().await.unwrap();
    let mut one = [0u8; 1];
    sb_in.read_exact(&mut one).await.unwrap();
    let mut sb_out = bc_b.open().unwrap();
    sb_out.write_all(b"x").await.unwrap();
    let mut sc = bc_c.accept().await.unwrap();
    sc.read_exact(&mut one).await.unwrap();
    let seen = Seen::default();
    let (in_r, in_w) = tokio::io::split(sb_in);
    let (out_r, out_w) = tokio::io::split(sb_out);
    tokio::spawn(relay_one(in_r, out_w, seen.to_c.clone(), to_c));
    tokio::spawn(relay_one(out_r, in_w, seen.to_a.clone(), to_a));
    let (cp, list) = (c.public(), allow(&[&a]));
    let (d, l) = tokio::join!(
        record::dial(sa, &a, Some(&cp), opts_a),
        record::accept(sc, &c, &list, opts_c)
    );
    std::mem::forget((ab_a, ab_b, bc_b, bc_c));
    (d.unwrap(), l.unwrap(), seen)
}

fn pass() -> Edit {
    Box::new(|_, r| vec![r])
}

fn header(record: &[u8]) -> Header {
    Header::parse(record).unwrap()
}

// ---- T-298 ----

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

// ---- T-301 ----

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

// ---- T-300 ----

#[tokio::test]
async fn padding_gives_sizes_from_the_buckets() {
    let opts = Options {
        pad: true,
        ..Options::default()
    };
    let (mut d, mut l, seen) = chain(opts, Options::default(), pass(), pass()).await;
    for size in [1usize, 100, 128, 200, 600, 1500, 3000] {
        d.write_all(&vec![7u8; size]).await.unwrap();
        let mut got = vec![0u8; size];
        l.read_exact(&mut got).await.unwrap();
        assert_eq!(got, vec![7u8; size], "padding is removed before the reader");
    }
    let lengths: Vec<u32> = seen
        .to_c
        .lock()
        .unwrap()
        .iter()
        .map(|r| header(r).length)
        .collect();
    assert_eq!(lengths, vec![128, 128, 128, 512, 1024, 2048, 4096]);
}

#[tokio::test]
async fn a_tampered_pad_fails_authentication() {
    let pad: Edit = Box::new(|i, mut r| {
        if i == 0 {
            // `pad` is bytes 20 and 21 of the header: say one byte less.
            let p = u16::from_be_bytes([r[20], r[21]]) - 1;
            r[20..22].copy_from_slice(&p.to_be_bytes());
        }
        vec![r]
    });
    let opts = Options {
        pad: true,
        ..Options::default()
    };
    let (mut d, mut l, _) = chain(opts, Options::default(), pad, pass()).await;
    d.write_all(b"hello").await.unwrap();
    let err = l.read(&mut [0u8; 16]).await.unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData, "{err}");
    assert!(err.to_string().contains("check"), "{err}");
}

// ---- T-299 ----

#[tokio::test]
async fn compression_shrinks_a_compressible_payload_and_the_data_stays_equal() {
    let data = b"podnet link record ".repeat(20_000);
    let on = Options {
        compress: true,
        ..Options::default()
    };
    let (d, l, seen) = chain(on, Options::default(), pass(), pass()).await;
    tokio::spawn(echo(l));
    let (back, d) = round(d, data.clone()).await;
    assert_eq!(back, data);
    assert!(
        d.stats().wire_out() < data.len() as u64 / 4,
        "{} bytes for {}",
        d.stats().wire_out(),
        data.len()
    );
    let flagged = |v: &Mutex<Vec<Vec<u8>>>| {
        v.lock()
            .unwrap()
            .iter()
            .filter(|r| header(r).flags & COMPRESSED != 0)
            .count()
    };
    assert!(flagged(&seen.to_c) > 0);
    assert_eq!(flagged(&seen.to_a), 0, "the echo end has compression off");
    // The same data with compression off is not smaller than the data.
    let (d, l) = pair(Options::default(), Options::default()).await;
    let (d, l) = (d.unwrap(), l.unwrap());
    tokio::spawn(echo(l));
    let (back, d) = round(d, data.clone()).await;
    assert_eq!(back, data);
    assert!(d.stats().wire_out() > data.len() as u64);
}

#[tokio::test]
async fn a_payload_that_does_not_shrink_or_is_short_goes_with_no_flag() {
    let on = Options {
        compress: true,
        ..Options::default()
    };
    let (mut d, mut l, seen) = chain(on, Options::default(), pass(), pass()).await;
    for data in [noise(4000), vec![b'a'; 255]] {
        d.write_all(&data).await.unwrap();
        let mut got = vec![0u8; data.len()];
        l.read_exact(&mut got).await.unwrap();
        assert_eq!(got, data);
    }
    assert!(seen
        .to_c
        .lock()
        .unwrap()
        .iter()
        .all(|r| header(r).flags & COMPRESSED == 0));
}

#[tokio::test]
async fn a_compressed_record_over_the_cap_closes_the_stream_with_decompress_limit() {
    let (a, b) = tokio::io::duplex(64 * 1024);
    let dir = tempfile::tempdir().unwrap();
    let (ia, ib) = (install(dir.path(), "a"), install(dir.path(), "b"));
    let evil = tokio::spawn(async move {
        let mut ch = pod_e2e::initiate(a, ia.key())
            .await
            .unwrap()
            .accept(None)
            .await
            .unwrap();
        ch.agree(&Suite::ALL).await.unwrap();
        let mut records = ch
            .exporter()
            .unwrap()
            .records(Suite::ChaCha20Poly1305, Settings::DEFAULT, Instant::now())
            .unwrap();
        let mut io = ch.into_transport();
        // The preamble says 9 MiB: over the cap of 8 MiB.
        let mut payload = Vec::new();
        let mut n = 9 * 1024 * 1024usize;
        while n >= 0x80 {
            payload.push((n & 0x7f) as u8 | 0x80);
            n >>= 7;
        }
        payload.push(n as u8);
        payload.resize(2000, 0);
        let rec = records
            .seal(Instant::now(), record::DATA, COMPRESSED, 0, &payload)
            .unwrap();
        io.write_all(&rec).await.unwrap();
        io
    });
    let asked = pod_e2e::respond(b, ib.key()).await.unwrap();
    let peer = asked.peer();
    let channel = asked.admit().await.unwrap();
    let mut l = record::from_channel(channel, peer, Options::default())
        .await
        .unwrap();
    let _io = evil.await.unwrap();
    let err = l.read(&mut [0u8; 16]).await.unwrap_err();
    assert!(err.to_string().contains("decompress-limit"), "{err}");
}
