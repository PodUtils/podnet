//! Deferred responses and the traffic generator (T-371, GitHub #494): a
//! local origin on the loopback, no root and no public network.

mod test_host;

use std::sync::Arc;
use std::time::{Duration, Instant};

use pod_harness::capture::{Capture, Direction, Packet};
use pod_harness::flow::FlowStats;
use podnet::defer::carrier::{frames, Carrier};
use podnet::defer::{Defer, Generator, HttpOrigin, Profile, Request, Timing, PROFILE_NAMES};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// An origin on the loopback that waits `delay` and answers `body`.
async fn origin(delay: Duration, body: Vec<u8>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let body = Arc::new(body);
    tokio::spawn(async move {
        while let Ok((mut tcp, _)) = listener.accept().await {
            let body = Arc::clone(&body);
            tokio::spawn(async move {
                let mut request = vec![0u8; 4096];
                let _ = tcp.read(&mut request).await;
                tokio::time::sleep(delay).await;
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = tcp.write_all(head.as_bytes()).await;
                let _ = tcp.write_all(&body[..]).await;
                let _ = tcp.shutdown().await;
            });
        }
    });
    addr.to_string()
}

const REQUEST: &[u8] = b"GET /page HTTP/1.1\r\nHost: origin\r\n\r\n";

/// Every byte of a direct fetch, read from the origin by the client.
async fn direct_fetch(address: &str, request: &[u8]) -> Vec<u8> {
    let mut tcp = TcpStream::connect(address).await.unwrap();
    tcp.write_all(request).await.unwrap();
    tcp.flush().await.unwrap();
    let mut out = Vec::new();
    tcp.read_to_end(&mut out).await.unwrap();
    out
}

/// The wall time of a direct fetch of `request`, outside the cache.
async fn direct_delay(address: &str, request: &[u8]) -> Duration {
    let start = Instant::now();
    let out = direct_fetch(address, request).await;
    assert!(!out.is_empty(), "the direct fetch returned no bytes");
    start.elapsed()
}

#[tokio::test]
async fn the_first_is_live_the_second_is_prepared_and_a_miss_is_live() {
    test_host::need_loopback!();
    let addr = origin(
        Duration::from_millis(30),
        b"the page of the origin".to_vec(),
    )
    .await;
    let mut defer = Defer::new(HttpOrigin::new(addr.clone()), 4096);
    let first = defer.serve(REQUEST).await.unwrap();
    assert!(
        matches!(first.timing, Timing::Live(_)),
        "{:?}",
        first.timing
    );
    assert!(
        first.timing.delay() >= Duration::from_millis(25),
        "the live answer took {:?}",
        first.timing.delay()
    );
    assert!(first.bytes.ends_with(b"the page of the origin"));

    let second = defer.serve(REQUEST).await.unwrap();
    assert!(second.timing.replayed(), "{:?}", second.timing);
    assert_eq!(second.bytes, first.bytes);
    assert!(
        second.timing.delay() < first.timing.delay(),
        "the prepared copy took {:?} against {:?}",
        second.timing.delay(),
        first.timing.delay()
    );
    assert_eq!(defer.replayed(), 1);

    // A direct fetch, outside the cache, is the control for both timings.
    let direct = direct_delay(&addr, REQUEST).await;
    println!(
        "live {:?}, replay {:?}, direct fetch {:?}",
        first.timing.delay(),
        second.timing.delay(),
        direct
    );
    assert!(
        first.timing.delay() >= direct.mul_f64(0.8),
        "the first request was faster than the origin: {:?} against a direct fetch of {direct:?}",
        first.timing.delay()
    );
    assert!(
        second.timing.delay() <= direct,
        "the prepared copy was slower than a direct fetch: {:?} against {direct:?}",
        second.timing.delay()
    );
    assert!(
        second.timing.delay() >= direct / 4,
        "the prepared copy was instant, not a plausible half of the origin: {:?} against a direct fetch of {direct:?}",
        second.timing.delay()
    );

    let miss = defer
        .serve(b"GET /other HTTP/1.1\r\nHost: origin\r\n\r\n")
        .await
        .unwrap();
    assert!(matches!(miss.timing, Timing::Live(_)));
    assert!(
        miss.timing.delay() >= direct.mul_f64(0.8),
        "a miss took {:?}, faster than a direct fetch of {direct:?}",
        miss.timing.delay()
    );
    assert_eq!(defer.live(), 2);
}

