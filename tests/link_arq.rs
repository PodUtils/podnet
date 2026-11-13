//! The selective-repeat ARQ under a link of podnet (T-277): the glue
//! `podnet::link_arq` over the in-process lossy shim of `pod-link`, on a
//! paused clock. A record is cut into fragments with their count; a reset
//! stream closes alone and the link goes on; a silent carrier ends the link
//! with exit 124; the sequence waits at its wrap until the glue answers the
//! rekey, and a failed rekey keeps it there. With the feature `dns`, the dns
//! carrier of podnet runs its own ARQ under a link at 3 % loss.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pod_link::arq::wire::Datagram;
use pod_link::datagram::lossy::{Faults, Lossy};
use pod_link::datagram::memory::{self, MemoryDatagrams};
use pod_link::datagram::DatagramFuture;
use pod_link::{ResetCode, Role, Settings};
use podnet::exit::code_of;
use podnet::link_arq::{self, ArqLink, ArqSettings, Datagrams, Rekey};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::time::timeout;

/// The bound of each wait on the paused clock: a stop fails the test and
/// does not hang it.
const LIMIT: Duration = Duration::from_secs(600);

/// 3 % of the datagrams lost, 1 % reordered, 1 % sent two times.
const GATE: Faults = Faults {
    loss: 0.03,
    reorder: 0.01,
    duplicate: 0.01,
};

type Shim = Arc<Lossy<MemoryDatagrams>>;

fn shims(faults: Faults, seed: u64) -> (Shim, Shim) {
    let (a, b) = memory::pair(memory::DEFAULT_MTU, memory::DEFAULT_DEPTH);
    (
        Arc::new(Lossy::new(a, faults, seed)),
        Arc::new(Lossy::new(b, faults, seed ^ 0x5eed)),
    )
}

/// A dialer and a listener over the ARQ over `near` and `far`.
fn links<D: Datagrams>(
    near: D,
    far: D,
    arq: ArqSettings,
    rekeys: (Rekey, Rekey),
) -> (ArqLink, ArqLink) {
    let s = Settings::default();
    let dialer = link_arq::over(near, Role::Dialer, arq, s, rekeys.0).unwrap();
    let listener = link_arq::over(far, Role::Listener, arq, s, rekeys.1).unwrap();
    (dialer, listener)
}

/// Bytes from a seed, so that a lost, doubled or moved fragment shows.
fn bytes(n: usize, seed: u64) -> Vec<u8> {
    let mut rng = pod_link::datagram::lossy::SplitMix64(seed);
    (0..n / 8)
        .flat_map(|_| rng.next_u64().to_le_bytes())
        .collect()
}

async fn send_all(mut to: impl AsyncWrite + Unpin, data: Vec<u8>) {
    for chunk in data.chunks(40_007) {
        to.write_all(chunk).await.unwrap();
    }
    to.shutdown().await.unwrap();
}

/// The SHA-256 and the count of each byte to the end.
async fn digest(mut from: impl AsyncRead + Unpin) -> ([u8; 32], usize) {
    let mut hash = Sha256::new();
    let mut buf = vec![0; 50_000];
    let mut n = 0;
    loop {
        let got = from.read(&mut buf).await.unwrap();
        if got == 0 {
            return (hash.finalize().into(), n);
        }
        hash.update(&buf[..got]);
        n += got;
    }
}

