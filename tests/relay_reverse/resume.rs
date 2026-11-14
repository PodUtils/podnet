//! T-087: the resumable layer on the reverse road. `relay connect --resume`
//! moves a session to a new leg before the cap of the relay, a session
//! lasts past the 12 h of one relay session on a fake clock, and a node and
//! an operator that do not agree on the layer fail with a clear error or
//! keep the plain mode.

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use pod_relay::reverse::{Handler, Opening, SessionId};
use podnet::relay_reverse::{
    carry, connect, connect_resumable, layered, load, run_node, run_node_resumable, Resume, Target,
};
use podnet::ws_relay::Relay;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _, DuplexStream};

use super::{echo, fake_relay, ms_from_now, wait_for_node, write_pair, CONNECT_TOKEN, NAME};

const MIB: usize = 1 << 20;

pub(crate) fn sha256(bytes: &[u8]) -> Vec<u8> {
    ring::digest::digest(&ring::digest::SHA256, bytes)
        .as_ref()
        .to_vec()
}

pub(crate) fn pseudo_random(len: usize, seed: u32) -> Vec<u8> {
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

/// A pair of the fake relay `url` in a new directory.
pub(crate) fn pair_of(url: &str) -> (tempfile::TempDir, podnet::relay_reverse::Pair) {
    let dir = tempfile::tempdir().unwrap();
    let expires = ms_from_now(Duration::from_secs(3600));
    write_pair(dir.path(), NAME, url, expires, "");
    let pair = load(dir.path(), Some(NAME)).unwrap();
    (dir, pair)
}

// T-087, the Prove: the fake relay closes each relay session at 1 MiB (both
// ways, with 1009, as the relay at 64 MiB). The session moves to a new leg
// before the cap, and carries 8 MiB in each direction through an echo with
// equal SHA-256.
#[tokio::test(flavor = "multi_thread")]
async fn relay_session_moves_past_the_cap() {
    crate::test_host::need_loopback!();
    let (url, state) = fake_relay(None).await;
    state.lock().unwrap().cap = Some(MIB);
    let target = Target::parse(&echo().await).unwrap();
    let (_dir, pair) = pair_of(&url);
    let node = tokio::spawn(async move { run_node_resumable(None, pair, &target).await });
    wait_for_node(&state, 1).await;
    let relay = Relay::new(&url, None).unwrap();
    let (ours, theirs) = tokio::io::duplex(1 << 16);
    let resume = Resume {
        move_bytes: 256 << 10,
        ..Resume::default()
    };
    let session = tokio::spawn(async move {
        let (read, write) = tokio::io::split(theirs);
        connect_resumable(&relay, NAME, CONNECT_TOKEN, read, write, resume).await
    });
    let sent = pseudo_random(8 * MIB, 0x1234_5678);
    let (mut read, mut write) = tokio::io::split(ours);
    let writer = {
        let sent = sent.clone();
        tokio::spawn(async move {
            for chunk in sent.chunks(64 << 10) {
                write.write_all(chunk).await.unwrap();
            }
            write
        })
    };
    let mut got = vec![0u8; sent.len()];
    tokio::time::timeout(Duration::from_secs(120), read.read_exact(&mut got))
        .await
        .expect("8 MiB back in time")
        .expect("8 MiB back");
    assert_eq!(sha256(&got), sha256(&sent), "the bytes of the echo");
    // The end of the input ends the session, with no error.
    let mut write = writer.await.unwrap();
    write.shutdown().await.unwrap();
    let ended = tokio::time::timeout(Duration::from_secs(60), session)
        .await
        .expect("the session ends")
        .unwrap();
    let (legs, capped) = {
        let state = state.lock().unwrap();
        (state.operator_sockets, state.capped)
    };
    eprintln!("legs {legs}, legs at the cap {capped}");
    assert!(ended.is_ok(), "{ended:?}");
    assert!(legs >= 16, "16 MiB at 1 MiB a leg: {legs} legs");
    node.abort();
}

/// Each session echoes what it takes.
struct Echo;

impl Handler for Echo {
    type Stream = DuplexStream;

    fn open(&self, _id: SessionId) -> Opening<DuplexStream> {
        Box::pin(async {
            let (ours, theirs) = tokio::io::duplex(1 << 16);
            tokio::spawn(async move {
                let (mut r, mut w) = tokio::io::split(theirs);
                let _ = tokio::io::copy(&mut r, &mut w).await;
            });
            Ok(ours)
        })
    }
}

/// A leg through a stand-in relay that ends it after `age`, as the relay
/// ends a session at 12 h.
fn aged_leg(far: DuplexStream, age: Duration) -> DuplexStream {
    let (ours, mut relay) = tokio::io::duplex(1 << 16);
    tokio::spawn(async move {
        let mut far = far;
        tokio::select! {
            _ = tokio::io::copy_bidirectional(&mut relay, &mut far) => {}
            () = tokio::time::sleep(age) => {}
        }
    });
    ours
}

// T-087: one session of 13 h on a fake clock (the paused clock of tokio).
// Each relay session ends at 12 h. The session moves to a new leg at 11 h
// and loses no leg, and each echo comes back.
#[tokio::test(start_paused = true)]
async fn relay_session_lasts_12_h_on_a_fake_clock() {
    const RELAY_AGE: Duration = Duration::from_secs(12 * 3600);
    let node = Arc::new(layered(Echo));
    let ids = Arc::new(AtomicU32::new(0));
    let open = {
        let (node, ids) = (node.clone(), ids.clone());
        move || {
            let (node, ids) = (node.clone(), ids.clone());
            async move {
                let n = ids.fetch_add(1, Ordering::SeqCst) + 1;
                let id = SessionId::parse(format!("{n:032x}").as_bytes()).unwrap();
                aged_leg(node.open(id).await.unwrap(), RELAY_AGE)
            }
        }
    };
    let (moves, losses) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    let next = {
        let (open, moves, losses) = (open.clone(), moves.clone(), losses.clone());
        move |moving: bool| {
            let count = if moving { &moves } else { &losses };
            count.fetch_add(1, Ordering::SeqCst);
            let open = open.clone();
            async move { pod_session::resume::Next::Link(open().await) }
        }
    };
    let first = open().await;
    let (mut app, app_end) = tokio::io::duplex(1 << 16);
    let session = tokio::spawn(carry(first, app_end, Resume::default().settings(), next));
    let start = tokio::time::Instant::now();
    let mut echoes = 0;
    while start.elapsed() < Duration::from_secs(13 * 3600) {
        tokio::time::sleep(Duration::from_secs(1800)).await;
        let word = pseudo_random(1024, echoes + 1);
        app.write_all(&word).await.unwrap();
        let mut back = vec![0u8; word.len()];
        tokio::time::timeout(Duration::from_secs(60), app.read_exact(&mut back))
            .await
            .expect("the echo in time")
            .unwrap();
        assert_eq!(back, word, "echo {echoes}");
        echoes += 1;
    }
    app.shutdown().await.unwrap();
    let mut rest = Vec::new();
    let _ = app.read_to_end(&mut rest).await;
    let ended = tokio::time::timeout(Duration::from_secs(120), session)
        .await
        .expect("the session ends")
        .unwrap();
    assert!(ended.is_ok(), "{ended:?}");
    assert_eq!(echoes, 26);
    assert_eq!(losses.load(Ordering::SeqCst), 0, "no leg met the 12 h");
    assert!(moves.load(Ordering::SeqCst) >= 1, "a move at 11 h");
}

// T-087: a node of the layer and an operator with no --resume: the operator
// exits 78 with the cause, not with the bytes of the layer.
#[tokio::test(flavor = "multi_thread")]
async fn a_plain_operator_learns_that_the_node_resumes() {
    crate::test_host::need_loopback!();
    let (url, state) = fake_relay(None).await;
    let target = Target::parse(&echo().await).unwrap();
    let (_dir, pair) = pair_of(&url);
    let node = tokio::spawn(async move { run_node_resumable(None, pair, &target).await });
    wait_for_node(&state, 1).await;
    let relay = Relay::new(&url, None).unwrap();
    let (ours, theirs) = tokio::io::duplex(1 << 16);
    let (read, write) = tokio::io::split(theirs);
    let err = tokio::time::timeout(
        Duration::from_secs(30),
        connect(&relay, NAME, CONNECT_TOKEN, read, write),
    )
    .await
    .expect("an answer in time")
    .unwrap_err();
    drop(ours);
    assert_eq!(podnet::exit::code_of(&err), podnet::exit::CONFIG, "{err:#}");
    assert!(format!("{err:#}").contains("--resume"), "{err:#}");
    node.abort();
}

/// A TCP server that says a line first, as sshd does, and then echoes.
async fn banner_echo() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            tokio::spawn(async move {
                let _ = socket.write_all(b"SSH-2.0-test\r\n").await;
                let (mut r, mut w) = socket.split();
                let _ = tokio::io::copy(&mut r, &mut w).await;
            });
        }
    });
    addr
}

