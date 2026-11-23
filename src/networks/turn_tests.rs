//! `turn://` and `turns://` (T-321): the spec, the account, and the
//! allocation, the channel and the refresh against the stand-in server of
//! `turn_mock.rs` on the loopback.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::sync::Arc;
use std::time::Duration;

use pod_turn::stun::{ALLOCATE, CHANNEL_BIND, REFRESH};
use pod_turn::Limits;

use super::turn::{self, Account};
use super::turn_mock::{self as mock, Mock};
use super::*;
use crate::network::{recv_from, send_to, BoxFuture, Connected, Datagrams};

const WAIT: Duration = Duration::from_secs(10);

/// Short waits, so that a lost datagram or a dead server ends a test soon.
fn fast() -> Limits {
    Limits {
        rto: Duration::from_millis(100),
        retransmits: 3,
        request_timeout: Duration::from_secs(3),
        ..Limits::default()
    }
}

async fn up_with(mock: &Mock, account: Account, limits: Limits) -> Result<Link, Failure> {
    up_with_rate(mock, account, limits, None).await
}

async fn up_with_rate(
    mock: &Mock,
    account: Account,
    limits: Limits,
    rate_limit: Option<u64>,
) -> Result<Link, Failure> {
    let spec = turn::parse(&format!("turn://{}", mock.addr)).unwrap();
    turn::up(&spec, account, None, limits, rate_limit).await
}

/// The TLS face of the stand, `turns://`: the client trusts the CA of the
/// stand alone (GitHub #485).
async fn up_tls(mock: &Mock, limits: Limits) -> Result<Link, Failure> {
    let spec = turn::parse(&format!("turns://{}", mock.tls)).unwrap();
    turn::up_with_tls(
        &spec,
        Account::new(mock::USER, mock::PASSWORD),
        None,
        limits,
        None,
        mock::client_context(),
    )
    .await
}

/// An allocation of the TLS face that waits for a peer, the accept
/// direction of RFC 6062 (GitHub #485).
async fn listen_tls(mock: &Mock, limits: Limits) -> Result<turn::Accepted, Failure> {
    let spec = turn::parse(&format!("turns://{}", mock.tls)).unwrap();
    turn::listen(
        &spec,
        Account::new(mock::USER, mock::PASSWORD),
        None,
        limits,
        None,
        mock::client_context(),
    )
    .await
}

