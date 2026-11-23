//! The shape of the records (T-370, GitHub #493): the histogram of each
//! profile, the entropy band, the opening write and the padded fraction of
//! a transfer. The chain runs over the unix carrier: no root and no network.

mod link_record_harness;

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use pod_shape::{Band, Entropy, Profile, Shaper};
use podnet::link_record::Options;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use link_record_harness::*;

/// A profile of one bucket, for a transfer at the 512-byte bucket.
const FIXED_512: &str = r#"
name = "fixed-512"
protocol = "test"
rule = "always"
buckets = [ { size = 512, weight = 1 } ]

source = { date = "2026-11-20", cover_version = "1" }
"#;

/// A profile of one bucket, with a first flight.
const FLY_512: &str = r#"
name = "fly-512"
protocol = "test"
rule = "always"
first_flight = 16384

buckets = [ { size = 512, weight = 1 } ]

pace = { rate = 1000000, jitter = 20, keepalive = 30 }

source = { date = "2026-11-20", cover_version = "1" }
"#;

/// A profile of one bucket whose padding tail carries the entropy band.
const ENTROPY_512: &str = r#"
name = "entropy-512"
protocol = "test"
rule = "always"
buckets = [ { size = 512, weight = 1 } ]
entropy = { min = 3.0, max = 7.0, printable_run = 16 }

source = { date = "2026-11-20", cover_version = "1" }
"#;

/// The first record that carries a byte of the stream. The record layer
/// writes a padded keepalive first to measure the link (T-371), and the
/// peer drops it.
fn first_data(seen: &Seen) -> Vec<u8> {
    seen.to_c
        .lock()
        .unwrap()
        .iter()
        .find(|record| {
            let head = header(record);
            head.length > u32::from(head.pad)
        })
        .cloned()
        .unwrap()
}

/// The body of a record, before its seal is opened. A record with the
/// entropy flag puts a four-byte payload length and the padding tail around
/// the bytes of the stream (PAD-3), so its body is longer than them.
fn body_len(record: &[u8]) -> usize {
    let head = header(record);
    head.length as usize - usize::from(head.pad)
}

/// Whether the body of a record carries the entropy frame of PAD-3.
fn framed(record: &[u8]) -> bool {
    header(record).flags & podnet::link_record::ENTROPY != 0
}

/// A body that carries `payload` bytes of the stream. The body is sealed, so
/// the length it declares cannot be read here; a framed body is the four
/// byte prefix, the payload and a tail shorter than the largest bucket.
fn carries(record: &[u8], payload: usize, profile: &Profile) -> bool {
    let body = body_len(record);
    if !framed(record) {
        return body == payload;
    }
    let bucket = profile.buckets.iter().map(|b| b.size).max().unwrap_or(0);
    body > payload + 4 && body - (payload + 4) < bucket
}

/// The independently declared target histogram of the shaped bodies.
#[derive(serde::Deserialize)]
struct Histogram {
    date: String,
    profile: Vec<HistogramProfile>,
}

#[derive(serde::Deserialize)]
struct HistogramProfile {
    name: String,
    samples: usize,
    /// The records of one session, for a profile that pads only its first
    /// records. Zero keeps one session for every sample.
    session: usize,
    payloads: Vec<usize>,
    target: Vec<Target>,
}

#[derive(serde::Deserialize)]
struct Target {
    size: usize,
    fraction: f64,
}

/// How far an observed fraction may sit from the recorded target. It is
/// tighter than the crate's 0.05 bucket tolerance because the target is a
/// fixed declaration, not another sample of the same shaper.
const HISTOGRAM_TOLERANCE: f64 = 0.02;

/// The measured target of the shaped body sizes. The file was not sampled
/// from the shaper and it is not `Shape::of_profile`: it is a separate
/// declaration, so a change to the shaper or to a profile shows up here.
#[test]
fn each_profile_matches_the_recorded_reference_histogram() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/shape/histogram.toml");
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        text.contains("produced_by: python3 scripts/shape-histogram.py"),
        "the target names its generator: {text}"
    );
    assert!(text.contains("produced_on: 2026-11-21"), "{text}");
    let histogram: Histogram = toml::from_str(&text).unwrap();
    assert_eq!(histogram.date, "2026-11-21");
    assert_eq!(
        histogram.profile.len(),
        pod_shape::NAMES.len(),
        "one target for each profile"
    );

    for expected in &histogram.profile {
        let profile =
            Profile::load(&expected.name).unwrap_or_else(|e| panic!("{}: {e}", expected.name));
        assert!(!expected.payloads.is_empty(), "{}", expected.name);
        let declared: f64 = expected.target.iter().map(|t| t.fraction).sum();
        assert!(
            (declared - 1.0).abs() < 1e-4,
            "{}: the target fractions sum to {declared}",
            expected.name
        );

        let mut shaper = Shaper::new(profile.clone(), 0x7a11_0001);
        let mut samples = Vec::with_capacity(expected.samples);
        for at in 0..expected.samples {
            if at > 0 && expected.session > 0 && at % expected.session == 0 {
                shaper = Shaper::new(profile.clone(), 0x7a11_0001 ^ at as u64);
            }
            let payload = expected.payloads[at % expected.payloads.len()];
            samples.push(shaper.shape(payload).body);
        }

        for target in &expected.target {
            let got = samples.iter().filter(|&&size| size == target.size).count() as f64
                / samples.len() as f64;
            assert!(
                (got - target.fraction).abs() <= HISTOGRAM_TOLERANCE,
                "{}: the body size {} is {got:.4} of {} samples, the recorded target is {:.4} (tolerance {HISTOGRAM_TOLERANCE})",
                expected.name,
                target.size,
                samples.len(),
                target.fraction
            );
        }
        for size in &samples {
            assert!(
                expected.target.iter().any(|target| target.size == *size),
                "{}: a body of {size} bytes is outside the recorded target {:?}",
                expected.name,
                expected.target.iter().map(|t| t.size).collect::<Vec<_>>()
            );
        }
    }
}

