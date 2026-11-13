//! The podnet relay as a way of the transit of a transfer (T-104): two ends
//! of podnet meet at a stand-in mailbox; the sender runs the node of a pair
//! at the fake relay of the reverse road, and the receiver dials it when no
//! direct hint and no transit relay works. The SHA-256 of each byte that
//! lands is the sender's.

#[path = "transfer_harness/mod.rs"]
mod harness;

#[path = "relay_reverse/fake.rs"]
mod fake;

mod test_host;

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use harness::Mailbox;
use pod_wormhole::Code;
use pod_ws::url::WsUrl;
use podnet::transfer::land::Caps;
use podnet::transfer::{self, Listen, Receive, Received, RelayPair, Settings, What};
use sha2::{Digest, Sha256};

const NAME: &str = "testnode";
/// The tokens that the fake relay takes: made-up values of a test.
const NODE_TOKEN: &str = "testonly-node";
const CONNECT_TOKEN: &str = "testonly-connect";
const MIB: usize = 1 << 20;

fn way() -> pod_ws::Way {
    podnet::net::init_crypto();
    let settings = pod_proxy::Settings::new("podnet-test", pod_proxy::ProxyChoice::Direct);
    let dial = pod_proxy::Dialer::new(settings, Arc::new(pod_proxy::resolve::System::new()));
    pod_ws::Way::new(
        podnet::trust::context().clone(),
        Arc::new(dial),
        "podnet/test",
    )
}

/// The settings of one end at `mailbox`, with `relays` and `listen`.
fn settings(mailbox: &str, relays: &[String], listen: Listen) -> Settings {
    let relays = relays
        .iter()
        .map(|r| transfer::Relay::parse(r).unwrap())
        .collect();
    let mut settings = Settings::new(vec![WsUrl::parse(mailbox).unwrap()], relays, way());
    settings.listen = listen;
    settings.wait = Duration::from_secs(30);
    settings.step = Duration::from_secs(5);
    settings.transit_wait = Duration::from_secs(30);
    settings
}

/// The pair `NAME` of the fake relay `url` in `dir`, as `podnet relay pair`
/// stores it (mode 0600 in a directory of mode 0700).
fn write_pair(dir: &Path, url: &str) -> RelayPair {
    use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _};
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .unwrap();
    let expires = (std::time::SystemTime::now() + Duration::from_secs(3600))
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let doc = serde_json::json!({
        "relay": url, "name": NAME, "node_token": NODE_TOKEN,
        "connect_token": CONNECT_TOKEN, "stop_token": "", "expires": expires,
    });
    let path = dir.join(format!("pair-{NAME}.json"));
    std::fs::write(&path, doc.to_string()).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    RelayPair {
        dir: dir.to_path_buf(),
        name: NAME.into(),
    }
}

/// A transit relay of magic-wormhole that refuses: nothing listens there.
fn refused_relay() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    format!("tcp:127.0.0.1:{}", listener.local_addr().unwrap().port())
}

fn pseudo_random(len: usize, seed: u32) -> Vec<u8> {
    let mut x = seed;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as u8
        })
        .collect()
}

fn sha256(path: &Path) -> String {
    pod_wormhole::hex(&Sha256::digest(std::fs::read(path).unwrap()))
}

/// Send the file `file` from `sender` and receive it with `receiver` into
/// `out`.
async fn run(
    sender: Settings,
    receiver: Settings,
    file: &Path,
    out: &Path,
) -> (anyhow::Result<transfer::Sent>, anyhow::Result<Received>) {
    let opened = transfer::open(&sender).await.expect("a code");
    let code = Code::parse(opened.code()).expect("a code that parses");
    let what = What::Path(file.to_path_buf());
    let sending = tokio::spawn(async move { opened.send(&sender, &what).await });
    let how = Receive {
        out: out.to_path_buf(),
        caps: Caps::default(),
    };
    let mut ask = |_: &transfer::Offered| Ok(true);
    let received = transfer::receive(&receiver, &code, &how, &mut ask).await;
    (sending.await.unwrap(), received)
}

/// The via of a received path, and the path.
fn landed(received: anyhow::Result<Received>) -> (String, std::path::PathBuf) {
    match received.unwrap() {
        Received::Path { via, path, .. } => (via, path),
        other => panic!("a path, not {other:?}"),
    }
}