// T-087: an operator with --resume and a node with no layer: the session
// runs in the plain mode.
#[tokio::test(flavor = "multi_thread")]
async fn a_resume_operator_keeps_the_plain_mode_with_a_plain_node() {
    crate::test_host::need_loopback!();
    let (url, state) = fake_relay(None).await;
    let target = Target::parse(&banner_echo().await).unwrap();
    let (_dir, pair) = pair_of(&url);
    let node = tokio::spawn(async move { run_node(None, pair, &target).await });
    wait_for_node(&state, 1).await;
    let relay = Relay::new(&url, None).unwrap();
    let (ours, theirs) = tokio::io::duplex(1 << 16);
    let session = tokio::spawn(async move {
        let (read, write) = tokio::io::split(theirs);
        connect_resumable(&relay, NAME, CONNECT_TOKEN, read, write, Resume::default()).await
    });
    let (mut read, mut write) = tokio::io::split(ours);
    let mut banner = [0u8; 14];
    tokio::time::timeout(Duration::from_secs(20), read.read_exact(&mut banner))
        .await
        .expect("the banner in time")
        .unwrap();
    assert_eq!(&banner, b"SSH-2.0-test\r\n");
    write.write_all(b"plain").await.unwrap();
    let mut back = [0u8; 5];
    tokio::time::timeout(Duration::from_secs(20), read.read_exact(&mut back))
        .await
        .expect("the echo in time")
        .unwrap();
    assert_eq!(&back, b"plain");
    drop((read, write));
    let ended = tokio::time::timeout(Duration::from_secs(40), session)
        .await
        .expect("the session ends")
        .unwrap();
    assert!(ended.is_ok(), "{ended:?}");
    node.abort();
}