#[test]
fn each_profile_holds_its_entropy_band_over_1000_windows() {
    for name in pod_shape::NAMES {
        let profile = Profile::load(name).unwrap();
        let band: Band = profile.entropy.unwrap();
        let mut entropy = Entropy::new(band, 0x7a11_0002).unwrap();
        let block = entropy.fill(4096 * 1000);
        entropy
            .check(&block)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
    }
}

#[tokio::test]
async fn a_fast_link_opens_at_the_profile_target() {
    let profile = Profile::load("page-load").unwrap();
    let opts = Options {
        profile: Some(profile.clone()),
        pace: false,
        ..Options::default()
    };
    assert_eq!(opts.opening_write(), profile.first_flight.unwrap());
    let (mut d, mut l, seen) = chain(opts, Options::default(), pass(), pass()).await;
    d.write_all(&vec![7u8; 32 * 1024]).await.unwrap();
    let mut got = vec![0u8; 32 * 1024];
    l.read_exact(&mut got).await.unwrap();
    let first = first_data(&seen);
    let opening = d.opening_write();
    assert_eq!(opening, profile.first_flight.unwrap());
    assert!(
        carries(&first, opening, &profile),
        "the first body is {} bytes for an opening write of {opening}",
        body_len(&first)
    );
}

#[tokio::test]
async fn a_measured_slow_link_sizes_the_opening_write() {
    let profile = Profile::load("page-load").unwrap();
    let opts = Options {
        profile: Some(profile.clone()),
        pace: false,
        // Zero: the record layer must measure the link itself.
        measured_bps: 0,
        ..Options::default()
    };
    // 64 kbit/s. The probe carries the rate, so the opening write is about
    // two seconds of it (FLY-1).
    let (mut d, mut l, seen) = chain_slow(opts, Options::default(), 64_000, pass(), pass()).await;
    let measured = d.measured_bps();
    assert!(
        (10_000..=250_000).contains(&measured),
        "the probe measured {measured} bit/s"
    );
    d.write_all(&vec![9u8; 32 * 1024]).await.unwrap();
    let mut got = vec![0u8; 32 * 1024];
    l.read_exact(&mut got).await.unwrap();
    let first = first_data(&seen);
    let opening = body_len(&first);
    let expected = podnet::link_record::first_flight(Some(&profile), None, measured);
    assert_eq!(d.opening_write(), expected);
    assert!(
        carries(&first, expected, &profile),
        "the first body is {opening} bytes for an opening write of {expected}"
    );
    let seconds = expected as f64 * 8.0 / measured as f64;
    assert!(
        (0.5..=4.0).contains(&seconds),
        "the first body is {seconds:.2} seconds of the measured rate {measured}"
    );
}

#[tokio::test]
async fn a_slow_link_opens_at_two_seconds_of_its_rate() {
    let profile = Profile::load("page-load").unwrap();
    let opts = Options {
        profile: Some(profile.clone()),
        measured_bps: 64_000,
        pace: false,
        ..Options::default()
    };
    assert_eq!(opts.opening_write(), 16_000);
    let (mut d, mut l, seen) = chain(opts, Options::default(), pass(), pass()).await;
    assert_eq!(d.measured_bps(), 64_000);
    assert_eq!(d.opening_write(), 16_000);
    d.write_all(&vec![9u8; 32 * 1024]).await.unwrap();
    let mut got = vec![0u8; 32 * 1024];
    l.read_exact(&mut got).await.unwrap();
    let first = first_data(&seen);
    assert!(
        carries(&first, 16_000, &profile),
        "the first body is {} bytes for the opening write of 16000",
        body_len(&first)
    );
}