// No end listens and the transit relay of the sender refuses: the file goes
// through the node of the podnet relay.
#[tokio::test(flavor = "multi_thread")]
async fn transfer_over_the_relay_when_the_direct_path_is_refused() {
    test_host::need_loopback!();
    let mailbox = Mailbox::start().await;
    let (url, state) = fake::fake_relay(None).await;
    let (src, out, pairs) = (
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
    );
    let file = src.path().join("f");
    std::fs::write(&file, pseudo_random(300_001, 7)).unwrap();
    let mut sender = settings(&mailbox.url, &[refused_relay()], Listen::No);
    sender.relay_pair = Some(write_pair(&pairs.path().join("relay"), &url));
    let receiver = settings(&mailbox.url, &[], Listen::No);
    let (sent, received) = run(sender, receiver, &file, out.path()).await;
    let sent = sent.unwrap();
    let (via, path) = landed(received);
    assert_eq!(via, format!("podnet relay {NAME}"));
    assert_eq!(sent.via, via);
    assert_eq!(sha256(&path), sha256(&file));
    assert_eq!(sent.sha256.as_deref(), Some(sha256(&file).as_str()));
    assert!(state.lock().unwrap().operator_sockets >= 1);
}

// T-104, the Prove: the fake relay closes each relay session at 1 MiB (both
// ways, with 1009, as the relay at 64 MiB). The layer moves the session to a
// new leg before the cap, and 4 MiB land with the SHA-256 of the sender.
#[tokio::test(flavor = "multi_thread")]
async fn transfer_over_the_relay_crosses_the_cap() {
    test_host::need_loopback!();
    let mailbox = Mailbox::start().await;
    let (url, state) = fake::fake_relay(None).await;
    state.lock().unwrap().cap = Some(MIB);
    let (src, out, pairs) = (
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
    );
    let file = src.path().join("big");
    std::fs::write(&file, pseudo_random(4 * MIB, 0x1234_5678)).unwrap();
    let mut sender = settings(&mailbox.url, &[], Listen::No);
    sender.relay_pair = Some(write_pair(&pairs.path().join("relay"), &url));
    let mut receiver = settings(&mailbox.url, &[], Listen::No);
    receiver.relay_resume.move_bytes = 256 << 10;
    let (sent, received) = run(sender, receiver, &file, out.path()).await;
    sent.unwrap();
    let (via, path) = landed(received);
    assert_eq!(via, format!("podnet relay {NAME}"));
    assert_eq!(sha256(&path), sha256(&file));
    let state = state.lock().unwrap();
    let total: usize = state.carried.values().sum();
    assert!(total > 4 * MIB, "the relay carried {total} bytes");
    assert!(
        state.carried.values().all(|b| *b <= MIB),
        "no relay session went past the cap: {:?}",
        state.carried
    );
    assert!(
        state.operator_sockets >= 4,
        "{} legs",
        state.operator_sockets
    );
}

// The podnet relay is the last way: a direct hint that works wins.
#[tokio::test(flavor = "multi_thread")]
async fn transfer_a_direct_hint_wins_over_the_relay() {
    test_host::need_loopback!();
    let mailbox = Mailbox::start().await;
    let (url, _state) = fake::fake_relay(None).await;
    let (src, out, pairs) = (
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
    );
    let file = src.path().join("f");
    std::fs::write(&file, pseudo_random(10_000, 3)).unwrap();
    let loopback = Listen::At(SocketAddr::from(([127, 0, 0, 1], 0)));
    let mut sender = settings(&mailbox.url, &[], loopback);
    sender.relay_pair = Some(write_pair(&pairs.path().join("relay"), &url));
    let receiver = settings(&mailbox.url, &[], Listen::No);
    let (sent, received) = run(sender, receiver, &file, out.path()).await;
    sent.unwrap();
    let (via, path) = landed(received);
    assert!(via.starts_with("direct to 127.0.0.1:"), "{via}");
    assert_eq!(sha256(&path), sha256(&file));
}

// A pair that is not stored stops the sender before its offer.
#[tokio::test]
async fn transfer_a_relay_pair_that_is_not_stored_is_refused() {
    test_host::need_loopback!();
    let mailbox = Mailbox::start().await;
    let (src, out, pairs) = (
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
    );
    let file = src.path().join("f");
    std::fs::write(&file, b"x").unwrap();
    let mut sender = settings(&mailbox.url, &[], Listen::No);
    let dir = pairs.path().join("relay");
    write_pair(&dir, "http://127.0.0.1:9");
    sender.relay_pair = Some(RelayPair {
        dir,
        name: "nosuchpair".into(),
    });
    let mut receiver = settings(&mailbox.url, &[], Listen::No);
    receiver.transit_wait = Duration::from_secs(2);
    let (sent, received) = run(sender, receiver, &file, out.path()).await;
    let e = sent.unwrap_err();
    assert!(format!("{e:#}").contains("nosuchpair"), "{e:#}");
    received.unwrap_err();
    assert_eq!(std::fs::read_dir(out.path()).unwrap().count(), 0);
}