/// The stated limit of the bytes that relay framing may add over the payload.
const OVERHEAD_LIMIT: f64 = 0.01;
/// The cache chunk and the body that is larger than it.
const CHUNK: usize = 4096;
const BODY: usize = 1 << 20;

/// What a wire measurement read. `relay` and `direct` are complete
/// responses as the client read them; `payload` is the body alone.
struct Wire {
    relay: u64,
    payload: u64,
    prepared: u64,
    old_overhead: u64,
    direct: u64,
}

/// Serve a `BODY`-byte response through a relay that writes HTTP/1.1
/// chunked framing of `CHUNK` bytes, and read every byte the relay wrote.
/// A direct fetch of the same request is the control.
async fn measure_large_body() -> Wire {
    let addr = origin(Duration::ZERO, vec![7u8; BODY]).await;
    let direct = direct_fetch(&addr, REQUEST).await.len() as u64;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let relay_addr = listener.local_addr().unwrap();
    let server = async move {
        let (mut tcp, _) = listener.accept().await.unwrap();
        let mut request = vec![0u8; 4096];
        let _ = tcp.read(&mut request).await;
        let mut defer = Defer::new(HttpOrigin::new(addr), CHUNK);
        let served = defer.serve(REQUEST).await.unwrap();
        let at = served
            .bytes
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .expect("the origin's head")
            + 4;
        let body = &served.bytes[at..];
        let head = b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
        tcp.write_all(head).await.unwrap();
        let mut wire = head.len() as u64;
        for piece in body.chunks(CHUNK) {
            let line = format!("{:x}\r\n", piece.len());
            tcp.write_all(line.as_bytes()).await.unwrap();
            tcp.write_all(piece).await.unwrap();
            tcp.write_all(b"\r\n").await.unwrap();
            wire += line.len() as u64 + piece.len() as u64 + 2;
        }
        tcp.write_all(b"0\r\n\r\n").await.unwrap();
        wire += 5;
        tcp.shutdown().await.unwrap();
        (
            wire,
            body.len() as u64,
            defer.prepared_bytes() as u64,
            defer.overhead(),
        )
    };
    let client = async {
        let mut tcp = TcpStream::connect(relay_addr).await.unwrap();
        tcp.write_all(REQUEST).await.unwrap();
        tcp.flush().await.unwrap();
        let mut got = Vec::new();
        tcp.read_to_end(&mut got).await.unwrap();
        got
    };
    let ((wire, payload, prepared, old_overhead), got) = tokio::join!(server, client);
    assert_eq!(
        got.len() as u64,
        wire,
        "the client read {} bytes and the relay counted {wire}",
        got.len()
    );
    Wire {
        relay: wire,
        payload,
        prepared,
        old_overhead,
        direct,
    }
}

