//! T-087: the end-to-end channel above the resumable layer (`relay node
//! --resume --seal`, `relay connect --resume --seal`). A sealed session
//! moves past the cap of the relay, and the relay sees no plain byte of
//! it. A key that the other end does not allow exits 77, and a refused
//! operator costs the target no connection.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use podnet::identity::{self, AllowList, Identity};
use podnet::relay_reverse::{
    connect_resumable, connect_sealed, run_node_resumable, run_node_sealed, Resume, Seal, Target,
};
use podnet::ws_relay::Relay;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use super::resume::{pair_of, pseudo_random, sha256};
use super::{echo, fake_relay, wait_for_node, CONNECT_TOKEN, NAME};

const MIB: usize = 1 << 20;
/// A word that the payload holds each 4 KiB: the relay must not see it.
const MARKER: &[u8] = b"PLAIN-T087-MARKER-OF-THE-PAYLOAD";

/// A new install key in a new directory.
fn key() -> (tempfile::TempDir, Identity) {
    let dir = tempfile::tempdir().unwrap();
    let key = identity::create(dir.path()).unwrap();
    (dir, key)
}

/// The channel of `me`, which allows the keys of `keys` (an allow list
/// of `podnet::identity`, as the CLI gives it).
fn seal(me: &Identity, keys: &[&Identity]) -> Seal {
    let text: Vec<String> = keys.iter().map(|k| k.fingerprint()).collect();
    let list = AllowList::parse(&text.join("\n")).unwrap();
    Seal::new(me.key(), move |k| list.admits(k))
}

/// `len` pseudo-random bytes with [`MARKER`] at each 4 KiB.
fn payload(len: usize, seed: u32) -> Vec<u8> {
    let mut bytes = pseudo_random(len, seed);
    for at in (0..len.saturating_sub(MARKER.len())).step_by(4096) {
        bytes[at..at + MARKER.len()].copy_from_slice(MARKER);
    }
    bytes
}

