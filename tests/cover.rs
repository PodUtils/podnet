//! The cover listener (T-369, GitHub #491): a real TLS handshake, the switch
//! on the first application bytes, the fall-through to the site, and the
//! sizes of the cover script. Every connection is on the loopback.

mod test_host;

mod cover_harness;

use std::path::{Path, PathBuf};
use std::time::Duration;

use cover_harness::*;
use pod_cover::{Lane, Script, Switch};
use podnet::cover::{self, Config, Cover};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn an_unauthenticated_client_gets_the_real_page_twice() {
    test_host::need_loopback!();
    let fixture = fixture(false).await;
    let ca = ca();
    let addr = serve(&fixture, &ca).await;
    let mut tls = connect(addr, &ca).await;
    tls.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .await
        .unwrap();
    let (head, body) = response(&mut tls).await;
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    assert_eq!(body, PAGE.as_bytes());
    // COVER-3: no response names this project, its version or the contract.
    let served = format!(
        "{}{}",
        head.to_ascii_lowercase(),
        String::from_utf8_lossy(&body).to_ascii_lowercase()
    );
    for word in ["podutils", "podnet", "0.4.0", "contract"] {
        assert!(!served.contains(word), "the site names {word}: {served}");
    }
    // No close and no reset: the same connection answers a second request.
    tls.write_all(b"GET /robots.txt HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .await
        .unwrap();
    let (head, body) = response(&mut tls).await;
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    assert!(String::from_utf8_lossy(&body).contains("User-agent"));
}

#[tokio::test]
async fn an_authenticated_client_gets_the_tunnel_and_the_same_certificate() {
    test_host::need_loopback!();
    let fixture = fixture(false).await;
    let ca = ca();
    let addr = serve(&fixture, &ca).await;
    let mut tls = connect(addr, &ca).await;
    let proof = bound(fixture.key, &tls).prove([3u8; 16], cover::unix_now());
    let mut sent = proof;
    sent.extend_from_slice(b"hello");
    tls.write_all(&sent).await.unwrap();
    let mut got = [0u8; 5];
    tls.read_exact(&mut got).await.unwrap();
    assert_eq!(&got, b"hello");

    let plain = connect(addr, &ca).await;
    let page = {
        let mut page = plain;
        page.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await
            .unwrap();
        response(&mut page).await.1
    };
    assert_eq!(page, PAGE.as_bytes());
    // The switch and the site present one certificate.
    assert_eq!(tls.get_ref().1.peer_certificates().unwrap()[0], {
        let other = connect(addr, &ca).await;
        other.get_ref().1.peer_certificates().unwrap()[0].clone()
    });
}

#[tokio::test]
async fn a_proof_bound_to_another_connection_gets_the_site() {
    test_host::need_loopback!();
    let fixture = fixture(false).await;
    let ca = ca();
    let addr = serve(&fixture, &ca).await;
    let other = connect(addr, &ca).await;
    let elsewhere = exporter(&other);
    let mut tls = connect(addr, &ca).await;
    // A proof bound to the exporter of another handshake fails the tag,
    // even inside the window and with a fresh nonce.
    let proof = Switch::new(fixture.key)
        .bind(&elsewhere)
        .prove([7u8; 16], cover::unix_now());
    tls.write_all(&proof).await.unwrap();
    let (head, _) = response(&mut tls).await;
    assert!(head.starts_with("HTTP/1.1 4"), "{head}");
    // The same connection, bound to its own exporter, opens the tunnel.
    let mut tls = connect(addr, &ca).await;
    let proof = bound(fixture.key, &tls).prove([8u8; 16], cover::unix_now());
    let mut sent = proof;
    sent.extend_from_slice(b"hello");
    tls.write_all(&sent).await.unwrap();
    let mut got = [0u8; 5];
    tls.read_exact(&mut got).await.unwrap();
    assert_eq!(&got, b"hello");
}

#[tokio::test]
async fn a_replayed_proof_past_the_window_falls_through_to_the_site() {
    test_host::need_loopback!();
    let fixture = fixture(false).await;
    let ca = ca();
    let addr = serve(&fixture, &ca).await;
    let mut tls = connect(addr, &ca).await;
    // A proof of an hour ago: outside the four-minute window.
    let old = bound(fixture.key, &tls).prove([4u8; 16], cover::unix_now() - 3600);
    tls.write_all(&old).await.unwrap();
    let (head, _) = response(&mut tls).await;
    assert!(head.starts_with("HTTP/1.1 4"), "{head}");
    // The connection is still the site, not a close.
    tls.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .await
        .unwrap();
    let (head, body) = response(&mut tls).await;
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    assert_eq!(body, PAGE.as_bytes());
}

#[tokio::test]
async fn a_replayed_proof_inside_the_window_falls_through_to_the_site() {
    test_host::need_loopback!();
    let fixture = fixture(false).await;
    let ca = ca();
    let addr = serve(&fixture, &ca).await;
    let mut first = connect(addr, &ca).await;
    let proof = bound(fixture.key, &first).prove([5u8; 16], cover::unix_now());
    // The first connection opens the tunnel and spends the nonce.
    let mut sent = proof.clone();
    sent.extend_from_slice(b"hello");
    first.write_all(&sent).await.unwrap();
    let mut got = [0u8; 5];
    first.read_exact(&mut got).await.unwrap();
    assert_eq!(&got, b"hello");
    // The same proof on a second connection is not a session (SWITCH-3).
    let mut second = connect(addr, &ca).await;
    second.write_all(&proof).await.unwrap();
    let (head, _) = response(&mut second).await;
    assert!(head.starts_with("HTTP/1.1 4"), "{head}");
    second
        .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .await
        .unwrap();
    let (head, body) = response(&mut second).await;
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    assert_eq!(body, PAGE.as_bytes());
}

#[tokio::test]
async fn a_muxed_connection_sends_the_page_before_the_session() {
    test_host::need_loopback!();
    let fixture = fixture(true).await;
    let ca = ca();
    let addr = serve(&fixture, &ca).await;
    let mut tls = connect(addr, &ca).await;
    let proof = bound(fixture.key, &tls).prove([6u8; 16], cover::unix_now());
    let mut sent = proof;
    sent.extend_from_slice(b"hello");
    tls.write_all(&sent).await.unwrap();
    // MUX-2: the whole page arrives before one byte of the session.
    let page = fixture.cover.page(Some("localhost"));
    let mut got = vec![0u8; page.len()];
    tls.read_exact(&mut got).await.unwrap();
    assert_eq!(got, page, "the page completes first");
    let mut echo = [0u8; 5];
    tls.read_exact(&mut echo).await.unwrap();
    assert_eq!(&echo, b"hello");
}

#[tokio::test]
async fn a_page_load_completes_before_one_session_byte() {
    test_host::need_loopback!();
    let fixture = fixture(true).await;
    let page = fixture.cover.page(Some("localhost"));
    let session = vec![7u8; 4096];
    let chunks = fixture.cover.interleave(&page, &session);
    let mut cover_bytes = 0;
    let mut seen_session = false;
    for (lane, bytes) in &chunks {
        match lane {
            Lane::Cover => {
                assert!(!seen_session, "a cover chunk came after a session chunk");
                cover_bytes += bytes.len();
            }
            Lane::Session => seen_session = true,
        }
    }
    assert_eq!(cover_bytes, page.len());
    assert!(seen_session);
    assert_eq!(chunks[0].0, Lane::Cover);
    // MUX-1: the two lanes spend one budget, and every byte is accounted.
    let spent: usize = chunks.iter().map(|(_, bytes)| bytes.len()).sum();
    assert_eq!(spent, page.len() + session.len());
}

/// The first record of a switch and of the cover site, captured at the
/// server socket. Both clients pad their first application-data record to
/// the size that the script gives the cover's first record (SCRIPT-1).
#[tokio::test]
async fn the_first_record_of_a_switch_is_the_covers_first_record() {
    test_host::need_loopback!();
    let fixture = fixture(false).await;
    let ca = ca();
    let first_size = fixture.cover.first_record_size();
    let addr = serve(&fixture, &ca).await;

    // (b) An unauthenticated client gets the real page. The handshake is
    // recorded and then dropped: the capture starts at its end, and the
    // first record from here is the first data record of the connection.
    let plain_tee = tee(addr).await;
    let mut plain = connect(plain_tee.addr, &ca).await;
    tokio::time::sleep(Duration::from_millis(20)).await;
    plain_tee.c2s.lock().unwrap().clear();
    let request = padded_get(first_size);
    plain.write_all(&request).await.unwrap();
    let (head, body) = response(&mut plain).await;
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    assert_eq!(body, PAGE.as_bytes());

    // (a) An authenticated client gets the tunnel.
    let auth_tee = tee(addr).await;
    let mut tls = connect(auth_tee.addr, &ca).await;
    tokio::time::sleep(Duration::from_millis(20)).await;
    auth_tee.c2s.lock().unwrap().clear();
    let proof = bound(fixture.key, &tls).prove([3u8; 16], cover::unix_now());
    let first = cover::pad_first(&proof, b"hello", first_size)
        .expect("the first record holds the proof and its padding");
    assert_eq!(first.len(), first_size);
    tls.write_all(&first).await.unwrap();
    let mut got = [0u8; 5];
    tls.read_exact(&mut got).await.unwrap();
    assert_eq!(&got, b"hello", "the tunnel sees the session bytes only");

    // Both present the same certificate.
    let plain_cert = plain.get_ref().1.peer_certificates().unwrap()[0].clone();
    let auth_cert = tls.get_ref().1.peer_certificates().unwrap()[0].clone();
    assert_eq!(plain_cert, auth_cert, "one certificate on one address");

    let plain_first = first_application(&plain_tee.c2s);
    let auth_first = first_application(&auth_tee.c2s);
    println!(
        "unauthenticated application records: {:?}\n\
         authenticated application records: {:?}",
        tls_records(&plain_tee.c2s)
            .iter()
            .filter(|record| record.kind == 0x17)
            .map(|record| record.payload)
            .collect::<Vec<_>>(),
        tls_records(&auth_tee.c2s)
            .iter()
            .filter(|record| record.kind == 0x17)
            .map(|record| record.payload)
            .collect::<Vec<_>>()
    );
    println!(
        "cover first record size from the script: {first_size} application bytes\n\
         unauthenticated first record: type {} version {:02x}{:02x} payload {} bytes\n\
         authenticated first record: type {} version {:02x}{:02x} payload {} bytes",
        plain_first.kind,
        plain_first.version[0],
        plain_first.version[1],
        plain_first.payload,
        auth_first.kind,
        auth_first.version[0],
        auth_first.version[1],
        auth_first.payload
    );

    // One stated bound: the first record of each connection is the script's
    // first record size plus the same TLS overhead, so the difference is 0.
    const BOUND: usize = 0;
    assert_eq!(
        plain_first.kind, 0x17,
        "a cover request is application data"
    );
    assert_eq!(auth_first.kind, 0x17, "a proof is application data");
    assert_eq!(plain_first.version, [3, 3], "the cover records TLS 1.3");
    assert_eq!(
        auth_first.version, plain_first.version,
        "one record version"
    );
    assert_eq!(
        plain_first.payload,
        first_size + TLS13_OVERHEAD,
        "the cover request is padded"
    );
    assert_eq!(
        auth_first.payload,
        first_size + TLS13_OVERHEAD,
        "the proof is padded"
    );
    assert_eq!(
        plain_first.payload.abs_diff(auth_first.payload),
        BOUND,
        "the first records differ, over the bound of {BOUND} bytes"
    );

    // The committed capture fixture carries the same values and its
    // provenance: how it was produced, on what date, with what cover
    // version.
    let text = std::fs::read_to_string(first_record_fixture()).unwrap();
    assert!(text.contains("produced_by:"), "{text}");
    assert!(text.contains("produced_on: 2026-11-21"), "{text}");
    assert!(text.contains("cover_version: 1.7.2"), "{text}");
    let mut seen = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split_whitespace().collect();
        assert_eq!(fields.len(), 4, "a capture line of four fields: {line}");
        let kind: u8 = fields[1].parse().unwrap();
        assert_eq!(fields[2].len(), 4, "a version of two hex bytes: {line}");
        let version = [
            u8::from_str_radix(&fields[2][0..2], 16).unwrap(),
            u8::from_str_radix(&fields[2][2..4], 16).unwrap(),
        ];
        let payload: usize = fields[3].parse().unwrap();
        seen.push((fields[0].to_string(), kind, version, payload));
    }
    assert_eq!(seen.len(), 2, "{seen:?}");
    for (label, kind, version, payload) in seen {
        let record = match label.as_str() {
            "authenticated" => auth_first,
            "unauthenticated" => plain_first,
            other => panic!("a capture label of {other:?}"),
        };
        assert_eq!(kind, record.kind, "{label}");
        assert_eq!(version, record.version, "{label}");
        assert_eq!(payload, record.payload, "{label}");
        assert_eq!(payload, first_size + TLS13_OVERHEAD, "{label}");
    }
}

