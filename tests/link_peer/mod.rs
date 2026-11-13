//! The shared part of the tests of the carriers of `podnet::link_keyed`
//! (quic, udp, poll, sse, chunked, httppoll, dns). Each test file of a
//! carrier includes this module and gives the name of its carrier and the
//! form of its dial address. The tests go through `podnet::link_keyed`, the
//! glue that `link listen`, `link dial` and `link connect` use, so a test
//! fails when the glue of its carrier is gone.

#![allow(dead_code)]

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use podnet::exit::code_of;
use podnet::identity::{self, AllowList, Identity};
use podnet::link::{self, Settings};
use podnet::link_keyed::{self, Tune};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::timeout;

pub const LIMIT: Duration = Duration::from_secs(30);

/// The wait for a peer at a listener that refuses it: a carrier can cut a
/// refused peer and go on, with no error at the listener.
pub const QUIET: Duration = Duration::from_secs(3);

/// The dial address of the carrier for the key of `listener` and the
/// address of its door.
pub type AddressOf = fn(&str, &str) -> String;

pub fn install(dir: &Path, name: &str) -> Identity {
    let sub = dir.join(name);
    std::fs::create_dir(&sub).unwrap();
    identity::create(&sub).unwrap()
}

pub fn list_of(keys: &[&Identity]) -> AllowList {
    let text: Vec<String> = keys.iter().map(|k| k.fingerprint()).collect();
    AllowList::parse(&text.join("\n")).unwrap()
}

type Dialed = anyhow::Result<(link::Link, pod_identity::PublicKey)>;
type Accepted = anyhow::Result<(link::Link, Option<pod_identity::PublicKey>)>;

/// One carrier of `link_keyed`: its name and what the tests give it.
pub struct Case {
    pub name: &'static str,
    pub tune: Tune,
    pub address_of: AddressOf,
}

impl Case {
    pub async fn listen(&self, me: &Identity, allow: &AllowList) -> link::Door {
        link_keyed::listen(self.name, &self.tune, me, allow, "127.0.0.1:0")
            .await
            .unwrap()
    }

    pub async fn dial(&self, me: &Identity, allow: &AllowList, to: &str) -> Dialed {
        link_keyed::dial(self.name, &self.tune, me, allow, to, Settings::default()).await
    }

    /// One dial and one accept at the same time.
    pub async fn meet(
        &self,
        me: &Identity,
        allow: AllowList,
        to: &str,
        door: &mut link::Door,
    ) -> (Dialed, Accepted) {
        tokio::join!(self.dial(me, &allow, to), quiet(door))
    }
}

/// An accept that gives up after [`QUIET`] with no peer.
pub async fn quiet(door: &mut link::Door) -> Accepted {
    match timeout(QUIET, door.accept_peer(Settings::default())).await {
        Ok(result) => result,
        Err(_) => Err(anyhow::anyhow!("no peer came in {QUIET:?}")),
    }
}

/// Open a stream at `near`, send `msg`, read it at `far`, send it back.
/// Each side writes while the other reads: a message larger than the
/// window of a stream waits for the reader.
pub async fn echo(near_link: &link::Link, far: &link::Link, msg: &[u8]) {
    let near = near_link.open().unwrap();
    let (mut near_read, mut near_write) = tokio::io::split(near);
    let send = async {
        near_write.write_all(msg).await.unwrap();
    };
    let turn = async {
        let peer = timeout(LIMIT, far.accept()).await.unwrap().unwrap();
        let (mut peer_read, mut peer_write) = tokio::io::split(peer);
        let mut got = vec![0; msg.len()];
        let mut sent = 0;
        while sent < msg.len() {
            let end = (sent + 16 * 1024).min(msg.len());
            timeout(LIMIT, peer_read.read_exact(&mut got[sent..end]))
                .await
                .unwrap()
                .unwrap();
            peer_write.write_all(&got[sent..end]).await.unwrap();
            sent = end;
        }
        got
    };
    let read_back = async {
        let mut back = vec![0; msg.len()];
        timeout(LIMIT, near_read.read_exact(&mut back))
            .await
            .unwrap()
            .unwrap();
        back
    };
    let (_, got, back) = tokio::join!(send, turn, read_back);
    assert!(got == msg, "the far end got the message whole");
    assert!(back == msg, "the message came back whole");
}

/// A link crosses the carrier, each end learns the key of the other, and
/// `bytes` go there and back on one stream.
pub async fn a_link_crosses(case: &Case, bytes: usize) {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (install(dir.path(), "a"), install(dir.path(), "b"));
    let mut door = case.listen(&b, &list_of(&[&a])).await;
    let to = (case.address_of)(&b.fingerprint(), &door.address());
    let (dialed, accepted) = case.meet(&a, list_of(&[&b]), &to, &mut door).await;
    let ((near, seen_by_a), (far, seen_by_b)) = (dialed.unwrap(), accepted.unwrap());
    assert_eq!(seen_by_a, b.public(), "the dialer learns the listener key");
    assert_eq!(
        seen_by_b,
        Some(a.public()),
        "the listener learns the dialer key"
    );
    let msg: Vec<u8> = (0..bytes).map(|i| (i % 251) as u8).collect();
    echo(&near, &far, &msg).await;
}