fn holds(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// Send `sent` through the session of `connect` and read the echo back.
/// Gives the bytes back and how the session ended.
async fn through<F, Fut>(sent: Vec<u8>, connect: F) -> (Vec<u8>, anyhow::Result<()>)
where
    F: FnOnce(
            tokio::io::ReadHalf<tokio::io::DuplexStream>,
            tokio::io::WriteHalf<tokio::io::DuplexStream>,
        ) -> Fut
        + Send
        + 'static,
    Fut: std::future::Future<Output = anyhow::Result<()>> + Send + 'static,
{
    let (ours, theirs) = tokio::io::duplex(1 << 16);
    let session = tokio::spawn(async move {
        let (read, write) = tokio::io::split(theirs);
        connect(read, write).await
    });
    let (mut read, mut write) = tokio::io::split(ours);
    let len = sent.len();
    let writer = tokio::spawn(async move {
        for chunk in sent.chunks(64 << 10) {
            write.write_all(chunk).await.unwrap();
        }
        write
    });
    let mut got = vec![0u8; len];
    tokio::time::timeout(Duration::from_secs(180), read.read_exact(&mut got))
        .await
        .expect("the echo in time")
        .expect("the echo");
    let mut write = writer.await.unwrap();
    write.shutdown().await.unwrap();
    let ended = tokio::time::timeout(Duration::from_secs(60), session)
        .await
        .expect("the session ends")
        .unwrap();
    (got, ended)
}

// T-087, the Prove of the channel: the fake relay closes each relay session
// at 1 MiB. A sealed session carries 8 MiB each way through an echo with an
// equal SHA-256, and the bytes that the relay carried hold no marker of the
// payload.
#[tokio::test(flavor = "multi_thread")]
async fn a_sealed_session_moves_past_the_cap_and_the_relay_sees_no_plain_byte() {
    crate::test_host::need_loopback!();
    let (url, state) = fake_relay(None).await;
    {
        let mut state = state.lock().unwrap();
        state.cap = Some(MIB);
        state.tap = Some(Vec::new());
    }
    let (_node_dir, node_key) = key();
    let (_op_dir, op_key) = key();
    let node_seal = seal(&node_key, &[&op_key]);
    let op_seal = seal(&op_key, &[&node_key]);
    let target = Target::parse(&echo().await).unwrap();
    let (_dir, pair) = pair_of(&url);
    let node = tokio::spawn(async move { run_node_sealed(None, pair, &target, &node_seal).await });
    wait_for_node(&state, 1).await;
    let relay = Relay::new(&url, None).unwrap();
    let resume = Resume {
        move_bytes: 256 << 10,
        ..Resume::default()
    };
    let sent = payload(8 * MIB, 0x0870_0001);
    assert!(holds(&sent, MARKER));
    let (got, ended) = through(sent.clone(), move |read, write| async move {
        connect_sealed(&relay, NAME, CONNECT_TOKEN, read, write, resume, &op_seal).await
    })
    .await;
    assert_eq!(sha256(&got), sha256(&sent), "the bytes of the echo");
    assert!(ended.is_ok(), "{ended:?}");
    let (legs, tap) = {
        let mut state = state.lock().unwrap();
        (state.operator_sockets, state.tap.take().unwrap())
    };
    eprintln!("legs {legs}, bytes at the relay {}", tap.len());
    assert!(legs >= 16, "16 MiB at 1 MiB a leg: {legs} legs");
    assert!(
        tap.len() >= 16 * MIB,
        "the relay carried {} bytes",
        tap.len()
    );
    assert!(
        !holds(&tap, MARKER),
        "the relay saw a plain marker of the payload"
    );
    node.abort();
}

// T-087: the control of the test above. With no channel, the relay sees
// the marker: the tap reads what the relay carries.
#[tokio::test(flavor = "multi_thread")]
async fn without_the_channel_the_relay_sees_the_payload() {
    crate::test_host::need_loopback!();
    let (url, state) = fake_relay(None).await;
    state.lock().unwrap().tap = Some(Vec::new());
    let target = Target::parse(&echo().await).unwrap();
    let (_dir, pair) = pair_of(&url);
    let node = tokio::spawn(async move { run_node_resumable(None, pair, &target).await });
    wait_for_node(&state, 1).await;
    let relay = Relay::new(&url, None).unwrap();
    let sent = payload(256 << 10, 0x0870_0002);
    let (got, ended) = through(sent.clone(), move |read, write| async move {
        connect_resumable(&relay, NAME, CONNECT_TOKEN, read, write, Resume::default()).await
    })
    .await;
    assert_eq!(sha256(&got), sha256(&sent));
    assert!(ended.is_ok(), "{ended:?}");
    let tap = state.lock().unwrap().tap.take().unwrap();
    assert!(holds(&tap, MARKER), "the relay carried the plain payload");
    node.abort();
}

/// A TCP echo that counts its connections.
async fn counted_echo() -> (String, Arc<AtomicUsize>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let count = Arc::new(AtomicUsize::new(0));
    let counted = count.clone();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            counted.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(async move {
                let (mut r, mut w) = socket.split();
                let _ = tokio::io::copy(&mut r, &mut w).await;
            });
        }
    });
    (addr, count)
}

/// One sealed session from `op` (which trusts `op_trusts`) to a node of
/// `node_key` (which allows `node_allows`). Gives the exit code of the
/// operator, its error, and the connections of the target.
async fn refused(
    node_key: &Identity,
    node_allows: &[&Identity],
    op_key: &Identity,
    op_trusts: &[&Identity],
) -> (i32, String, usize) {
    let (url, state) = fake_relay(None).await;
    let node_seal = seal(node_key, node_allows);
    let op_seal = seal(op_key, op_trusts);
    let (addr, count) = counted_echo().await;
    let target = Target::parse(&addr).unwrap();
    let (_dir, pair) = pair_of(&url);
    let node = tokio::spawn(async move { run_node_sealed(None, pair, &target, &node_seal).await });
    wait_for_node(&state, 1).await;
    let relay = Relay::new(&url, None).unwrap();
    let (ours, theirs) = tokio::io::duplex(1 << 16);
    let (read, write) = tokio::io::split(theirs);
    let err = tokio::time::timeout(
        Duration::from_secs(60),
        connect_sealed(
            &relay,
            NAME,
            CONNECT_TOKEN,
            read,
            write,
            Resume::default(),
            &op_seal,
        ),
    )
    .await
    .expect("an answer in time")
    .unwrap_err();
    drop(ours);
    node.abort();
    (
        podnet::exit::code_of(&err),
        format!("{err:#}"),
        count.load(Ordering::SeqCst),
    )
}