/// A padded proof that arrives in two TLS records, the first ending exactly
/// at the proof, still reaches the tunnel without its padding.
#[tokio::test]
async fn a_first_record_split_at_the_proof_keeps_the_padding_out_of_the_session() {
    test_host::need_loopback!();
    let fixture = fixture(false).await;
    let ca = ca();
    let addr = serve(&fixture, &ca).await;
    let mut tls = connect(addr, &ca).await;
    let size = fixture.cover.first_record_size();
    let proof = bound(fixture.key, &tls).prove([9u8; 16], cover::unix_now());
    let first = cover::pad_first(&proof, b"hello", size).unwrap();
    // Two writes, two TLS records: the second one carries the padding.
    tls.write_all(&first[..pod_cover::switch::PROOF_LEN])
        .await
        .unwrap();
    tls.flush().await.unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    tls.write_all(&first[pod_cover::switch::PROOF_LEN..])
        .await
        .unwrap();
    let mut got = [0u8; 5];
    tls.read_exact(&mut got).await.unwrap();
    assert_eq!(&got, b"hello", "the padding is not session bytes");
}

/// The script fixture is a measurement of the local cover, and the shaped
/// stream reproduces that measurement within the stated tolerance.
#[tokio::test]
async fn the_script_reproduces_the_cover_that_was_measured() {
    test_host::need_loopback!();
    let fixture = fixture(false).await;
    let ca = ca();
    let script = fixture.cover.script().clone();
    assert!(!script.record_size.is_empty());
    assert!(!script.timing.is_empty());

    let measured = measure_cover(&fixture, &ca).await;
    let sizes: Vec<usize> = measured
        .records
        .iter()
        .map(|record| record.payload - TLS13_OVERHEAD)
        .collect();
    println!(
        "measured gaps between cover records: {:?} us",
        measured.gaps
    );
    let buckets: Vec<(usize, usize)> = script
        .record_size
        .iter()
        .map(|bucket| (bucket.min, bucket.max))
        .collect();

    // The fixture is the cover: its first record is the first measured one,
    // and every measured record falls in a recorded bucket.
    let first_size = fixture.cover.first_record_size();
    println!("first script record {first_size}, measured {sizes:?}");
    // The script's first record and the cover's measured first record are in
    // one bucket: the replay picks a size inside the measured bucket, so the
    // two can differ by the width of that bucket and no more.
    assert!(
        buckets
            .iter()
            .any(|&(min, max)| (min..=max).contains(&first_size)),
        "the script's first record of {first_size} bytes is outside its own buckets"
    );
    assert!(
        buckets.iter().any(|&(min, max)| {
            (min..=max).contains(&sizes[0]) && (min..=max).contains(&first_size)
        }),
        "the cover's first record of {} bytes and the script's {first_size} are in no one bucket",
        sizes[0]
    );
    for size in &sizes {
        assert!(
            buckets.iter().any(|&(min, max)| (min..=max).contains(size)),
            "a measured cover record of {size} bytes is outside the script"
        );
    }

    let total: f64 = script
        .record_size
        .iter()
        .map(|bucket| bucket.count as f64)
        .sum();
    let declared: Vec<f64> = script
        .record_size
        .iter()
        .map(|bucket| bucket.count as f64 / total)
        .collect();
    let observed = fractions(&sizes, &buckets);
    let distance = total_variation(&declared, &observed);
    println!(
        "measured cover records: {sizes:?}\n\
         script buckets: {buckets:?}\n\
         declared fractions: {declared:?}\n\
         observed fractions: {observed:?}\n\
         total variation distance {distance:.4} (tolerance 0.10)"
    );
    assert!(
        distance <= 0.10,
        "the measured cover is {distance:.4} from its script"
    );

    // The timing buckets hold the measured gaps between response records.
    // The scheduler can stretch one gap under load, so the check is of the
    // median gap of the five runs at each position, not of every sample.
    assert_eq!(measured.gaps.len(), MEASURED_RUNS);
    assert_eq!(script.timing.len(), 2, "one bucket for each gap of a run");
    for (position, bucket) in script.timing.iter().enumerate() {
        let mut samples: Vec<u64> = measured
            .gaps
            .iter()
            .map(|run| {
                assert_eq!(run.len(), script.timing.len(), "a run of three records");
                run[position]
            })
            .collect();
        samples.sort_unstable();
        let median = samples[samples.len() / 2];
        assert!(
            median.abs_diff(bucket.micros) <= bucket.jitter_micros.max(1),
            "the median gap {median} us at position {position} is outside {} +- {} us (gaps {samples:?})",
            bucket.micros,
            bucket.jitter_micros
        );
    }

    // The shaped stream reproduces the recorded histogram within the stated
    // tolerance, not the histogram that the profile computes from itself.
    let payload = vec![5u8; 1 << 20];
    let chunks = fixture.cover.chunks(&payload);
    assert_eq!(chunks.iter().map(Vec::len).sum::<usize>(), payload.len());
    let shaped_sizes: Vec<usize> = chunks.iter().map(Vec::len).collect();
    for size in shaped_sizes
        .iter()
        .take(shaped_sizes.len().saturating_sub(1))
    {
        assert!(
            buckets.iter().any(|&(min, max)| (min..=max).contains(size)),
            "a shaped record of {size} bytes is outside the measured script"
        );
    }
    let shaped = fractions(&shaped_sizes, &buckets);
    let shaped_distance = total_variation(&declared, &shaped);
    println!(
        "shaped records: {}\n\
         shaped fractions: {shaped:?}\n\
         total variation distance {shaped_distance:.4} (tolerance 0.05)",
        shaped_sizes.len()
    );
    assert!(
        shaped_distance <= 0.05,
        "the shaped stream is {shaped_distance:.4} from its script"
    );
    assert_eq!(script.replay(4096), script.replay(4096), "a replay repeats");
}

