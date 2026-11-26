//! The DNS simplex carrier of the link (T-278): query names up and TXT
//! answers down, the ARQ of `pod-link` and TLS 1.3 above it with the
//! Ed25519 key of each end pinned. The listener is a local authoritative
//! server of the zone on the UDP loopback. The tests with no socket run on
//! each host. A test that needs the loopback prints `skipping, REASON` on a
//! host that denies it, and does not fail.
#![cfg(feature = "dns")]

#[path = "test_host/mod.rs"]
mod test_host;

#[path = "link_peer/mod.rs"]
mod link_peer;

use link_peer::{install, list_of, Case};
use pod_link::{Carrier, Cost};
use podnet::exit::code_of;
use podnet::identity::{AllowList, Identity};
use podnet::link::Door;
use podnet::link_dns;
use podnet::link_keyed::{self, Tune};

const ZONE: &str = "t.example";

fn address_of(key: &str, _: &str) -> String {
    format!("{key}@{ZONE}")
}

/// The tune of a listener of the zone.
fn zone() -> Tune {
    Tune {
        dns_zone: Some(ZONE.into()),
        ..Tune::default()
    }
}

/// The tune of a dialer that sends its queries to the listener `door`.
fn toward(door: &Door) -> Tune {
    Tune {
        dns_server: Some(door.address().parse().unwrap()),
        ..Tune::default()
    }
}

async fn listen(me: &Identity, allow: &AllowList) -> Door {
    link_keyed::listen("dns", &zone(), me, allow, "127.0.0.1:0")
        .await
        .unwrap()
}

/// The case of the dialer of `door`.
fn case(door: &Door) -> Case {
    Case {
        name: "dns",
        tune: toward(door),
        address_of,
    }
}

// ---- no socket ----

#[test]
fn the_carrier_is_a_stream_of_the_highest_overhead() {
    let dir = tempfile::tempdir().unwrap();
    let me = install(dir.path(), "me");
    let carrier = link_dns::carrier(&me, &AllowList::default(), &zone());
    assert_eq!(carrier.name(), "dns");
    assert_eq!(carrier.zone, ZONE);
    let facts = carrier.facts();
    assert!(facts.reliable && facts.ordered && facts.duplex);
    assert_eq!(facts.cost, Cost::Highest);
    assert_eq!(carrier.alpn, pod_link::tcp::ALPN);
}

#[test]
fn the_registry_row_has_no_key_and_refuses_a_dial() {
    link_peer::the_registry_row_refuses_a_dial("dns", "dns", "x@t.example");
}

#[tokio::test]
async fn a_listener_with_no_zone_and_an_address_with_no_key_exit_64() {
    let dir = tempfile::tempdir().unwrap();
    let me = install(dir.path(), "me");
    let e = link_keyed::listen(
        "dns",
        &Tune::default(),
        &me,
        &list_of(&[&me]),
        "127.0.0.1:0",
    )
    .await
    .err()
    .unwrap();
    assert_eq!(code_of(&e), 64, "{e:#}");
    assert!(format!("{e:#}").contains("--dns-zone"), "{e:#}");
    let to = Case {
        name: "dns",
        tune: Tune {
            dns_server: Some("127.0.0.1:53".parse().unwrap()),
            ..Tune::default()
        },
        address_of,
    };
    link_peer::no_key_exits_64(&to, "t.example").await;
    link_peer::no_key_exits_64(&to, "nokey@t.example").await;
}

#[test]
fn the_dns_flags_are_for_the_dns_carrier() {
    let dir = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| link_peer::podnet(dir.path(), "s", args).output().unwrap();
    let out = run(&[
        "link",
        "listen",
        "/tmp/x",
        "--dns-zone",
        ZONE,
        "--allow",
        "/nonexistent",
    ]);
    assert_eq!(out.status.code(), Some(64), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("dns carrier"),
        "{out:?}"
    );
    // The flags pass: the next step reads the allow list (exit 66).
    let out = run(&[
        "link",
        "listen",
        "127.0.0.1:0",
        "--carrier",
        "dns",
        "--dns-zone",
        ZONE,
        "--allow",
        "/nonexistent",
    ]);
    assert_eq!(out.status.code(), Some(66), "{out:?}");
}