/// Wait until the stand-in has seen `want` control connections of a live
/// allocation end.
async fn until_closed(mock: &Mock, want: u32) {
    for _ in 0..200 {
        if mock.closed() >= want {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("{want} closes not seen: {:?}", mock.seen());
}

async fn up(mock: &Mock) -> Link {
    up_with(mock, Account::new(mock::USER, mock::PASSWORD), fast())
        .await
        .unwrap_or_else(|e| panic!("{e}"))
}

fn dial_of(link: &Link) -> Arc<dyn Dial> {
    link.way.as_ref().expect("a way").dial.clone()
}

/// Wait until `count` of the server reaches `want`.
async fn until(mock: &Mock, method: u16, code: u16, want: usize) {
    for _ in 0..200 {
        if mock.count(method, code) >= want {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("{want} x ({method}, {code}) not seen: {:?}", mock.seen());
}

async fn recv(socket: &dyn Datagrams) -> std::io::Result<(Vec<u8>, std::net::SocketAddr)> {
    let mut buf = [0u8; 2048];
    let (n, from) = tokio::time::timeout(WAIT, recv_from(socket, &mut buf))
        .await
        .expect("a datagram or an error in time")?;
    Ok((buf[..n].to_vec(), from))
}

#[test]
fn network_spec_turn_forms_are_taken() {
    for (spec, label, tls) in [
        (
            "turn://turn.example:3478",
            "turn://turn.example:3478",
            false,
        ),
        ("turn://turn.example", "turn://turn.example:3478", false),
        ("turns://turn.example", "turns://turn.example:5349", true),
        ("turns://turn.example:443", "turns://turn.example:443", true),
        ("turn://203.0.113.9:3479", "turn://203.0.113.9:3479", false),
        (
            "turn://[2001:db8::1]:3478",
            "turn://[2001:db8::1]:3478",
            false,
        ),
        ("turns://[2001:db8::1]", "turns://[2001:db8::1]:5349", true),
    ] {
        let network = parse(spec).unwrap_or_else(|e| panic!("{spec}: {e}"));
        assert_eq!(network.scheme(), "turn", "{spec}");
        assert_eq!(network.label(), label, "{spec}");
        assert_eq!(network.turn().map(|t| t.1), Some(tls), "{spec}");
        assert!(network.caps().contains(Caps::EGRESS_UDP), "{spec}");
        assert!(network.caps().contains(Caps::EGRESS_TCP), "{spec}");
    }
    // The TLS form needs the TCP connection to the server first.
    assert_eq!(parse("turns://turn.example").unwrap().checks().len(), 1);
    assert!(parse("turn://turn.example").unwrap().checks().is_empty());
}

#[test]
fn network_spec_turn_a_bad_spec_is_one_usage_line_without_a_secret() {
    for spec in [
        "turn://",
        "turn://:3478",
        "turn://host:0",
        "turn://host:70000",
        "turn://host:port",
        "turn://::1:3478",
        "turn://[::1",
        "turn://[]:3478",
        "turn://user:secret@host:3478",
        "turns://user:pa/ss-secret@host",
        "turn://host:3478?password=secret",
        "turn://host:3478/secret",
        "turn://host#secret",
    ] {
        let err = parse(spec)
            .err()
            .unwrap_or_else(|| panic!("{spec:?} parsed"));
        assert_eq!(err.class, Class::Usage, "{spec:?}: {err}");
        let text = err.text();
        assert!(!text.contains('\n'), "{text}");
        assert!(
            !text.contains("secret") && !text.contains("pa/ss"),
            "{spec}: {text}"
        );
    }
    let err = parse("turn://user:secret@host").err().unwrap();
    assert!(err.text().contains("PODNET_TURN_USER"), "{err}");
}

#[test]
fn turn_account_comes_from_the_variables_or_from_a_file() {
    let vars = |pairs: &[(&str, &str)]| {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |name: &str| map.get(name).cloned()
    };
    let got = turn::account(&vars(&[
        (turn::USER_VARIABLE, "alice"),
        (turn::PASSWORD_VARIABLE, "pw1"),
        (turn::REALM_VARIABLE, "r.example"),
    ]))
    .unwrap();
    assert_eq!(got.user, "alice");
    assert_eq!(got.realm.as_deref(), Some("r.example"));
    // A file: one line; a trailing newline is not part of the password.
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("pw");
    std::fs::write(&file, "from-file\r\n").unwrap();
    let path = file.to_str().unwrap();
    let got = turn::account(&vars(&[
        (turn::USER_VARIABLE, "alice"),
        (turn::PASSWORD_FILE_VARIABLE, path),
    ]))
    .unwrap();
    assert_eq!(got.password, "from-file");
    assert!(!format!("{got:?}").contains("from-file"));
    // Each fault names the variables, and none holds a value.
    let faults = [
        (vars(&[]), turn::USER_VARIABLE),
        (
            vars(&[(turn::USER_VARIABLE, "alice")]),
            turn::PASSWORD_FILE_VARIABLE,
        ),
        (
            vars(&[
                (turn::USER_VARIABLE, "alice"),
                (turn::PASSWORD_VARIABLE, "never-shown-pw"),
                (turn::PASSWORD_FILE_VARIABLE, path),
            ]),
            "not both",
        ),
        (
            vars(&[
                (turn::USER_VARIABLE, "alice"),
                (turn::PASSWORD_FILE_VARIABLE, "/no/such/turn-pw"),
            ]),
            "/no/such/turn-pw",
        ),
    ];
    for (get, word) in faults {
        let err = turn::account(&get).expect_err("a fault");
        assert_eq!(err.class, Class::Usage);
        assert!(err.text().contains(word), "{word}: {err}");
        assert!(!err.text().contains("never-shown-pw"), "{err}");
    }
    std::fs::write(&file, "\n").unwrap();
    let err = turn::account(&vars(&[
        (turn::USER_VARIABLE, "a"),
        (turn::PASSWORD_FILE_VARIABLE, path),
    ]))
    .err()
    .unwrap();
    assert!(err.text().contains("no password"), "{err}");
}

#[test]
fn turn_account_debug_shows_the_user_alone() {
    let account = Account::new("alice", "very-secret-password");
    let shown = format!("{account:?}");
    assert!(
        shown.contains("alice") && !shown.contains("very-secret"),
        "{shown}"
    );
}

/// The probe at `up` makes one allocation and ends it; the status shows the
/// transport and no credential.
#[tokio::test]
async fn turn_up_allocates_once_ends_it_and_shows_no_credential() {
    crate::test_host::need_loopback!();
    let mock = mock::start().await;
    let link = up(&mock).await;
    assert_eq!(mock.seen(), [(ALLOCATE, 401), (ALLOCATE, 0), (REFRESH, 0)]);
    assert_eq!(link.status["scheme"], "turn");
    assert_eq!(link.status["transport"], "udp");
    assert_eq!(link.status["spec"], format!("turn://{}", mock.addr));
    let shown = link.status.to_string();
    assert!(
        !shown.contains(mock::PASSWORD) && !shown.contains(mock::USER),
        "{shown}"
    );
}

/// A datagram goes to a peer and the answer comes back; the second datagram
/// to the same peer uses the same channel; the socket ends its allocation
/// when it is dropped.
#[tokio::test]
async fn turn_datagrams_carry_a_datagram_to_a_peer_and_back() {
    crate::test_host::need_loopback!();
    let (mock, peer) = (mock::start().await, mock::echo_peer().await);
    let dial = dial_of(&up(&mock).await);
    let socket = dial.datagrams().await.unwrap();
    assert!(socket.local_addr().unwrap().ip().is_loopback());
    for text in [&b"one"[..], b"two"] {
        send_to(&*socket, text, peer).await.unwrap();
        let (body, from) = recv(&*socket).await.unwrap();
        assert_eq!(
            (body.as_slice(), from),
            ([&b"echo:"[..], text].concat().as_slice(), peer)
        );
    }
    assert_eq!(mock.count(CHANNEL_BIND, 0), 1, "one channel for one peer");
    assert_eq!(mock.control.lock().unwrap().channel_data, 2);
    drop(socket);
    until(&mock, REFRESH, 0, 2).await;
}

/// A 438 answer is answered with the fresh nonce, and the datagram arrives.
#[tokio::test]
async fn turn_a_stale_nonce_does_not_lose_the_datagram() {
    crate::test_host::need_loopback!();
    let (mock, peer) = (mock::start().await, mock::echo_peer().await);
    let dial = dial_of(&up(&mock).await);
    let socket = dial.datagrams().await.unwrap();
    mock.control.lock().unwrap().stale_next = 1;
    send_to(&*socket, b"ping", peer).await.unwrap();
    let (body, _) = recv(&*socket).await.unwrap();
    assert_eq!(body, b"echo:ping");
    assert_eq!(mock.count(CHANNEL_BIND, 438), 1);
    assert_eq!(mock.count(CHANNEL_BIND, 0), 1);
}

/// Each datagram socket has its own allocation.
#[tokio::test]
async fn turn_each_socket_has_its_own_allocation() {
    crate::test_host::need_loopback!();
    let mock = mock::start().await;
    let dial = dial_of(&up(&mock).await);
    let (a, b) = (
        dial.datagrams().await.unwrap(),
        dial.datagrams().await.unwrap(),
    );
    assert_ne!(a.local_addr().unwrap(), b.local_addr().unwrap());
    assert_eq!(mock.count(ALLOCATE, 0), 3, "the probe and two sockets");
}

/// A refresh that fails ends the socket with the stage `refresh`, and the
/// socket does not ask again.
#[tokio::test]
async fn turn_a_refresh_failure_closes_the_socket_with_a_named_stage() {
    crate::test_host::need_loopback!();
    for code in [437, 508] {
        let mock = mock::start().await;
        // A lifetime of 2 s: the client refreshes at 1 s.
        let limits = Limits {
            lifetime: Duration::from_secs(2),
            ..fast()
        };
        let link = up_with(&mock, Account::new(mock::USER, mock::PASSWORD), limits)
            .await
            .unwrap();
        let socket = dial_of(&link).datagrams().await.unwrap();
        mock.control.lock().unwrap().refresh_code = Some(code);
        let err = recv(&*socket).await.expect_err("the socket ended");
        let text = err.to_string();
        assert!(
            text.contains("refresh") && text.contains(&code.to_string()),
            "{text}"
        );
        assert!(text.contains(&format!("turn://{}", mock.addr)), "{text}");
        assert!(!text.contains(mock::PASSWORD), "{text}");
        // The socket stays ended: a send fails with the same error.
        assert!(send_to(&*socket, b"x", "127.0.0.1:9".parse().unwrap())
            .await
            .is_err());
        tokio::time::sleep(Duration::from_millis(1500)).await;
        assert_eq!(mock.count(REFRESH, code), 1, "code {code}: no second try");
    }
}

/// A channel that the server refuses ends the socket with the stage `bind`.
#[tokio::test]
async fn turn_a_bind_failure_closes_the_socket_with_a_named_stage() {
    crate::test_host::need_loopback!();
    let (mock, peer) = (mock::start().await, mock::echo_peer().await);
    let dial = dial_of(&up(&mock).await);
    let socket = dial.datagrams().await.unwrap();
    mock.control.lock().unwrap().bind_code = Some(403);
    send_to(&*socket, b"x", peer).await.unwrap();
    let text = recv(&*socket).await.expect_err("ended").to_string();
    assert!(text.contains("bind") && text.contains("403"), "{text}");
}

/// A wrong password is refused at `allocate`; the text holds neither
/// password.
#[tokio::test]
async fn turn_a_wrong_password_names_the_allocate_stage() {
    crate::test_host::need_loopback!();
    let mock = mock::start().await;
    let err = up_with(
        &mock,
        Account::new(mock::USER, "wrong-password-xyz"),
        fast(),
    )
    .await
    .expect_err("a refusal");
    assert_eq!(err.class, Class::Refused);
    let text = err.text();
    assert!(text.contains("allocate") && text.contains("401"), "{text}");
    assert!(
        !text.contains("wrong-password-xyz") && !text.contains(mock::PASSWORD),
        "{text}"
    );
}

/// A server that does not answer is a timeout at `allocate`.
#[tokio::test]
async fn turn_a_silent_server_names_the_allocate_stage() {
    crate::test_host::need_loopback!();
    let silent = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let mock = Mock {
        addr: silent.local_addr().unwrap(),
        tcp: silent.local_addr().unwrap(),
        tls: silent.local_addr().unwrap(),
        control: Arc::default(),
    };
    let limits = Limits {
        rto: Duration::from_millis(50),
        retransmits: 1,
        request_timeout: Duration::from_millis(300),
        ..Limits::default()
    };
    let err = up_with(&mock, Account::new(mock::USER, mock::PASSWORD), limits)
        .await
        .expect_err("no answer");
    assert!(err.text().contains("allocate"), "{err}");
}

/// A TCP stream goes through a TCP allocation of RFC 6062: the Connect, its
/// ConnectionBind, and the bare bytes of each direction. The allocation is
/// refreshed before its end (TURN-2).
#[tokio::test]
async fn turn_a_tcp_stream_goes_through_an_rfc6062_allocation() {
    crate::test_host::need_loopback!();
    let (mock, peer) = (mock::start().await, mock::tcp_echo_peer().await);
    // A lifetime of 2 s: the stream refreshes at 1 s.
    let limits = Limits {
        lifetime: Duration::from_secs(2),
        refresh_before: Duration::from_secs(1),
        ..fast()
    };
    let link = up_with(&mock, Account::new(mock::USER, mock::PASSWORD), limits)
        .await
        .unwrap();
    let dial = dial_of(&link);
    let before = mock.count(REFRESH, 0);
    let opened = dial
        .dial(&Target::Addr(peer), tokio::time::Instant::now() + WAIT)
        .await
        .unwrap();
    assert_eq!(opened.peer, Some(peer));
    let mut io = opened.io;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    for text in [&b"first"[..], b"second"] {
        io.write_all(text).await.unwrap();
        let mut back = vec![0u8; 5 + text.len()];
        io.read_exact(&mut back).await.unwrap();
        assert_eq!(back, [&b"echo:"[..], text].concat());
    }
    // The keeper of the allocation refreshes it before the end.
    until(&mock, REFRESH, 0, before + 1).await;
    let shown = link.status.to_string();
    assert!(
        !shown.contains(mock::PASSWORD) && !shown.contains(mock::USER),
        "{shown}"
    );
    drop(io);
}

/// GitHub #485: the same RFC 6062 stream over `turns://`, with TURN over
/// TLS on TCP. The stream refreshes the allocation before its end, and the
/// close sends a Refresh with lifetime 0.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn turn_a_tls_tcp_stream_goes_through_an_rfc6062_allocation() {
    crate::test_host::need_loopback!();
    let (mock, peer) = (mock::start().await, mock::tcp_echo_peer().await);
    let limits = Limits {
        lifetime: Duration::from_secs(2),
        refresh_before: Duration::from_secs(1),
        ..fast()
    };
    let link = up_tls(&mock, limits).await.unwrap();
    assert_eq!(link.status["transport"], "tls", "{}", link.status);
    assert!(
        link.status["spec"]
            .as_str()
            .unwrap()
            .starts_with("turns://"),
        "{}",
        link.status
    );
    let dial = dial_of(&link);
    let closed_before = mock.closed();
    let before = mock.count(REFRESH, 0);
    let opened = dial
        .dial(&Target::Addr(peer), tokio::time::Instant::now() + WAIT)
        .await
        .unwrap();
    assert_eq!(opened.peer, Some(peer));
    let mut io = opened.io;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    for text in [&b"first"[..], b"second"] {
        io.write_all(text).await.unwrap();
        let mut back = vec![0u8; 5 + text.len()];
        io.read_exact(&mut back).await.unwrap();
        assert_eq!(back, [&b"echo:"[..], text].concat());
    }
    // The refresh of the TCP allocation, before the lifetime of 2 s.
    until(&mock, REFRESH, 0, before + 1).await;
    assert!(
        !link.status.to_string().contains(mock::PASSWORD)
            && !link.status.to_string().contains(mock::USER),
        "{}",
        link.status
    );
    drop(io);
    // The close: the aborted keeper drops the control connection, and the
    // allocation of the stand-in ends with it.
    until_closed(&mock, closed_before + 1).await;
}

/// GitHub #485: the other direction. A podnet allocation accepts a peer
/// that connects to its relayed address (RFC 6062, 4.2), and the bytes of
/// the peer and of the caller go both ways over TLS.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn turn_a_tls_allocation_accepts_a_peer() {
    crate::test_host::need_loopback!();
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mock = mock::start().await;
    let limits = Limits {
        lifetime: Duration::from_secs(120),
        ..fast()
    };
    let accepted = listen_tls(&mock, limits).await.unwrap();
    let relayed = accepted.relayed;
    // A peer connects to the relayed address; the server sends the
    // CONNECTION-ATTEMPT, and the client binds the new data connection.
    let mut peer = tokio::net::TcpStream::connect(relayed).await.unwrap();
    let stream = tokio::time::timeout(WAIT, accepted.stream)
        .await
        .expect("the accept before the timeout")
        .expect("the accept channel")
        .expect("the accepted stream");
    let mut io = stream;
    io.write_all(b"from client").await.unwrap();
    let mut back = [0u8; 11];
    peer.read_exact(&mut back).await.unwrap();
    assert_eq!(&back, b"from client");
    peer.write_all(b"from peer!").await.unwrap();
    let mut back = [0u8; 10];
    io.read_exact(&mut back).await.unwrap();
    assert_eq!(&back, b"from peer!");
    assert!(mock.count(ALLOCATE, 0) >= 1, "{:?}", mock.seen());
    assert!(
        mock.count(pod_turn::stun::CONNECTION_BIND, 0) >= 1,
        "{:?}",
        mock.seen()
    );
    drop(io);
    drop(peer);
}
/// the caller. TURN carries no rate limit, so the caller's value is shown.
#[tokio::test]
async fn turn_the_status_shows_the_lifetime_and_the_rate_limit() {
    crate::test_host::need_loopback!();
    let mock = mock::start().await;
    let link = up_with_rate(
        &mock,
        Account::new(mock::USER, mock::PASSWORD),
        Limits {
            lifetime: Duration::from_secs(120),
            ..fast()
        },
        Some(500_000),
    )
    .await
    .unwrap();
    assert_eq!(link.status["lifetime_s"], 120);
    assert_eq!(link.status["rate_limit"], 500_000);
    assert_eq!(link.status["relay"], "udp");
    assert_eq!(link.status["tcp_relay"], "rfc6062");
    assert!(!link.status.to_string().contains(mock::PASSWORD));
}

/// A host that refuses the UDP socket names the host error before any
/// allocation, and the relay path is skipped by name.
#[tokio::test]
async fn turn_a_host_that_refuses_udp_names_the_host_error() {
    struct NoUdp;
    impl Dial for NoUdp {
        fn label(&self) -> String {
            "no-udp".into()
        }
        fn dial<'a>(
            &'a self,
            _: &'a Target,
            _: tokio::time::Instant,
        ) -> BoxFuture<'a, Result<Connected, Failure>> {
            Box::pin(async { Err(Failure::new(Stage::Connect, Class::Refused, "unused")) })
        }
        fn datagrams<'a>(&'a self) -> BoxFuture<'a, Result<Arc<dyn Datagrams>, Failure>> {
            Box::pin(async {
                Err(Failure::new(
                    Stage::Connect,
                    Class::Unavailable,
                    "no UDP socket: the seccomp filter denies socket(AF_INET, SOCK_DGRAM)",
                ))
            })
        }
    }
    let mock = mock::start().await;
    let way: Arc<dyn Dial> = Arc::new(NoUdp);
    let spec = turn::parse(&format!("turn://{}", mock.addr)).unwrap();
    let err = turn::up(
        &spec,
        Account::new(mock::USER, mock::PASSWORD),
        Some(&way),
        fast(),
        None,
    )
    .await
    .expect_err("no UDP");
    assert_eq!(err.class, Class::Unavailable);
    assert!(err.text().contains("allocate"), "{err}");
    assert!(
        err.text().contains("seccomp") && err.text().contains("SOCK_DGRAM"),
        "{err}"
    );
}

