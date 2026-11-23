//! GitHub #484: the NTPv4, SMTP and IMAP carriers carry bytes, and do not
//! merely register. Each test drives the carrier of `pod-link` against a
//! local responder of the protocol on the loopback: an NTPv4 server over
//! UDP, a submission server and a retrieval server over TCP with TLS. The
//! registry's row is a name and a cost; this proves the carrier behind it.
//!
//! The bytes of each direction are 300, the size of the gate of the issue.
//! No test reaches a public service, and no credential is an account's.

#![cfg(all(feature = "ntp", feature = "smtp", feature = "imap"))]

mod mail_stand;
mod test_host;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine as _;
use pod_link::ntp::wire::{self, Timestamp};
use pod_link::ntp::{Ntp, Packet, Session};
use pod_link::smtp::Smtp;
use pod_link::Carrier as _;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::time::timeout;

/// 300 bytes, no two the same, so that a reordering or a byte lost is seen.
fn payload(tag: u8) -> Vec<u8> {
    (0..300u16).map(|i| (i as u8).wrapping_add(tag)).collect()
}

/// GitHub #484, the mail size limit end to end: the stand refuses a
/// message above its configured size with `552` and names the limit, and
/// the carrier reports a `Refused` limit and fails with the refusal in the
/// text. A message under the limit still goes.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_smtp_message_over_the_limit_is_refused_by_name() {
    test_host::need_loopback!();
    let server = mail_stand::smtp_with_limit(pod_link::smtp::Mode::Starttls, Some(400)).await;
    let mut carrier = Smtp::new(mail_stand::credential(), "from@stand.test", "to@stand.test")
        .with_tls(mail_stand::client_config())
        .with_mode(pod_link::smtp::Mode::Starttls);
    carrier.batch = 300;
    let mut io = carrier
        .dial(&server.addr.to_string())
        .await
        .expect("a submission socket");
    io.write_all(&payload(4)).await.unwrap();
    io.flush().await.unwrap();
    let limit = timeout(Duration::from_secs(10), async {
        loop {
            if let Some(limit) = carrier.limit() {
                break limit;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the limit of the refusal");
    let mut byte = [0u8; 1];
    let error = timeout(Duration::from_secs(10), async {
        loop {
            match io.read(&mut byte).await {
                // The stream says first that it carries no bytes down;
                // the refusal of the submission task comes after it.
                Err(e) if e.kind() == std::io::ErrorKind::Unsupported => {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                Err(e) => break e,
                Ok(_) => panic!("a byte came down the submission carrier"),
            }
        }
    })
    .await
    .expect("the carrier answers before the timeout");
    let text = error.to_string();
    assert!(
        text.contains("the server refuses a message of") && text.contains("at the smallest batch"),
        "the refusal by name: {text}"
    );
    assert_eq!(limit.kind, pod_link::LimitKind::Refused, "{limit:?}");
    assert_eq!(limit.bytes, Some(400), "the size of the stand: {limit:?}");
    assert!(
        server
            .messages
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty(),
        "no message above the limit was taken"
    );

    // The same carrier takes a message under the limit of the stand.
    let small = mail_stand::smtp_with_limit(pod_link::smtp::Mode::Starttls, Some(4096)).await;
    let mut carrier = Smtp::new(mail_stand::credential(), "from@stand.test", "to@stand.test")
        .with_tls(mail_stand::client_config())
        .with_mode(pod_link::smtp::Mode::Starttls);
    carrier.batch = 300;
    let mut io = carrier
        .dial(&small.addr.to_string())
        .await
        .expect("a submission socket");
    io.write_all(&payload(5)).await.unwrap();
    io.flush().await.unwrap();
    let messages = mail_stand::wait_messages(&small, 1).await;
    assert_eq!(messages.len(), 1, "the message under the limit");
    assert_eq!(mail_stand::smtp_body(&messages[0]), payload(5));
    let _ = io.shutdown().await;
}

/// An NTPv4 server: it answers each request with a server packet whose
/// origin is the transmit timestamp of the request, so the client accepts
/// it. The field type of each reply is the type of its request.
async fn ntp_server() -> (std::net::SocketAddr, Arc<Mutex<Vec<Packet>>>) {
    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = socket.local_addr().unwrap();
    let seen: Arc<Mutex<Vec<Packet>>> = Arc::default();
    let taken = seen.clone();
    tokio::spawn(async move {
        let mut buffer = vec![0u8; 2048];
        loop {
            let Ok((n, from)) = socket.recv_from(&mut buffer).await else {
                return;
            };
            let Ok(request) = wire::parse(&buffer[..n]) else {
                continue;
            };
            let field = request.fields.first().cloned();
            let (kind, value) = field.map(|f| (f.kind, f.value)).unwrap_or((0, Vec::new()));
            let now = Timestamp::now();
            let reply = wire::reply(&request, now, now, request.transmit, 2, kind, value);
            let Ok(bytes) = reply.encode() else { continue };
            taken.lock().unwrap().push(request);
            let _ = socket.send_to(&bytes, from).await;
        }
    });
    (addr, seen)
}

/// The server of the session test: its clock stands `offset` nanoseconds
/// ahead, each reply carries the next bytes of `down`, and it records the
/// type of each request, the bytes that came up, and the count.
async fn ntp_session_server(
    offset: i128,
    down: Vec<u8>,
) -> (
    std::net::SocketAddr,
    Arc<Mutex<Vec<u16>>>,
    Arc<Mutex<Vec<u8>>>,
    Arc<AtomicUsize>,
) {
    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = socket.local_addr().unwrap();
    let types: Arc<Mutex<Vec<u16>>> = Arc::default();
    let up: Arc<Mutex<Vec<u8>>> = Arc::default();
    let requests = Arc::new(AtomicUsize::new(0));
    let (seen, taken, count) = (types.clone(), up.clone(), requests.clone());
    let mut down = down;
    tokio::spawn(async move {
        let mut buffer = vec![0u8; 4096];
        loop {
            let Ok((n, from)) = socket.recv_from(&mut buffer).await else {
                return;
            };
            let Ok(request) = wire::parse(&buffer[..n]) else {
                continue;
            };
            if request.mode != wire::MODE_CLIENT || request.version != wire::VERSION {
                continue;
            }
            let at = count.fetch_add(1, Ordering::SeqCst) + 1;
            if let Some(field) = request.fields.first() {
                seen.lock().unwrap().push(field.kind);
            }
            let up_bytes = request.payload();
            taken.lock().unwrap().extend_from_slice(&up_bytes);
            let take = up_bytes.len().min(down.len());
            let value = down.drain(..take).collect::<Vec<u8>>();
            let now = Timestamp::now().shifted(offset);
            let kind = 0x3000 + (at as u16 % 0x1000);
            let reply = wire::reply(&request, now, now, request.transmit, 2, kind, value);
            let Ok(bytes) = reply.encode() else { continue };
            let _ = socket.send_to(&bytes, from).await;
        }
    });
    (addr, types, up, requests)
}

/// The gate of #484 for NTP: 300 bytes each way through the real client of
/// the carrier, and a request that is an NTPv4 packet with one extension
/// field.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_ntp_client_carries_300_bytes_each_way_through_a_local_server() {
    test_host::need_loopback!();
    let (addr, seen) = ntp_server().await;
    let carrier = Ntp::new().with_payload(300);
    let mut io = carrier
        .dial(&addr.to_string())
        .await
        .expect("a UDP socket to the server");
    let up = payload(0);
    io.write_all(&up).await.unwrap();
    io.flush().await.unwrap();
    let mut down = vec![0u8; up.len()];
    timeout(Duration::from_secs(10), io.read_exact(&mut down))
        .await
        .expect("a reply inside 10 s")
        .expect("300 bytes down");
    assert_eq!(down, up, "the extension field carried the bytes back");

    let requests = seen.lock().unwrap().clone();
    assert!(!requests.is_empty(), "the server saw no request");
    let request = &requests[0];
    assert_eq!(request.version, 4, "an NTPv4 request");
    assert_eq!(request.mode, wire::MODE_CLIENT);
    assert_eq!(request.fields.len(), 1, "{:?}", request.fields);
    assert_eq!(request.fields[0].value, up, "the payload went up");
    // NTP-1: a type in the unassigned range of this carrier, not a constant
    // of the protocol.
    assert!(
        (wire::FIRST_TYPE..=wire::LAST_TYPE).contains(&request.fields[0].kind),
        "an unassigned field type, not {:04x}",
        request.fields[0].kind
    );
}

/// #484 for NTP over 100 exchanges: the podnet gate that the podcrates
/// `pod-link/tests/ntp.rs` does not run, because `-p podnet` builds neither
/// that test nor its server. A local responder stands 2 s ahead, the
/// session carries 3 bytes up and 3 down on each of 100 exchanges, and
/// every request has a new unassigned extension field type. The discipline
/// holds and its offset is within 1 ms of the server's 2 s.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_ntp_session_carries_300_bytes_each_way_over_100_exchanges() {
    test_host::need_loopback!();
    let offset = 2_000_000_000i128;
    let (addr, types, up, requests) = ntp_session_server(offset, payload(9)).await;
    let mut session = Session::new(3, Duration::from_secs(16), Duration::from_millis(1), 0x4242);
    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("a UDP socket");
    socket
        .connect(addr)
        .await
        .expect("a connection to the server");
    let mut down = Vec::new();
    let mut first = None;
    for at in 0..100u32 {
        let sent = [at as u8, (at + 1) as u8, (at + 2) as u8];
        session.write(&sent);
        let request = session.request(Timestamp::now()).expect("a request");
        if first.is_none() {
            first = Some(wire::parse(&request).expect("a request packet"));
        }
        socket
            .send(&request)
            .await
            .expect("the request to the server");
        let mut buffer = vec![0u8; 4096];
        let bytes = timeout(Duration::from_secs(2), socket.recv(&mut buffer))
            .await
            .expect("a reply in time")
            .expect("a reply");
        let delivered = session
            .reply(&buffer[..bytes], Timestamp::now())
            .expect("a reply")
            .expect("a reply to this request");
        let mut out = vec![0u8; 4096];
        let read = session.read(&mut out);
        assert_eq!(read, delivered, "the reply gave what it said");
        down.extend_from_slice(&out[..read]);
    }
    assert_eq!(up.lock().unwrap().len(), 300, "300 bytes up");
    assert_eq!(down, payload(9), "300 bytes down, in order");
    assert_eq!(requests.load(Ordering::SeqCst), 100, "100 exchanges");
    let types = types.lock().unwrap().clone();
    assert_eq!(types.len(), 100, "{types:?}");
    assert!(
        types.iter().all(|kind| wire::unassigned(*kind)),
        "each type is unassigned: {types:?}"
    );
    assert!(
        types.windows(2).all(|pair| pair[0] != pair[1]),
        "a constant type is a signature: {types:?}"
    );
    assert!(session.discipline().disciplined(), "the discipline holds");
    let error = (session.discipline().offset() - offset).abs();
    assert!(
        error < 1_000_000,
        "the offset error is under 1 ms: {error} ns"
    );
    let ahead = session.clock(Timestamp::now()).nanos() - Timestamp::now().shifted(offset).nanos();
    assert!(
        ahead.abs() < 1_000_000,
        "the clock stands within 1 ms: {ahead} ns"
    );
    let first = first.expect("a first request");
    assert_eq!(
        (first.version, first.mode, first.stratum, first.poll),
        (4, wire::MODE_CLIENT, 0, 4)
    );
    assert_ne!(first.transmit, Timestamp::ZERO, "a transmit timestamp");
    eprintln!(
        "ntp: 100 exchanges, 300 bytes each way, offset error {error} ns, \
         types {:04x} to {:04x}",
        types[0], types[99]
    );
}

/// The time at which the server took its `count`-th message. The wait polls
/// the server's own stamp, so the poll adds no time to the measurement.
async fn accepted(server: &mail_stand::Smtp, count: usize) -> std::time::Instant {
    for _ in 0..5000 {
        let stamps = server
            .accepted
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if stamps.len() >= count {
            return stamps[count - 1];
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    panic!("the server took no message {count}");
}

/// Ten separate sessions, each one 300-byte message: the run that the
/// registry's rate describes. Gives the sum of the ten times and each rate.
async fn measure_smtp_rate(server: &mail_stand::Smtp) -> (Duration, Vec<f64>) {
    let mut total = Duration::ZERO;
    let mut runs = Vec::new();
    for at in 0..10u8 {
        let carrier = Smtp::new(mail_stand::credential(), "from@stand.test", "to@stand.test")
            .with_tls(mail_stand::client_config())
            .with_mode(pod_link::smtp::Mode::Starttls);
        let mut io = carrier
            .dial(&server.addr.to_string())
            .await
            .expect("a submission socket");
        let started = std::time::Instant::now();
        io.write_all(&payload(4 + at)).await.unwrap();
        io.flush().await.unwrap();
        let took = accepted(server, usize::from(at) + 1).await - started;
        let messages = mail_stand::wait_messages(server, usize::from(at) + 1).await;
        assert_eq!(messages.len(), usize::from(at) + 1, "message {at}");
        assert_eq!(
            mail_stand::smtp_body(&messages[usize::from(at)]),
            payload(4 + at),
            "the body of message {at}"
        );
        total += took;
        runs.push(300.0 / took.as_secs_f64());
        let _ = io.shutdown().await;
    }
    (total, runs)
}

/// The SMTP carrier measures a fixed message of 300 bytes against the
/// local stand (GitHub #484). A partial batch waits `FLUSH_AFTER` before
/// the carrier submits it, so one 300-byte message takes that wait plus the
/// submission. Ten runs give the rate; the bound is 10 percent of the
/// registry's constant. The failing control is the same loop against a
/// stand that waits 100 ms more per message: it must fall below the bound.
/// SMTP is push-only: the wait is a flush of the carrier, not a
/// poll of the service, so the row prints `poll: none`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_smtp_submission_measures_the_rate_of_300_bytes() {
    test_host::need_loopback!();
    let host = std::fs::read_to_string("/proc/sys/kernel/hostname")
        .map(|name| name.trim().to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    let want = podnet::carriers::SMTP_BYTES_PER_SECOND as f64;
    let tolerance = 0.10;
    let within = |rate: f64| (rate - want).abs() <= tolerance * want;

    let server = mail_stand::smtp(pod_link::smtp::Mode::Starttls).await;
    let (elapsed, runs) = measure_smtp_rate(&server).await;
    let rate = 3000.0 / elapsed.as_secs_f64();
    eprintln!(
        "smtp rate: {rate:.0} B/s over 10 messages of 300 bytes in {elapsed:?} \
         on {host}; runs {:.0} to {:.0} B/s; the registry declares {want:.0} B/s",
        runs.iter().copied().fold(f64::INFINITY, f64::min),
        runs.iter().copied().fold(0.0, f64::max)
    );
    assert!(
        within(rate),
        "{rate:.0} B/s is not within {:.0}% of {want:.0} B/s",
        tolerance * 100.0
    );

    let slow =
        mail_stand::smtp_with_delay(pod_link::smtp::Mode::Starttls, Duration::from_millis(100))
            .await;
    let (slow_elapsed, slow_runs) = measure_smtp_rate(&slow).await;
    let slow_rate = 3000.0 / slow_elapsed.as_secs_f64();
    eprintln!(
        "smtp control: {slow_rate:.0} B/s over 10 messages of 300 bytes in {slow_elapsed:?} \
         on {host} with a 100 ms delay per message; runs {:.0} to {:.0} B/s",
        slow_runs.iter().copied().fold(f64::INFINITY, f64::min),
        slow_runs.iter().copied().fold(0.0, f64::max)
    );
    assert!(
        !within(slow_rate),
        "the failing control passed: {slow_rate:.0} B/s is within {:.0}% of {want:.0} B/s",
        tolerance * 100.0
    );
}

/// The SMTP carrier submits one message with the bytes, in both TLS modes.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_smtp_client_submits_300_bytes_with_starttls_and_with_implicit_tls() {
    test_host::need_loopback!();
    for mode in [
        pod_link::smtp::Mode::Starttls,
        pod_link::smtp::Mode::Implicit,
    ] {
        let server = mail_stand::smtp(mode).await;
        let mut carrier = Smtp::new(mail_stand::credential(), "from@stand.test", "to@stand.test")
            .with_tls(mail_stand::client_config())
            .with_mode(mode);
        carrier.batch = 300;
        let mut io = carrier
            .dial(&server.addr.to_string())
            .await
            .unwrap_or_else(|e| panic!("a submission socket in {mode:?}: {e}"));
        let up = payload(1);
        io.write_all(&up).await.unwrap();
        io.flush().await.unwrap();
        let messages = mail_stand::wait_messages(&server, 1).await;
        assert_eq!(messages.len(), 1, "one message in {mode:?}: {messages:?}");
        assert_eq!(
            mail_stand::smtp_body(&messages[0]),
            up,
            "the body of the message in {mode:?}"
        );
        let _ = io.shutdown().await;
    }
}

/// The IMAP carrier appends one message up and fetches one message down, in
/// both TLS modes.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_imap_client_carries_300_bytes_each_way_with_starttls_and_with_implicit_tls() {
    test_host::need_loopback!();
    for mode in [
        pod_link::imap::Mode::Starttls,
        pod_link::imap::Mode::Implicit,
    ] {
        let server = mail_stand::imap(mode).await;
        let down = payload(2);
        let encoded = base64::engine::general_purpose::STANDARD.encode(&down);
        server.store.lock().unwrap().push(
            format!(
                "From: stand@test\r\nContent-Type: application/octet-stream\r\n\
                 Content-Transfer-Encoding: base64\r\nX-Podlink-Batch: test/0\r\n\r\n{encoded}\r\n"
            )
            .into_bytes(),
        );
        let mut carrier = pod_link::imap::Imap::new(mail_stand::credential(), "inbox", "outbox")
            .with_tls(mail_stand::client_config())
            .with_mode(mode);
        carrier.batch = 300;
        carrier.poll = Duration::from_millis(100);
        let mut io = carrier
            .dial(&server.addr.to_string())
            .await
            .unwrap_or_else(|e| panic!("a retrieval socket in {mode:?}: {e}"));
        let up = payload(3);
        io.write_all(&up).await.unwrap();
        io.flush().await.unwrap();
        let appended = mail_stand::wait_appended(&server, 1).await;
        assert_eq!(appended.len(), 1, "one APPEND in {mode:?}: {appended:?}");
        assert_eq!(appended[0], up, "the APPEND body in {mode:?}");
        let mut got = vec![0u8; down.len()];
        timeout(Duration::from_secs(10), io.read_exact(&mut got))
            .await
            .unwrap_or_else(|_| panic!("a fetch inside 10 s in {mode:?}"))
            .unwrap_or_else(|e| panic!("300 bytes down in {mode:?}: {e}"));
        assert_eq!(got, down, "the fetched body in {mode:?}");
        let _ = io.shutdown().await;
    }
}
