//! The TCP carrier of the link (T-268): TLS 1.3 with the Ed25519 key of each
//! end pinned in the handshake. The tests with no socket run on each host.
//! A test that needs TCP loopback prints `skipping, REASON` on a host that
//! denies it (`EACCES`), and does not fail.
#![cfg(feature = "tcp")]

#[path = "test_host/mod.rs"]
mod test_host;

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use pod_link::tcp::{self, Tcp};
use pod_link::{Carrier, CarrierError, CarrierErrorKind};
use podnet::exit::code_of;
use podnet::identity::{self, AllowList, Identity};
use podnet::link::{self, Settings};
use podnet::link_tcp;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::timeout;

const LIMIT: Duration = Duration::from_secs(20);

fn install(dir: &Path, name: &str) -> Identity {
    let sub = dir.join(name);
    std::fs::create_dir(&sub).unwrap();
    identity::create(&sub).unwrap()
}

fn list_of(keys: &[&Identity]) -> AllowList {
    let text: Vec<String> = keys.iter().map(|k| k.fingerprint()).collect();
    AllowList::parse(&text.join("\n")).unwrap()
}

fn address(listener: &Identity, door: &link::Door) -> String {
    format!("{}@{}", listener.fingerprint(), door.address())
}

type Dialed = anyhow::Result<(link::Link, pod_identity::PublicKey)>;
type Accepted = anyhow::Result<(link::Link, Option<pod_identity::PublicKey>)>;

/// One dial and one accept at the same time.
async fn meet(
    dialing: &Tcp,
    allow: AllowList,
    to: &str,
    door: &mut link::Door,
) -> (Dialed, Accepted) {
    tokio::join!(
        link_tcp::dial(dialing, &allow, to, Settings::default()),
        door.accept_peer(Settings::default())
    )
}

// ---- no socket ----

#[test]
fn the_allow_list_feeds_the_client_keys_in_each_form() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b, c) = (
        install(dir.path(), "a"),
        install(dir.path(), "b"),
        install(dir.path(), "c"),
    );
    let text = format!(
        "{}\n{} a comment\n{}\n",
        a.fingerprint(),
        b.public().fingerprint(),
        b.public().hex()
    );
    let allow = AllowList::parse(&text).unwrap();
    assert_eq!(allow.names().len(), 3);
    let keys = link_tcp::keys(&allow);
    assert!(keys.admits(&a.public()), "the display form");
    assert!(
        keys.admits(&b.public()),
        "the SHA256: form and the hex form"
    );
    assert!(!keys.admits(&c.public()), "default deny");
    assert!(!link_tcp::keys(&AllowList::default()).admits(&a.public()));
}

#[test]
fn the_carrier_is_a_stream_and_says_why_a_host_cannot_use_it() {
    let dir = tempfile::tempdir().unwrap();
    let me = install(dir.path(), "me");
    let tcp = link_tcp::carrier(&me, &AllowList::default());
    assert_eq!(tcp.name(), "tcp");
    assert!(tcp.facts().reliable && tcp.facts().ordered && tcp.facts().duplex);
    assert_eq!(tcp.facts().mtu, None);
    assert_eq!(tcp::ALPN, b"podlink/1");
    if let Err(e) = tcp.available() {
        assert!(e.to_string().contains("loopback"), "{e}");
    }
}

#[test]
fn the_registry_row_has_no_key_and_refuses_a_dial() {
    let report = podnet::carriers::report(&podnet::carriers::Options::default());
    let row = report.iter().find(|r| r.name == "tcp").unwrap();
    assert!(row.built);
    assert_eq!(row.feature, Some("tcp"));
    let opened = podnet::carriers::open("tcp", &podnet::carriers::Options::default());
    if let Ok(probe) = opened {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let e = rt.block_on(probe.dial("x@127.0.0.1:1")).err().unwrap();
        assert_eq!(e.kind, CarrierErrorKind::Usage, "{e}");
    }
}

#[tokio::test]
async fn a_dial_address_with_no_key_exits_64() {
    let dir = tempfile::tempdir().unwrap();
    let me = install(dir.path(), "me");
    let tcp = link_tcp::carrier(&me, &AllowList::default());
    let e = link_tcp::dial(
        &tcp,
        &list_of(&[&me]),
        "127.0.0.1:7000",
        Settings::default(),
    )
    .await
    .err()
    .unwrap();
    assert_eq!(code_of(&e), 64, "{e:#}");
    let e = link_tcp::dial(
        &tcp,
        &list_of(&[&me]),
        "nokey@127.0.0.1:7000",
        Settings::default(),
    )
    .await
    .err()
    .unwrap();
    assert_eq!(code_of(&e), 64, "{e:#}");
}