#[tokio::test]
async fn the_wire_overhead_of_a_body_over_the_chunk_stays_under_the_stated_ratio() {
    test_host::need_loopback!();
    let wire = measure_large_body().await;
    let numerator = wire.relay - wire.payload;
    let denominator = wire.payload;
    let ratio = numerator as f64 / denominator as f64;
    let direct_numerator = wire.direct - wire.payload;
    let direct_ratio = direct_numerator as f64 / denominator as f64;
    println!(
        "wire overhead: numerator {numerator} bytes (relay {} - payload {denominator}), denominator {denominator} bytes, ratio {ratio:.6} ({:.3}%)",
        wire.relay,
        ratio * 100.0
    );
    println!(
        "direct control: numerator {direct_numerator} bytes (direct {} - payload {denominator}), denominator {denominator} bytes, ratio {direct_ratio:.6} ({:.3}%)",
        wire.direct,
        direct_ratio * 100.0
    );
    println!(
        "old prepared-bytes accounting: prepared {} bytes, overhead {} bytes",
        wire.prepared, wire.old_overhead
    );
    assert_eq!(wire.payload, BODY as u64, "the payload is the whole body");
    assert_eq!(
        wire.old_overhead, 0,
        "a body over the chunk is not prepared, so the old accounting reports zero"
    );
    assert!(
        numerator > wire.old_overhead,
        "the measured wire overhead {numerator} must exceed the old accounting {}",
        wire.old_overhead
    );
    assert!(
        ratio <= OVERHEAD_LIMIT,
        "the relay framing added {numerator} bytes over {denominator}, a ratio of {ratio:.6} over the stated limit {OVERHEAD_LIMIT}"
    );
    assert!(
        ratio - direct_ratio <= 0.005,
        "the relay adds {:.6} over the direct fetch, over 0.005",
        ratio - direct_ratio
    );
}

/// The failing control: the prepared-bytes accounting of the cache is zero
/// for a body over the chunk, while the relay wrote real framing bytes. The
/// old accounting is not the wire overhead, which is the defect this test
/// replaced. The test asserts the two numbers directly (not with
/// `should_panic`), so that a host with no loopback skips it.
#[tokio::test]
async fn the_old_prepared_bytes_accounting_undercounts_the_wire() {
    test_host::need_loopback!();
    let wire = measure_large_body().await;
    let numerator = wire.relay - wire.payload;
    assert_eq!(
        wire.old_overhead, 0,
        "the old accounting reports {} bytes, not 0",
        wire.old_overhead
    );
    assert_ne!(
        wire.old_overhead, numerator,
        "the old accounting reports {} bytes, the wire wrote {numerator} bytes over the payload",
        wire.old_overhead
    );
}

#[tokio::test]
async fn a_response_larger_than_the_chunk_is_served_live_each_time() {
    test_host::need_loopback!();
    let addr = origin(Duration::from_millis(5), vec![3u8; 512]).await;
    let mut defer = Defer::new(HttpOrigin::new(addr), 64);
    let a = defer.serve(REQUEST).await.unwrap();
    let b = defer.serve(REQUEST).await.unwrap();
    assert!(!a.timing.replayed());
    assert!(!b.timing.replayed());
    assert_eq!(defer.prepared_bytes(), 0);
    assert_eq!(defer.overhead(), 0);
}