fn config_with(dir: &Path, script: PathBuf) -> Config {
    Config {
        site: dir.display().to_string(),
        name: Some("localhost".to_string()),
        script: script.display().to_string(),
        switch_key: [1u8; 32],
        target: "127.0.0.1:1".to_string(),
        server: None,
        mux: false,
    }
}

#[test]
fn a_missing_script_or_one_with_no_date_fails_at_load() {
    let dir = tempfile::tempdir().unwrap();
    write_site(dir.path());
    let missing = dir.path().join("missing.toml");
    let err = Cover::load(config_with(dir.path(), missing)).unwrap_err();
    assert_eq!(podnet::exit::code_of(&err), 78, "{err:#}");

    let undated = dir.path().join("undated.toml");
    std::fs::write(
        &undated,
        AUTHORED_SCRIPT.replace("date = \"2026-11-20\"\n", ""),
    )
    .unwrap();
    let err = Cover::load(config_with(dir.path(), undated)).unwrap_err();
    assert_eq!(podnet::exit::code_of(&err), 78, "{err:#}");
}

#[test]
fn the_measured_script_fixture_carries_its_provenance() {
    let text = std::fs::read_to_string(measured_script()).unwrap();
    assert!(
        text.contains("measured"),
        "the fixture names its kind: {text}"
    );
    assert!(text.contains("produced_by:"), "{text}");
    assert!(text.contains("produced_on: 2026-11-21"), "{text}");
    assert!(text.contains("cover_version: 1.7.2"), "{text}");
    let script = Script::parse(&text).unwrap();
    assert_eq!(script.date, "2026-11-21");
    assert_eq!(script.cover_version, "1.7.2");
    assert!(!script.record_size.is_empty());
    assert!(!script.timing.is_empty());
    assert!(script.split.min >= 16);
}

#[test]
fn the_authored_script_says_that_it_is_authored() {
    let script = Script::parse(AUTHORED_SCRIPT).unwrap();
    assert_eq!(script.date, "2026-11-20");
    assert_eq!(script.split.min, 512);
    assert!(
        AUTHORED_SCRIPT.contains("not a measurement"),
        "an authored script must not claim a measurement"
    );
}