#[test]
fn a_refused_key_maps_to_77_and_a_denied_socket_does_not() {
    let alert = std::io::Error::other(rustls::Error::AlertReceived(
        rustls::AlertDescription::CertificateUnknown,
    ));
    let refused =
        CarrierError::with_source("tcp", CarrierErrorKind::Refused, "the listener", alert);
    assert_eq!(code_of(&link::carrier_fault(refused)), 77);
    let denied = CarrierError::io(
        "tcp",
        "bind 0.0.0.0:7000",
        std::io::Error::from_raw_os_error(13),
    );
    assert_eq!(denied.kind, CarrierErrorKind::Refused);
    assert_eq!(
        code_of(&link::carrier_fault(denied)),
        69,
        "a host that denies the socket is not a refused key"
    );
}

#[test]
fn the_dial_of_the_cli_with_no_key_in_the_address_exits_64() {
    let dir = tempfile::tempdir().unwrap();
    let out = podnet(
        dir.path(),
        "state",
        &[
            "link",
            "dial",
            "127.0.0.1:1",
            "--carrier",
            "tcp",
            "--peer",
            "x",
        ],
    )
    .output()
    .unwrap();
    // `--peer x` is no key: the allow list is read first and exits 78.
    assert_eq!(out.status.code(), Some(78), "{out:?}");
    let me = install(dir.path(), "cli-me");
    let file = dir.path().join("allow");
    std::fs::write(&file, format!("{}\n", me.fingerprint())).unwrap();
    let out = podnet(
        dir.path(),
        "cli-me",
        &[
            "link",
            "dial",
            "127.0.0.1:1",
            "--carrier",
            "tcp",
            "--allow",
            file.to_str().unwrap(),
        ],
    )
    .output()
    .unwrap();
    assert_eq!(out.status.code(), Some(64), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stderr).contains("KEY@HOST:PORT"));
}

// ---- TCP loopback ----