/// A dialer that the listener does not allow exits 77, and the door stays
/// open for a peer that the list holds.
pub async fn a_stranger_exits_77(case: &Case) {
    let dir = tempfile::tempdir().unwrap();
    let (a, b, c) = (
        install(dir.path(), "a"),
        install(dir.path(), "b"),
        install(dir.path(), "c"),
    );
    let mut door = case.listen(&b, &list_of(&[&a])).await;
    let to = (case.address_of)(&b.fingerprint(), &door.address());
    let (dialed, accepted) = case.meet(&c, list_of(&[&b]), &to, &mut door).await;
    let refused = dialed.expect_err("the dial is refused");
    assert_eq!(code_of(&refused), 77, "{refused:#}");
    accepted.expect_err("the listener takes no peer");
    let (dialed, accepted) = case.meet(&a, list_of(&[&b]), &to, &mut door).await;
    dialed.unwrap();
    assert_eq!(accepted.unwrap().1, Some(a.public()));
}

/// A listener with another key than the pinned one exits 77 at the dialer.
pub async fn another_key_exits_77(case: &Case) {
    let dir = tempfile::tempdir().unwrap();
    let (a, b, c) = (
        install(dir.path(), "a"),
        install(dir.path(), "b"),
        install(dir.path(), "c"),
    );
    let mut door = case.listen(&b, &list_of(&[&a])).await;
    let to = (case.address_of)(&c.fingerprint(), &door.address());
    let (dialed, _) = case.meet(&a, list_of(&[&b, &c]), &to, &mut door).await;
    let e = dialed.err().unwrap();
    assert_eq!(code_of(&e), 77, "{e:#}");
}

/// A dial address with no key exits 64, before any packet.
pub async fn no_key_exits_64(case: &Case, address: &str) {
    let dir = tempfile::tempdir().unwrap();
    let me = install(dir.path(), "me");
    let e = case
        .dial(&me, &list_of(&[&me]), address)
        .await
        .err()
        .unwrap();
    assert_eq!(code_of(&e), 64, "{e:#}");
}

pub fn podnet(dir: &Path, state: &str, args: &[&str]) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_podnet"));
    cmd.args(args)
        .env("PODNET_STATE_DIR", dir.join(state))
        .env("PODNET_RUN_DIR", dir.join(format!("run-{state}")))
        .env_remove("RUST_LOG");
    cmd
}

struct Running(Child);

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub fn write_allow(dir: &Path, name: &str, keys: &[&Identity]) -> PathBuf {
    let file = dir.join(name);
    let text: Vec<String> = keys.iter().map(|k| k.fingerprint()).collect();
    std::fs::write(&file, text.join("\n") + "\n").unwrap();
    file
}

/// `link listen --carrier NAME` and `link dial --check`: a peer of the list
/// exits 0 and prints the key of the listener; a stranger exits 77. The
/// listen address is `listen` with the flags `listen_flags`; the dial takes
/// `dial_flags`.
pub fn cli_check(
    carrier: &str,
    listen: &str,
    listen_flags: &[&str],
    address_of: AddressOf,
    dial_flags: &[&str],
) {
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
    let mut args = vec![
        "link",
        "listen",
        listen,
        "--carrier",
        carrier,
        "--allow",
        allow_b.to_str().unwrap(),
        "--json",
    ];
    args.extend_from_slice(listen_flags);
    let mut listener = podnet(dir.path(), "b", &args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(listener.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let _guard = Running(listener);
    let doc: serde_json::Value = serde_json::from_str(&line).expect(&line);
    assert_eq!(doc["carrier"], carrier);
    let to = address_of(&b.fingerprint(), doc["listen"].as_str().unwrap());
    let run = |state: &str, allow: &Path| {
        let mut args = vec![
            "link",
            "dial",
            &to,
            "--carrier",
            carrier,
            "--allow",
            allow.to_str().unwrap(),
            "--check",
        ];
        args.extend_from_slice(dial_flags);
        podnet(dir.path(), state, &args).output().unwrap()
    };
    let ok = run("a", &allow_a);
    assert_eq!(ok.status.code(), Some(0), "{ok:?}");
    assert!(String::from_utf8_lossy(&ok.stdout).contains(&b.fingerprint()));
    let stranger = run("c", &allow_c);
    assert_eq!(stranger.status.code(), Some(77), "{stranger:?}");
    let _ = c;
}

/// The registry row of `name`: built, its feature, and a probe that
/// refuses a dial (it has no key of this install).
pub fn the_registry_row_refuses_a_dial(name: &str, feature: &str, address: &str) {
    use pod_link::CarrierErrorKind;
    let report = podnet::carriers::report(&podnet::carriers::Options::default());
    let row = report.iter().find(|r| r.name == name).unwrap();
    assert!(row.built, "{name}");
    assert_eq!(row.feature, Some(feature));
    if let Ok(probe) = podnet::carriers::open(name, &podnet::carriers::Options::default()) {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let e = rt.block_on(probe.dial(address)).err().unwrap();
        assert_eq!(e.kind, CarrierErrorKind::Usage, "{e}");
    }
}
