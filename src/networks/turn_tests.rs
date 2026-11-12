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
    let spec = turn::parse(&format!("turn://{}", mock.addr)).unwrap();
    turn::up(&spec, account, None, limits).await
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
        assert!(!network.caps().contains(Caps::EGRESS_TCP), "{spec}");
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

/// TCP is refused by name: RFC 6062 is not in `pod-turn`.
#[tokio::test]
async fn turn_a_tcp_dial_is_refused_by_name() {
    crate::test_host::need_loopback!();
    let mock = mock::start().await;
    let dial = dial_of(&up(&mock).await);
    let target = Target::Name {
        host: "example.test".into(),
        port: 443,
    };
    let err = dial
        .dial(&target, tokio::time::Instant::now() + WAIT)
        .await
        .expect_err("TCP refused");
    assert_eq!(err.class, Class::Unsupported);
    assert!(
        err.text().contains("RFC 6062") && err.text().contains("example.test"),
        "{err}"
    );
    assert_eq!(
        mock.count(ALLOCATE, 0),
        1,
        "the refusal opens no allocation"
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
    let link = turn::up(&spec, account, Some(&way), fast()).await.unwrap();
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