/// A network below the TURN network gives its datagrams to the allocation.
#[tokio::test]
async fn turn_over_the_datagrams_of_the_way_below() {
    crate::test_host::need_loopback!();
    struct Counted(AtomicUsize);
    impl Dial for Counted {
        fn label(&self) -> String {
            "counted".into()
        }
        fn dial<'a>(
            &'a self,
            target: &'a Target,
            deadline: tokio::time::Instant,
        ) -> BoxFuture<'a, Result<Connected, Failure>> {
            crate::dialers::Direct.dial(target, deadline)
        }
        fn datagrams<'a>(&'a self) -> BoxFuture<'a, Result<Arc<dyn Datagrams>, Failure>> {
            self.0.fetch_add(1, SeqCst);
            crate::dialers::Direct.datagrams()
        }
    }
    let (mock, peer) = (mock::start().await, mock::echo_peer().await);
    let below = Arc::new(Counted(AtomicUsize::new(0)));
    let way: Arc<dyn Dial> = below.clone();
    let spec = turn::parse(&format!("turn://{}", mock.addr)).unwrap();
    let account = Account::new(mock::USER, mock::PASSWORD);
    let link = turn::up(&spec, account, Some(&way), fast(), None)
        .await
        .unwrap();
    let socket = dial_of(&link).datagrams().await.unwrap();
    send_to(&*socket, b"hi", peer).await.unwrap();
    assert_eq!(recv(&*socket).await.unwrap().0, b"echo:hi");
    assert_eq!(below.0.load(SeqCst), 2, "the probe and the socket");
}

/// The router takes the network in `networks:`.
#[test]
fn turn_the_router_takes_the_network_in_networks() {
    let text = "networks:\n  relay-udp: turns://turn.example:443\n  other: turn://203.0.113.9\ndoors:\n  - listen: unix:/tmp/r-turn.sock\n    via: other\n";
    let config = crate::router::parse(text).unwrap();
    crate::router::Router::plan(&config).unwrap();
}