async fn echo(link: &link::Link, far: &link::Link, msg: &[u8]) {
    let mut near = link.open().unwrap();
    near.write_all(msg).await.unwrap();
    let mut peer = timeout(LIMIT, far.accept()).await.unwrap().unwrap();
    let mut got = vec![0; msg.len()];
    timeout(LIMIT, peer.read_exact(&mut got))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got, msg);
    peer.write_all(&got).await.unwrap();
    let mut back = vec![0; msg.len()];
    timeout(LIMIT, near.read_exact(&mut back))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(back, msg);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_link_crosses_tcp_and_each_end_learns_the_key_of_the_other() {
    test_host::need_loopback!();
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (install(dir.path(), "a"), install(dir.path(), "b"));
    let listening = link_tcp::carrier(&b, &list_of(&[&a]));
    let dialing = link_tcp::carrier(&a, &AllowList::default());
    let mut door = link_tcp::listen(&listening, "127.0.0.1:0").await.unwrap();
    let to = address(&b, &door);
    let (dialed, accepted) = meet(&dialing, list_of(&[&b]), &to, &mut door).await;
    let ((near, seen_by_a), (far, seen_by_b)) = (dialed.unwrap(), accepted.unwrap());
    assert_eq!(seen_by_a, b.public(), "the dialer learns the listener key");
    assert_eq!(
        seen_by_b,
        Some(a.public()),
        "the listener learns the dialer key"
    );
    echo(&near, &far, b"over tcp with tls 1.3").await;
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
    let listening = link_tcp::carrier(&b, &list_of(&[&a]));
    let mut door = link_tcp::listen(&listening, "127.0.0.1:0").await.unwrap();
    let to = address(&b, &door);
    // c is not in the list of b.
    let dialing = link_tcp::carrier(&c, &AllowList::default());
    let (dialed, accepted) = meet(&dialing, list_of(&[&b]), &to, &mut door).await;
    let refused = dialed.expect_err("the dial is refused");
    assert_eq!(code_of(&refused), 77, "{refused:#}");
    assert_eq!(code_of(&accepted.expect_err("the accept fails")), 77);
    // The door is still open for a peer that the list holds.
    let ok = link_tcp::carrier(&a, &AllowList::default());
    let (dialed, accepted) = meet(&ok, list_of(&[&b]), &to, &mut door).await;
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
    let listening = link_tcp::carrier(&b, &list_of(&[&a]));
    let door = link_tcp::listen(&listening, "127.0.0.1:0").await.unwrap();
    // The dialer pins the key of c, but b answers.
    let to = address(&c, &door);
    let dialing = link_tcp::carrier(&a, &AllowList::default());
    let e = link_tcp::dial(&dialing, &list_of(&[&b, &c]), &to, Settings::default())
        .await
        .err()
        .unwrap();
    assert_eq!(code_of(&e), 77, "{e:#}");
    assert!(
        format!("{e:#}").contains(&b.public().fingerprint()),
        "{e:#}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_listener_that_the_dialer_list_does_not_hold_exits_77() {
    test_host::need_loopback!();
    let dir = tempfile::tempdir().unwrap();
    let (a, b, c) = (
        install(dir.path(), "a"),
        install(dir.path(), "b"),
        install(dir.path(), "c"),
    );
    let listening = link_tcp::carrier(&b, &list_of(&[&a]));
    let mut door = link_tcp::listen(&listening, "127.0.0.1:0").await.unwrap();
    let to = address(&b, &door);
    let dialing = link_tcp::carrier(&a, &AllowList::default());
    let (dialed, _) = meet(&dialing, list_of(&[&c]), &to, &mut door).await;
    let e = dialed.err().unwrap();
    assert_eq!(code_of(&e), 77, "{e:#}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_change_of_the_client_keys_applies_to_the_next_handshake() {
    test_host::need_loopback!();
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (install(dir.path(), "a"), install(dir.path(), "b"));
    let listening = link_tcp::carrier(&b, &AllowList::default());
    let mut door = link_tcp::listen(&listening, "127.0.0.1:0").await.unwrap();
    let to = address(&b, &door);
    let dialing = link_tcp::carrier(&a, &AllowList::default());
    let (dialed, _) = meet(&dialing, list_of(&[&b]), &to, &mut door).await;
    assert_eq!(code_of(&dialed.err().unwrap()), 77, "default deny");
    listening.clients().set(list_of(&[&a]).names());
    let (dialed, accepted) = meet(&dialing, list_of(&[&b]), &to, &mut door).await;
    dialed.unwrap();
    accepted.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_peer_that_speaks_no_tls_is_not_a_link() {
    test_host::need_loopback!();
    let dir = tempfile::tempdir().unwrap();
    let b = install(dir.path(), "b");
    let listening = link_tcp::carrier(&b, &AllowList::default());
    let mut door = link_tcp::listen(&listening, "127.0.0.1:0").await.unwrap();
    let mut raw = tokio::net::TcpStream::connect(door.address())
        .await
        .unwrap();
    raw.write_all(b"GET / HTTP/1.0\r\n\r\n").await.unwrap();
    let e = timeout(LIMIT, door.accept_peer(Settings::default()))
        .await
        .unwrap()
        .expect_err("a plain peer is refused");
    assert_ne!(code_of(&e), 0, "{e:#}");
}

// ---- the commands ----

fn podnet(dir: &Path, state: &str, args: &[&str]) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_podnet"));
    cmd.args(args)
        .env("PODNET_STATE_DIR", dir.join(state))
        .env("PODNET_RUN_DIR", dir.join(format!("run-{state}")))
        .env_remove("RUST_LOG");
    cmd
}

struct Listener2(Child);

impl Drop for Listener2 {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn write_allow(dir: &Path, name: &str, keys: &[&Identity]) -> PathBuf {
    let file = dir.join(name);
    let text: Vec<String> = keys.iter().map(|k| k.fingerprint()).collect();
    std::fs::write(&file, text.join("\n") + "\n").unwrap();
    file
}

#[test]
fn link_dial_check_meets_link_listen_over_tcp_and_a_stranger_exits_77() {
    test_host::need_loopback!();
    let dir = tempfile::tempdir().unwrap();
    let (a, b, c) = (
        install(dir.path(), "a"),
        install(dir.path(), "b"),
        install(dir.path(), "c"),
    );
    let (allow_b, allow_a, allow_c) = (
        write_allow(dir.path(), "allow-b", &[&a]),
        write_allow(dir.path(), "allow-a", &[&b]),
        write_allow(dir.path(), "allow-c", &[&b]),
    );
    let mut listener = podnet(
        dir.path(),
        "b",
        &[
            "link",
            "listen",
            "127.0.0.1:0",
            "--carrier",
            "tcp",
            "--allow",
            allow_b.to_str().unwrap(),
            "--json",
        ],
    )
    .stdout(Stdio::piped())
    .stderr(Stdio::null())
    .spawn()
    .unwrap();
    let mut line = String::new();
    BufReader::new(listener.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let _guard = Listener2(listener);
    let doc: serde_json::Value = serde_json::from_str(&line).expect(&line);
    assert_eq!(doc["carrier"], "tcp");
    let to = format!("{}@{}", b.fingerprint(), doc["listen"].as_str().unwrap());
    let run = |state: &str, allow: &Path| {
        podnet(
            dir.path(),
            state,
            &[
                "link",
                "dial",
                &to,
                "--carrier",
                "tcp",
                "--allow",
                allow.to_str().unwrap(),
                "--check",
            ],
        )
        .output()
        .unwrap()
    };
    let ok = run("a", &allow_a);
    assert_eq!(ok.status.code(), Some(0), "{ok:?}");
    assert!(String::from_utf8_lossy(&ok.stdout).contains(&b.fingerprint()));
    let stranger = run("c", &allow_c);
    assert_eq!(stranger.status.code(), Some(77), "{stranger:?}");
    assert!(
        String::from_utf8_lossy(&stranger.stderr).contains("refused"),
        "{stranger:?}"
    );
    let _ = c;
}