// ---- UDP loopback: a local authoritative server ----

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_link_crosses_dns_and_each_end_learns_the_key_of_the_other() {
    test_host::need_loopback!();
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (install(dir.path(), "a"), install(dir.path(), "b"));
    let mut door = listen(&b, &list_of(&[&a])).await;
    let to = address_of(&b.fingerprint(), "");
    let (dialed, accepted) = case(&door).meet(&a, list_of(&[&b]), &to, &mut door).await;
    let ((near, seen_by_a), (far, seen_by_b)) = (dialed.unwrap(), accepted.unwrap());
    assert_eq!(seen_by_a, b.public());
    assert_eq!(seen_by_b, Some(a.public()));
    link_peer::echo(&near, &far, &[7u8; 4096]).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_dialer_that_the_listener_does_not_allow_exits_77_and_the_door_stays_open() {
    test_host::need_loopback!();
    let dir = tempfile::tempdir().unwrap();
    let (a, b, c) = (
        install(dir.path(), "a"),
        install(dir.path(), "b"),
        install(dir.path(), "c"),
    );
    let mut door = listen(&b, &list_of(&[&a])).await;
    let to = address_of(&b.fingerprint(), "");
    let (dialed, accepted) = case(&door).meet(&c, list_of(&[&b]), &to, &mut door).await;
    let refused = dialed.expect_err("the dial is refused");
    assert_eq!(code_of(&refused), 77, "{refused:#}");
    // T-557: the listener gets the refusal too, with the exit 77.
    let accepted = accepted.expect_err("the listener takes no peer");
    assert_eq!(code_of(&accepted), 77, "{accepted:#}");
    let (dialed, accepted) = case(&door).meet(&a, list_of(&[&b]), &to, &mut door).await;
    dialed.unwrap();
    assert_eq!(accepted.unwrap().1, Some(a.public()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_listener_with_another_key_than_the_pinned_one_exits_77() {
    test_host::need_loopback!();
    let dir = tempfile::tempdir().unwrap();
    let (a, b, c) = (
        install(dir.path(), "a"),
        install(dir.path(), "b"),
        install(dir.path(), "c"),
    );
    let mut door = listen(&b, &list_of(&[&a])).await;
    let to = address_of(&c.fingerprint(), "");
    let (dialed, accepted) = case(&door)
        .meet(&a, list_of(&[&b, &c]), &to, &mut door)
        .await;
    let e = dialed.err().unwrap();
    assert_eq!(code_of(&e), 77, "{e:#}");
    // T-557: the listener gets `KeyRefused`, with the exit 77.
    let Err(e) = accepted else {
        panic!("the listener takes no peer")
    };
    assert_eq!(code_of(&e), 77, "{e:#}");
}

// ---- the commands ----

#[test]
fn link_dial_check_meets_link_listen_over_dns_and_a_stranger_exits_77() {
    test_host::need_loopback!();
    link_peer::cli_check(
        "dns",
        "127.0.0.1:0",
        &["--dns-zone", ZONE],
        address_of,
        &["--dns-server", "{listen}"],
    );
}

// ---- T-281: the window, the SERVFAIL backoff and the rate ceiling ----

/// A UDP relay between a dialer and the listener `to`: it counts each query,
/// keeps the time of each, and answers each `servfail_every`th query with
/// SERVFAIL itself (0: none). Each other query goes to the listener, and its
/// answer back to the dialer.
struct Relay {
    addr: std::net::SocketAddr,
    starts: std::sync::Arc<std::sync::Mutex<Vec<std::time::Instant>>>,
    servfails: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

async fn relay(to: std::net::SocketAddr, servfail_every: usize) -> Relay {
    use std::sync::atomic::Ordering;
    let front = std::sync::Arc::new(tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap());
    let back = std::sync::Arc::new(tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap());
    back.connect(to).await.unwrap();
    let addr = front.local_addr().unwrap();
    let starts = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let servfails = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let client = std::sync::Arc::new(std::sync::Mutex::new(None));
    let (f, b, c) = (front.clone(), back.clone(), client.clone());
    tokio::spawn(async move {
        let mut buf = vec![0u8; 4096];
        while let Ok(n) = b.recv(&mut buf).await {
            let to = *c.lock().unwrap();
            if let Some(to) = to {
                let _ = f.send_to(&buf[..n], to).await;
            }
        }
    });
    let (s, sf) = (starts.clone(), servfails.clone());
    tokio::spawn(async move {
        let mut buf = vec![0u8; 4096];
        let mut count = 0usize;
        while let Ok((n, from)) = front.recv_from(&mut buf).await {
            *client.lock().unwrap() = Some(from);
            s.lock().unwrap().push(std::time::Instant::now());
            count += 1;
            if servfail_every > 0 && count.is_multiple_of(servfail_every) && n >= 12 {
                // The query back as an answer: QR, the opcode and RD kept,
                // RCODE 2 (SERVFAIL), no record.
                let mut answer = buf[..n].to_vec();
                answer[2] |= 0x80;
                answer[3] = (answer[3] & 0xf0) | 2;
                sf.fetch_add(1, Ordering::Relaxed);
                let _ = front.send_to(&answer, from).await;
                continue;
            }
            let _ = back.send(&buf[..n]).await;
        }
    });
    Relay {
        addr,
        starts,
        servfails,
    }
}

/// A dial of `to` through `server` with the rate `qps` and `in_flight`
/// queries at once (the settings of `pod-link`; podnet gives 20 and 8).
async fn dial_with(
    me: &Identity,
    allow: &AllowList,
    to: &str,
    server: std::net::SocketAddr,
    qps: f64,
    in_flight: usize,
) -> anyhow::Result<podnet::link::Link> {
    let tune = Tune {
        dns_server: Some(server),
        ..Tune::default()
    };
    let mut carrier = link_dns::carrier(me, allow, &tune);
    carrier.dns.queries_per_second = qps;
    carrier.dns.in_flight = in_flight;
    let (link, _) = link_dns::dial(&carrier, allow, to, podnet::link::Settings::default()).await?;
    Ok(link)
}

/// T-281: 256 KiB go each way over a window of 32 queries while the path
/// answers one query in ten with SERVFAIL: the dialer sends each lost
/// datagram again, and the stream is whole. Then a dialer with a rate of
/// 40 queries a second starts no more than its rate over a transfer.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_window_of_queries_carries_256_kib_through_servfail_and_keeps_its_rate() {
    test_host::need_loopback!();
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (install(dir.path(), "a"), install(dir.path(), "b"));
    let to = address_of(&b.fingerprint(), "");
    // (the SERVFAIL of each Nth query, the rate, the window, the bytes)
    let cases: [(usize, f64, usize, usize); 2] =
        [(10, 4000.0, 32, 256 * 1024), (0, 40.0, 32, 16 * 1024)];
    for (every, qps, window, bytes) in cases {
        let mut door = listen(&b, &list_of(&[&a])).await;
        let path = relay(door.address().parse().unwrap(), every).await;
        let started = std::time::Instant::now();
        let allow = list_of(&[&b]);
        let (dialed, accepted) = tokio::join!(
            dial_with(&a, &allow, &to, path.addr, qps, window),
            link_peer::quiet(&mut door)
        );
        let near = dialed.unwrap();
        let (far, _) = accepted.unwrap();
        link_peer::echo(&near, &far, &vec![5u8; bytes]).await;
        let elapsed = started.elapsed().as_secs_f64();
        let starts = path.starts.lock().unwrap().clone();
        let servfails = path.servfails.load(std::sync::atomic::Ordering::Relaxed);
        if every > 0 {
            assert!(servfails > 10, "{servfails} SERVFAIL answers");
        } else {
            // The rate over the transfer, with one second of burst.
            let ceiling = qps * (elapsed + 1.0);
            assert!(
                (starts.len() as f64) <= ceiling,
                "{} queries in {elapsed:.1} s at {qps} a second",
                starts.len()
            );
            assert!(elapsed > 1.0, "the transfer took {elapsed:.2} s");
        }
        eprintln!(
            "servfail 1/{every}, rate {qps}: {} queries, {servfails} SERVFAIL, {bytes} bytes each way in {elapsed:.2} s",
            starts.len()
        );
    }
}