fn sha(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

/// `data` from the dialer to the listener on one stream: the SHA-256 and
/// the count that the listener read.
async fn carry(dialer: &ArqLink, listener: &ArqLink, data: Vec<u8>) -> ([u8; 32], usize) {
    let s = dialer.link.open().unwrap();
    let sender = tokio::spawn(send_all(s, data));
    let got = listener.link.accept().await.unwrap();
    let out = timeout(LIMIT, digest(got))
        .await
        .expect("the transfer in time");
    sender.await.unwrap();
    out
}

/// A carrier that keeps a copy of each datagram that it sends.
struct Tap {
    inner: MemoryDatagrams,
    sent: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl Datagrams for Tap {
    fn mtu(&self) -> usize {
        self.inner.mtu()
    }

    fn send<'a>(&'a self, datagram: &'a [u8]) -> DatagramFuture<'a, ()> {
        self.sent.lock().unwrap().push(datagram.to_vec());
        self.inner.send(datagram)
    }

    fn recv(&self) -> DatagramFuture<'_, Vec<u8>> {
        self.inner.recv()
    }
}

/// Each `DATA` frame of the dialer has a payload of 1 KiB at most and
/// fits the unit of the carrier. The fragments of a record have the
/// indexes 0 to count - 1 on sequences in a row, and a stream of 300 KB
/// gives records of more than one fragment.
#[tokio::test(start_paused = true)]
async fn a_record_is_cut_into_fragments_of_one_kib_with_their_count() {
    let (a, b) = memory::pair(memory::DEFAULT_MTU, memory::DEFAULT_DEPTH);
    let sent = Arc::new(Mutex::new(Vec::new()));
    let near = Tap {
        inner: a,
        sent: sent.clone(),
    };
    let far = Tap {
        inner: b,
        sent: Arc::new(Mutex::new(Vec::new())),
    };
    let s = Settings::default();
    let arq = ArqSettings::default();
    let dialer = link_arq::over(near, Role::Dialer, arq, s, Rekey::Above).unwrap();
    let listener = link_arq::over(far, Role::Listener, arq, s, Rekey::Above).unwrap();
    let data = bytes(300_000, 1);
    let (hash, n) = carry(&dialer, &listener, data.clone()).await;
    assert_eq!((n, hash), (data.len(), sha(&data)));
    let mut frames = BTreeMap::new();
    for d in sent.lock().unwrap().iter() {
        assert!(
            d.len() <= memory::DEFAULT_MTU,
            "a datagram of {} bytes",
            d.len()
        );
        if let Ok(Datagram::Data {
            seq,
            index,
            count,
            payload,
        }) = Datagram::decode(d)
        {
            assert!(
                payload.len() <= 1024,
                "a fragment of {} bytes",
                payload.len()
            );
            frames.insert(seq, (index, count));
        }
    }
    let mut records = 0;
    let mut longest = 0;
    let mut want = 0u16;
    let mut count_of_record = 0u16;
    for (index, count) in frames.values() {
        if want == 0 {
            count_of_record = *count;
        }
        assert_eq!(
            (*index, *count),
            (want, count_of_record),
            "the fragments of a record"
        );
        want += 1;
        if want == count_of_record {
            records += 1;
            longest = longest.max(count_of_record);
            want = 0;
        }
    }
    assert_eq!(want, 0, "the last record is whole");
    assert!(
        records > 1 && longest > 1,
        "{records} records, the longest {longest} fragments"
    );
}

/// At the faults of the gate, the dialer resets one stream of two: the
/// peer's read of it fails, the other stream carries 1 MiB whole, and the
/// link and the ARQ live on.
#[tokio::test(start_paused = true)]
async fn a_reset_stream_closes_alone_and_the_link_goes_on() {
    let (sa, sb) = shims(GATE, 11);
    let (dialer, listener) = links(
        sa.clone(),
        sb.clone(),
        ArqSettings::default(),
        (Rekey::Above, Rekey::Above),
    );
    let mut doomed = dialer.link.open().unwrap();
    doomed.write_all(b"the first bytes").await.unwrap();
    let mut gone = listener.link.accept().await.unwrap();
    let mut first = [0; 15];
    timeout(LIMIT, gone.read_exact(&mut first))
        .await
        .unwrap()
        .unwrap();
    doomed.reset(ResetCode::CANCEL);
    let read = timeout(LIMIT, gone.read(&mut [0; 16]))
        .await
        .expect("the reset in time");
    assert!(read.is_err(), "the read of a reset stream: {read:?}");
    let data = bytes(1 << 20, 2);
    let (hash, n) = carry(&dialer, &listener, data.clone()).await;
    assert_eq!((n, hash), (data.len(), sha(&data)));
    assert!(dialer.link.ended().is_none() && listener.link.ended().is_none());
    assert!(dialer.arq.error().is_none() && listener.arq.error().is_none());
    assert!(
        sa.stats().lost + sb.stats().lost > 10,
        "the shim lost datagrams"
    );
    assert!(dialer.arq.stats().retransmitted > 0);
}

/// The carrier goes silent: the link ends with the class of a timeout
/// (exit 124), its streams fail, and the ARQ ends with `arq-timeout`
/// (exit 124).
#[tokio::test(start_paused = true)]
async fn a_silent_carrier_ends_the_link_and_the_arq_with_exit_124() {
    let (sa, sb) = shims(Faults::NONE, 6);
    let (dialer, _listener) = links(
        sa.clone(),
        sb.clone(),
        ArqSettings::default(),
        (Rekey::Above, Rekey::Above),
    );
    let mut s = dialer.link.open().unwrap();
    s.write_all(b"hello").await.unwrap();
    sa.set_faults(Faults::SILENT);
    sb.set_faults(Faults::SILENT);
    let e = timeout(LIMIT, dialer.link.closed())
        .await
        .expect("the end in time");
    assert_eq!(code_of(&podnet::link::fault(e)), 124);
    assert!(s.read(&mut [0; 1]).await.is_err());
    let ended = timeout(LIMIT, async {
        loop {
            if let Some(e) = dialer.arq_error() {
                return e;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    })
    .await
    .expect("the arq-timeout in time");
    assert!(format!("{ended:#}").contains("arq-timeout"), "{ended:#}");
    assert_eq!(code_of(&ended), 124);
}

/// The first sequence 100 frames before the wrap, a rekey asked 256 frames
/// ahead.
fn near_the_wrap() -> ArqSettings {
    ArqSettings {
        start: u32::MAX - 99,
        rekey_ahead: 256,
        ..ArqSettings::default()
    }
}

/// With `Rekey::Above` the glue answers the rekey of each end, each
/// sequence wraps one time, and 1 MiB crosses the wrap whole at 3 % loss.
#[tokio::test(start_paused = true)]
async fn the_glue_answers_the_rekey_and_the_bytes_cross_the_wrap() {
    let (sa, sb) = shims(GATE, 21);
    let (dialer, listener) = links(sa, sb, near_the_wrap(), (Rekey::Above, Rekey::Above));
    let data = bytes(1 << 20, 3);
    let (hash, n) = carry(&dialer, &listener, data.clone()).await;
    assert_eq!((n, hash), (data.len(), sha(&data)));
    assert_eq!(dialer.rekeys(), 1);
    assert_eq!(
        dialer.arq.stats().rekeys,
        1,
        "the send sequence wrapped one time"
    );
    assert!(dialer.rekey_failure().is_none());
}

/// A hook runs one time for the wrap of its end before the sequence wraps.
#[tokio::test(start_paused = true)]
async fn a_rekey_hook_runs_before_the_wrap() {
    let (sa, sb) = shims(Faults::NONE, 22);
    let calls = Arc::new(AtomicU64::new(0));
    let seen = calls.clone();
    let hook = Rekey::With(Box::new(move || {
        let seen = seen.clone();
        Box::pin(async move {
            seen.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    }));
    let (dialer, listener) = links(sa, sb, near_the_wrap(), (hook, Rekey::Above));
    let data = bytes(512 << 10, 4);
    let (hash, n) = carry(&dialer, &listener, data.clone()).await;
    assert_eq!((n, hash), (data.len(), sha(&data)));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!((dialer.rekeys(), dialer.arq.stats().rekeys), (1, 1));
}

/// A hook that fails keeps each new frame of its end at the wrap: no
/// sequence is used again, the bytes after the wrap do not come, and the
/// keepalive ends the link: the dialer sees a timeout (exit 124), or the
/// end of the carrier when the listener ended first (exit 69).
#[tokio::test(start_paused = true)]
async fn a_failed_rekey_keeps_the_frames_at_the_wrap_and_the_link_ends() {
    let (sa, sb) = shims(Faults::NONE, 23);
    let hook = Rekey::With(Box::new(|| {
        Box::pin(async { Err(anyhow::anyhow!("no new key")) })
    }));
    let (dialer, listener) = links(sa, sb, near_the_wrap(), (hook, Rekey::Above));
    let s = dialer.link.open().unwrap();
    let data = bytes(512 << 10, 5);
    tokio::spawn(async move {
        let mut s = s;
        let _ = s.write_all(&data).await;
    });
    let got = listener.link.accept().await;
    let e = timeout(LIMIT, dialer.link.closed())
        .await
        .expect("the end in time");
    let code = code_of(&podnet::link::fault(e));
    assert!(code == 124 || code == 69, "exit {code}");
    if let Ok(mut got) = got {
        let mut buf = Vec::new();
        let read = got.read_to_end(&mut buf).await;
        assert!(
            read.is_err() && buf.len() < 512 << 10,
            "{} bytes, {read:?}",
            buf.len()
        );
    }
    assert_eq!(dialer.rekey_failure().as_deref(), Some("no new key"));
    assert_eq!((dialer.rekeys(), dialer.arq.stats().rekeys), (0, 0));
    assert!(dialer.arq.stats().sent <= 100, "no frame past the wrap");
}

/// A setting of the ARQ out of its limits exits 64 before a frame goes.
#[tokio::test]
async fn a_setting_out_of_its_limits_exits_64() {
    let (a, _b) = memory::pair(memory::DEFAULT_MTU, memory::DEFAULT_DEPTH);
    let bad = ArqSettings {
        window: 0,
        ..ArqSettings::default()
    };
    let e = link_arq::over(a, Role::Dialer, bad, Settings::default(), Rekey::Above)
        .err()
        .expect("a window of 0 is refused");
    assert_eq!(code_of(&e), 64);
    assert!(format!("{e:#}").contains("window"), "{e:#}");
}

/// The dns carrier of podnet runs the ARQ of `pod-link` under its TLS:
/// two streams of a link cross it at 3 % loss with reorder and duplicates,
/// and a reset of one stream leaves the other and the link.
#[cfg(feature = "dns")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_dns_carrier_runs_its_own_arq_under_a_link_at_three_percent_loss() {
    use pod_core::Target;
    use pod_identity::KeyName;
    use podnet::identity::{self, AllowList};
    use podnet::link_keyed::Tune;

    let dir = tempfile::tempdir().unwrap();
    let mk = |name: &str| {
        let sub = dir.path().join(name);
        std::fs::create_dir(&sub).unwrap();
        identity::create(&sub).unwrap()
    };
    let (a, b) = (mk("a"), mk("b"));
    let allow = AllowList::parse(&a.fingerprint()).unwrap();
    let listening = podnet::link_dns::carrier(&b, &allow, &Tune::default());
    let dialing = podnet::link_dns::carrier(&a, &AllowList::default(), &Tune::default());
    let (near, far) = shims(GATE, 31);
    let target = Target::Name {
        host: "l.test".into(),
        port: 53,
    };
    let key = KeyName::Key(b.public());
    let (dialed, accepted) = timeout(Duration::from_secs(60), async {
        tokio::join!(
            dialing.connect_over(near.clone(), &target, &key),
            listening.accept_over(far.clone(), &"the peer")
        )
    })
    .await
    .expect("the handshake in time");
    let s = Settings::default();
    let dialer = podnet::link::from_io(dialed.unwrap().0, Role::Dialer, s).unwrap();
    let listener = podnet::link::from_io(accepted.unwrap().0, Role::Listener, s).unwrap();
    let doomed = dialer.open().unwrap();
    let data = bytes(256 << 10, 6);
    let kept = dialer.open().unwrap();
    let kept_id = kept.id();
    doomed.reset(ResetCode::CANCEL);
    let sender = tokio::spawn(send_all(kept, data.clone()));
    let got = loop {
        let got = timeout(Duration::from_secs(60), listener.accept())
            .await
            .unwrap()
            .unwrap();
        if got.id() == kept_id {
            break got;
        }
    };
    let whole = timeout(Duration::from_secs(120), digest(got))
        .await
        .expect("the transfer in time");
    sender.await.unwrap();
    assert_eq!(whole, (sha(&data), data.len()));
    assert!(dialer.ended().is_none() && listener.ended().is_none());
    assert!(
        near.stats().lost + far.stats().lost > 0,
        "the shim lost datagrams"
    );
}