// T-087: an operator key that the node does not allow exits 77, and the
// node does not dial the target for it.
#[tokio::test(flavor = "multi_thread")]
async fn an_operator_key_that_the_node_does_not_allow_exits_77() {
    crate::test_host::need_loopback!();
    let (_a, node_key) = key();
    let (_b, op_key) = key();
    let (_c, other) = key();
    let (code, err, dials) = refused(&node_key, &[&other], &op_key, &[&node_key]).await;
    assert_eq!(code, podnet::exit::NO_PERM, "{err}");
    assert_eq!(dials, 0, "the target got a connection for a refused key");
}

// T-087: a node key that the operator does not trust exits 77.
#[tokio::test(flavor = "multi_thread")]
async fn a_node_key_that_the_operator_does_not_trust_exits_77() {
    crate::test_host::need_loopback!();
    let (_a, node_key) = key();
    let (_b, op_key) = key();
    let (_c, other) = key();
    let (code, err, dials) = refused(&node_key, &[&op_key], &op_key, &[&other]).await;
    assert_eq!(code, podnet::exit::NO_PERM, "{err}");
    assert!(err.contains("node key"), "{err}");
    assert_eq!(dials, 0, "the target got a connection for a refused node");
}

// T-087: a sealed operator and a node with no --seal: the operator exits 69
// in time (the handshake fails), and writes no byte.
#[tokio::test(flavor = "multi_thread")]
async fn a_sealed_operator_and_a_node_with_no_seal_fail() {
    crate::test_host::need_loopback!();
    let (url, state) = fake_relay(None).await;
    let (_a, node_key) = key();
    let (_b, op_key) = key();
    let op_seal = seal(&op_key, &[&node_key]);
    let target = Target::parse(&echo().await).unwrap();
    let (_dir, pair) = pair_of(&url);
    let node = tokio::spawn(async move { run_node_resumable(None, pair, &target).await });
    wait_for_node(&state, 1).await;
    let relay = Relay::new(&url, None).unwrap();
    let (mut ours, theirs) = tokio::io::duplex(1 << 16);
    let (read, write) = tokio::io::split(theirs);
    let err = tokio::time::timeout(
        Duration::from_secs(60),
        connect_sealed(
            &relay,
            NAME,
            CONNECT_TOKEN,
            read,
            write,
            Resume::default(),
            &op_seal,
        ),
    )
    .await
    .expect("an answer in time")
    .unwrap_err();
    let mut got = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(1), ours.read_to_end(&mut got)).await;
    assert_eq!(
        podnet::exit::code_of(&err),
        podnet::exit::UNAVAILABLE,
        "{err:#}"
    );
    assert!(format!("{err:#}").contains("handshake"), "{err:#}");
    assert!(got.is_empty(), "the operator got {} bytes", got.len());
    node.abort();
}

// T-087: a node with --seal and an operator with no --seal: the session
// ends in time, the operator gets the bytes of the handshake only, and the
// target gets no connection.
#[tokio::test(flavor = "multi_thread")]
async fn a_plain_operator_and_a_sealed_node_get_nothing() {
    crate::test_host::need_loopback!();
    let (url, state) = fake_relay(None).await;
    let (_a, node_key) = key();
    let (_b, op_key) = key();
    let node_seal = seal(&node_key, &[&op_key]);
    let (addr, count) = counted_echo().await;
    let target = Target::parse(&addr).unwrap();
    let (_dir, pair) = pair_of(&url);
    let node = tokio::spawn(async move { run_node_sealed(None, pair, &target, &node_seal).await });
    wait_for_node(&state, 1).await;
    let relay = Relay::new(&url, None).unwrap();
    let (ours, theirs) = tokio::io::duplex(1 << 16);
    let session = tokio::spawn(async move {
        let (read, write) = tokio::io::split(theirs);
        connect_resumable(&relay, NAME, CONNECT_TOKEN, read, write, Resume::default()).await
    });
    let (mut read, mut write) = tokio::io::split(ours);
    write
        .write_all(b"plain bytes of an operator with no seal")
        .await
        .unwrap();
    write.shutdown().await.unwrap();
    let mut got = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(60), read.read_to_end(&mut got)).await;
    let ended = tokio::time::timeout(Duration::from_secs(60), session)
        .await
        .expect("the session ends")
        .unwrap();
    eprintln!("ended {ended:?}, got {} bytes", got.len());
    assert!(ended.is_ok(), "{ended:?}");
    assert!(!holds(&got, b"plain bytes"), "the echo came back in plain");
    assert_eq!(
        count.load(Ordering::SeqCst),
        0,
        "the target got a connection"
    );
    node.abort();
}