#[tokio::test]
async fn a_response_carries_a_control_stream_inside_the_http_profile() {
    test_host::need_loopback!();
    let body = b"the page of the origin";
    let addr = origin(Duration::from_millis(5), body.to_vec()).await;
    let mut carrier = Carrier::new();
    carrier.push(b"start");
    carrier.push(b"next");
    let mut defer = Defer::new(HttpOrigin::new(addr.clone()), 4096).carrying(carrier);

    let first = defer.serve(REQUEST).await.unwrap();
    // The far end parses the response as HTTP/1.1: a status line, headers
    // and a body whose length the head declares.
    let head_end = first
        .bytes
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .unwrap()
        + 4;
    let head = std::str::from_utf8(&first.bytes[..head_end]).unwrap();
    assert!(head.starts_with("HTTP/1.1 200 OK\r\n"), "{head}");
    let length: usize = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().ok())
                .flatten()
        })
        .unwrap();
    assert_eq!(
        &first.bytes[head_end..],
        body,
        "the body is untouched; head {:?}",
        String::from_utf8_lossy(&first.bytes[..head_end])
    );
    assert_eq!(length, body.len());
    assert_eq!(frames(&first.bytes).unwrap(), vec![b"start".to_vec()]);

    // The carrier adds real bytes to the head; a direct fetch of the same
    // request, with no carrier, is the control for that overhead.
    let direct = direct_fetch(&addr, REQUEST).await;
    let carrier_overhead = first.bytes.len() as i64 - direct.len() as i64;
    println!(
        "carrier wire overhead: {carrier_overhead} bytes (live response {} - direct {})",
        first.bytes.len(),
        direct.len()
    );
    assert!(
        carrier_overhead > 0,
        "the carrier added no bytes to the response"
    );
    assert!(
        carrier_overhead as usize <= defer.chunk(),
        "the carrier line is {carrier_overhead} bytes, over the chunk of {}",
        defer.chunk()
    );

    let second = defer.serve(REQUEST).await.unwrap();
    assert!(second.timing.replayed());
    let direct_timing = direct_delay(&addr, REQUEST).await;
    println!(
        "carrier replay {:?}, direct fetch {direct_timing:?}",
        second.timing.delay()
    );
    assert!(
        second.timing.delay() <= direct_timing,
        "the carrier replay was slower than a direct fetch: {:?} against {direct_timing:?}",
        second.timing.delay()
    );
    assert!(
        second.timing.delay() >= direct_timing / 4,
        "the carrier replay was instant, not a plausible half of the origin: {:?} against {direct_timing:?}",
        second.timing.delay()
    );
    let second_end = second
        .bytes
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .unwrap()
        + 4;
    assert_eq!(&second.bytes[second_end..], body, "the body is untouched");
    assert_eq!(frames(&second.bytes).unwrap(), vec![b"next".to_vec()]);
    assert_eq!(defer.carrier_mut().unwrap().sent(), 2);
    // The prepared copy is still the origin's bytes: the carrier is added
    // to the served copy only.
    assert_eq!(defer.prepared_bytes(), 0);
}

#[tokio::test]
async fn a_response_with_no_room_in_the_chunk_keeps_the_frame_for_later() {
    test_host::need_loopback!();
    let body = &[5u8; 32];
    let addr = origin(Duration::from_millis(2), body.to_vec()).await;
    let mut carrier = Carrier::new();
    carrier.push(b"later");
    let mut defer = Defer::new(HttpOrigin::new(addr), 100).carrying(carrier);
    let first = defer.serve(REQUEST).await.unwrap();
    assert!(first.bytes.ends_with(body));
    assert!(frames(&first.bytes).unwrap().is_empty());
    assert_eq!(defer.carrier_mut().unwrap().len(), 1);
}

#[test]
fn each_application_profile_drives_the_generator() {
    for name in PROFILE_NAMES {
        let profile = Profile::load(name).unwrap();
        assert!(!profile.object.is_empty(), "{name}");
        let stream = Generator::new(profile.clone(), 5).stream(25);
        assert_eq!(stream.len(), 25, "{name}");
        let last = stream.last().unwrap();
        assert!(last.at > Duration::ZERO, "{name} is a stream over time");
        assert!(stream.iter().any(|request| request.size > 0), "{name}");
        // The profile of a page load and of a terminal pauses to read; a
        // video call and a file transfer do not.
        let reads = stream.iter().filter(|request| request.read).count();
        match name {
            "page-load" | "terminal" => assert!(reads > 0, "{name} has a read pause"),
            _ => assert_eq!(reads, 0, "{name} has no read pause"),
        }
    }
}

/// A capture of the server's records of a generated stream: one TLS
/// application-data record for each 16384 bytes of an object, at the times
/// of the profile.
fn generated_capture(stream: &[Request]) -> Capture {
    let mut capture = Capture::new();
    for request in stream {
        let mut at = request.at.as_micros() as u64;
        let mut remaining = request.size;
        while remaining > 0 {
            let len = remaining.min(16384);
            let mut bytes = vec![0x17, 0x03, 0x03, (len >> 8) as u8, len as u8];
            bytes.resize(5 + len, 0x41);
            capture.push(Packet::new(Direction::ServerToClient, at, bytes));
            remaining -= len;
            at += 1;
        }
    }
    capture
}

