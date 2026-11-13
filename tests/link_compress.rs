//! The snappy compression of records (T-299) over the unix carrier: a
//! compressible payload shrinks and stays equal, a short payload or one that
//! does not shrink goes with no flag, and a record over the cap ends the
//! stream with `decompress-limit`.

mod link_record_harness;

use std::sync::Mutex;
use std::time::Instant;

use pod_e2e::record::{Settings, Suite};
use podnet::link_record::{self as record, Options, COMPRESSED};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use link_record_harness::*;

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