/// A small echo over a link at each slow rate. The handshake is at full
/// speed; the throttle starts with the data, so the two seconds are the
/// data path and not the setup.
#[tokio::test]
async fn an_echo_at_one_four_and_twenty_kbits_passes_the_two_second_bound() {
    for bps in [1_000u64, 4_000, 20_000] {
        let opts = Options {
            measured_bps: bps,
            ..Options::default()
        };
        let (mut d, l, open) = pair_slow_after_handshake(opts, Options::default(), bps).await;
        open.store(true, Ordering::SeqCst);
        tokio::spawn(echo(l));

        let payload = b"echo";
        let started = Instant::now();
        d.write_all(payload).await.unwrap();
        let mut got = [0u8; 4];
        tokio::time::timeout(Duration::from_secs(2), d.read_exact(&mut got))
            .await
            .unwrap_or_else(|_| panic!("the echo at {bps} bit/s did not return within two seconds"))
            .unwrap();
        let elapsed = started.elapsed();
        assert_eq!(&got, payload, "the echo at {bps} bit/s");
        assert!(
            elapsed < Duration::from_secs(2),
            "the echo at {bps} bit/s took {elapsed:?}, over the two-second bound"
        );
        println!("echo at {bps} bit/s: {elapsed:?}");
    }
}

#[tokio::test]
async fn the_padding_tail_holds_the_entropy_band_over_1000_records() {
    let profile = Profile::parse("entropy-512", ENTROPY_512).unwrap();
    let opts = Options {
        profile: Some(profile),
        pace: false,
        ..Options::default()
    };
    let (d, l) = pair(opts, Options::default()).await;
    let (mut d, mut l) = (d.unwrap(), l.unwrap());
    let shape = d.shape_counters();
    let mut want = Vec::new();
    let mut got = Vec::new();
    let read = l.read_to_end(&mut got);
    let send = async {
        for i in 0..1000u32 {
            let chunk = i.to_be_bytes().repeat(8);
            let before = shape.records();
            d.write_all(&chunk).await.unwrap();
            while shape.records() == before {
                tokio::task::yield_now().await;
            }
            want.extend_from_slice(&chunk);
        }
        d.shutdown().await.unwrap();
    };
    let (_, read) = tokio::join!(send, read);
    read.unwrap();
    assert_eq!(got, want, "the plaintext is intact");
    assert_eq!(
        shape.entropy_records(),
        1000,
        "every data record has a tail"
    );
    let low = shape.entropy_low().unwrap();
    let high = shape.entropy_high().unwrap();
    assert!(
        low >= 2.98,
        "a padding tail is {low:.3} bits per byte, below the band of 3.0"
    );
    assert!(
        high <= 7.02,
        "a padding tail is {high:.3} bits per byte, above the band of 7.0"
    );
}

#[tokio::test]
async fn a_megabyte_at_the_512_bucket_stays_under_25_percent() {
    let profile = Profile::parse("fixed-512", FIXED_512).unwrap();
    let opts = Options {
        profile: Some(profile),
        pace: false,
        ..Options::default()
    };
    let (d, l) = pair(opts, Options::default()).await;
    let (mut d, mut l) = (d.unwrap(), l.unwrap());
    let shape = d.shape_counters();
    let data = vec![3u8; 1 << 20];
    let send = async {
        let mut left = 1 << 20;
        while left > 0 {
            let n = left.min(500);
            d.write_all(&vec![3u8; n]).await.unwrap();
            left -= n;
        }
        d.shutdown().await.unwrap();
    };
    let mut got = Vec::new();
    let read = l.read_to_end(&mut got);
    let (_, read) = tokio::join!(send, read);
    read.unwrap();
    assert_eq!(got.len(), data.len());
    let fraction = shape.fraction();
    assert!(
        fraction.fraction() < 0.25,
        "the padded fraction is {:.3}: {}",
        fraction.fraction(),
        shape.line()
    );
    assert!(shape.line().contains("padded"), "{}", shape.line());
}

#[tokio::test]
async fn the_first_flight_survives_the_flat_512_bucket() {
    let profile = Profile::parse("fly-512", FLY_512).unwrap();
    let opts = Options {
        profile: Some(profile.clone()),
        pace: false,
        ..Options::default()
    };
    let (mut d, mut l, seen) = chain(opts, Options::default(), pass(), pass()).await;
    d.write_all(&vec![1u8; 32 * 1024]).await.unwrap();
    let mut got = vec![0u8; 32 * 1024];
    l.read_exact(&mut got).await.unwrap();
    let first = first_data(&seen);
    assert_eq!(d.opening_write(), 16 * 1024);
    assert!(
        carries(&first, 16 * 1024, &profile),
        "the first body is {} bytes for the 16 KiB opening write",
        body_len(&first)
    );
}

#[test]
fn the_clock_of_the_pacer_is_the_one_that_the_caller_gives() {
    let profile = Profile::load("terminal").unwrap();
    let pacer = podnet::link_record::Paced::new(&profile, 800_000, Instant::now());
    assert_eq!(pacer.capacity(), 10_000);
    assert_eq!(pacer.delay(1000), std::time::Duration::from_millis(10));
}