/// A plain tunnel stream: `bytes` of fixed 128-byte records, one every
/// millisecond, with no application profile.
fn plain_capture(bytes: usize) -> Capture {
    let mut capture = Capture::new();
    let mut at = 0u64;
    let mut remaining = bytes;
    while remaining > 0 {
        let len = remaining.min(128);
        let mut packet = vec![0x17, 0x03, 0x03, (len >> 8) as u8, len as u8];
        packet.resize(5 + len, 0x41);
        capture.push(Packet::new(Direction::ServerToClient, at, packet));
        remaining -= len;
        at += 1_000;
    }
    capture
}

/// The generator's page-load stream scores lower than a plain tunnel of the
/// same bytes in the existing pod-harness flow classifier. Both scores are
/// printed with the classifier and the margin that the assertion states.
#[test]
fn the_generated_stream_scores_lower_than_a_plain_tunnel_in_the_harness() {
    let model = FlowStats::from_json(include_str!("data/harness/flow.json")).unwrap();
    println!("classifier: pod-harness FlowStats (flow), model tests/data/harness/flow.json");
    for name in PROFILE_NAMES {
        let profile = Profile::load(name).unwrap();
        let stream = Generator::new(profile.clone(), 9).stream(200);
        let total: usize = stream.iter().map(|request| request.size).sum();
        let generated = generated_capture(&stream);
        let plain = plain_capture(total);
        let (generated_score, generated_reason) = model.score(&generated);
        let (plain_score, plain_reason) = model.score(&plain);
        println!(
            "generated {name} ({}): score {generated_score:.6} ({generated_reason})",
            profile.application
        );
        println!(
            "plain {name} ({total} bytes in 128-byte records every 1 ms): score {plain_score:.6} ({plain_reason})"
        );
        assert!(
            plain_score > 0.5,
            "{name}: the plain tunnel scored {plain_score:.6}, not a tunnel"
        );
        assert!(
            generated_score + 0.25 <= plain_score,
            "{name}: generated {generated_score:.6} is not at least 0.25 below the plain tunnel {plain_score:.6}"
        );
    }
}

#[tokio::test]
async fn the_generator_reaches_the_cache_with_the_times_of_the_profile() {
    test_host::need_loopback!();
    let addr = origin(Duration::from_millis(20), b"ok".to_vec()).await;
    let mut defer = Defer::new(HttpOrigin::new(addr.clone()), 1024);
    let profile = Profile::load("page-load").unwrap();
    let stream = Generator::new(profile.clone(), 9).stream(8);
    for pair in stream.windows(2) {
        let gap = pair[1].at - pair[0].at;
        assert!(
            gap >= Duration::from_millis(profile.gap_ms.0),
            "a generated gap of {gap:?} is below the profile's {} ms",
            profile.gap_ms.0
        );
        let longest = profile.gap_ms.1 + profile.read_ms.unwrap_or((0, 0)).1;
        assert!(
            gap <= Duration::from_millis(longest),
            "a generated gap of {gap:?} is over the profile's {longest} ms"
        );
    }
    let ask = b"GET /object HTTP/1.1\r\nHost: origin\r\n\r\n";
    let first = defer.serve(ask).await.unwrap();
    assert!(matches!(first.timing, Timing::Live(_)));
    let second = defer.serve(ask).await.unwrap();
    assert!(second.timing.replayed(), "{:?}", second.timing);
    let direct = direct_delay(&addr, ask).await;
    println!(
        "generated replay {:?}, direct fetch {direct:?}",
        second.timing.delay()
    );
    assert!(
        first.timing.delay() >= direct.mul_f64(0.8),
        "the first generated request was faster than the origin: {:?} against {direct:?}",
        first.timing.delay()
    );
    assert!(
        second.timing.delay() <= direct,
        "the generated replay was slower than a direct fetch: {:?} against {direct:?}",
        second.timing.delay()
    );
    assert!(
        second.timing.delay() >= direct / 4,
        "the generated replay was instant: {:?} against {direct:?}",
        second.timing.delay()
    );
    for _ in 2..stream.len() {
        let _ = defer.serve(ask).await.unwrap();
    }
    assert!(defer.replayed() >= 1);
}
