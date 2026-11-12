//! The link of podnet over a carrier (T-262): many streams over one
//! carrier, with credits and a keepalive. The multiplexer is `pod-link`
//! (podcrates call C120 to C124); these tests prove the glue and the
//! rules of GitHub #466, row E0-2. They bind no TCP port.

use std::sync::Arc;
use std::time::Duration;

use pod_link::memory::Memory;
use pod_link::Error;
use podnet::link::{self, Link, Role, Settings};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::timeout;

const LIMIT: Duration = Duration::from_secs(30);

/// Two links over one memory carrier: (dialer, listener).
async fn pair(settings: Settings) -> (Link, Link) {
    let carrier = Memory::default();
    let mut door = link::listen(&carrier, "session").await.unwrap();
    let (dialer, listener) = tokio::join!(
        link::dial(&carrier, "session", settings),
        door.accept(settings)
    );
    (dialer.unwrap(), listener.unwrap())
}

/// Echo each stream of `link` until the link ends.
fn echo(link: Arc<Link>) {
    tokio::spawn(async move {
        while let Ok(mut stream) = link.accept().await {
            tokio::spawn(async move {
                let (mut r, mut w) = tokio::io::split(&mut stream);
                let _ = tokio::io::copy(&mut r, &mut w).await;
                let _ = w.shutdown().await;
            });
        }
    });
}

#[tokio::test]
async fn the_dialer_has_odd_ids_and_the_listener_even_ids() {
    let (dialer, listener) = pair(Settings::default()).await;
    assert_eq!(dialer.role(), Role::Dialer);
    assert_eq!(listener.role(), Role::Listener);
    let a = dialer.open().unwrap();
    let b = listener.open().unwrap();
    assert_eq!(a.id() % 2, 1);
    assert_eq!(b.id() % 2, 0);
}

#[tokio::test]
async fn three_hundred_echoes_cross_one_link() {
    let settings = Settings {
        max_streams: 512,
        ..Settings::default()
    };
    let (dialer, listener) = pair(settings).await;
    echo(Arc::new(listener));
    let dialer = Arc::new(dialer);
    let mut tasks = Vec::new();
    for n in 0..300u32 {
        let dialer = dialer.clone();
        tasks.push(tokio::spawn(async move {
            let mut s = dialer.open().unwrap();
            let data = vec![(n % 251) as u8; 40_000 + n as usize];
            let (mut r, mut w) = tokio::io::split(&mut s);
            let send = data.clone();
            let writer = async move {
                w.write_all(&send).await.unwrap();
                w.shutdown().await.unwrap();
            };
            let reader = async {
                let mut got = Vec::new();
                r.read_to_end(&mut got).await.unwrap();
                got
            };
            let (_, got) = tokio::join!(writer, reader);
            assert!(got == data, "echo {n} differs");
        }));
    }
    for t in tasks {
        timeout(LIMIT, t).await.unwrap().unwrap();
    }
}

#[tokio::test]
async fn a_stalled_reader_does_not_stop_the_other_echoes() {
    let settings = Settings {
        max_streams: 512,
        ..Settings::default()
    };
    let (dialer, listener) = pair(settings).await;
    let listener = Arc::new(listener);
    // The first stream of the dialer: its far reader never reads.
    let mut stalled = dialer.open().unwrap();
    let stalled_far = listener.accept().await.unwrap();
    // Write more than the windows hold: the write must wait.
    let big = vec![7u8; 8 << 20];
    let blocked = timeout(Duration::from_millis(500), stalled.write_all(&big)).await;
    assert!(blocked.is_err(), "the write to a stalled reader must wait");
    echo(listener.clone());
    let dialer = Arc::new(dialer);
    let mut tasks = Vec::new();
    for n in 0..299u32 {
        let dialer = dialer.clone();
        tasks.push(tokio::spawn(async move {
            let mut s = dialer.open().unwrap();
            let msg = format!("echo {n}").into_bytes();
            s.write_all(&msg).await.unwrap();
            s.shutdown().await.unwrap();
            let mut got = Vec::new();
            s.read_to_end(&mut got).await.unwrap();
            assert_eq!(got, msg);
        }));
    }
    for t in tasks {
        timeout(LIMIT, t).await.unwrap().unwrap();
    }
    drop(stalled_far);
}

#[tokio::test]
async fn a_silent_peer_ends_the_link_as_dead() {
    // The far end is held and never read: no PONG comes back.
    let (near, _far) = pod_link::memory::pair(pod_link::memory::MIN_RING);
    let settings = Settings {
        keepalive: Duration::from_millis(50),
        dead_after: Duration::from_millis(200),
        ..Settings::default()
    };
    let link = link::from_io(near, Role::Dialer, settings).unwrap();
    let e = timeout(LIMIT, link.closed()).await.unwrap();
    assert!(matches!(e, Error::Dead(_)), "got {e:?}");
}

#[tokio::test]
async fn a_live_peer_keeps_the_link_and_answers_the_ping() {
    let settings = Settings {
        keepalive: Duration::from_millis(50),
        dead_after: Duration::from_millis(500),
        ..Settings::default()
    };
    let (dialer, _listener) = pair(settings).await;
    tokio::time::sleep(Duration::from_millis(900)).await;
    assert!(dialer.ended().is_none(), "a live peer keeps the link");
    assert!(dialer.rtt().is_some(), "a PONG came back");
}

#[tokio::test]
async fn a_setting_out_of_its_limit_is_a_usage_error() {
    let carrier = Memory::default();
    let mut door = link::listen(&carrier, "bad").await.unwrap();
    let settings = Settings {
        max_streams: 5000,
        ..Settings::default()
    };
    let (d, _l) = tokio::join!(
        link::dial(&carrier, "bad", settings),
        door.accept(Settings::default())
    );
    let e = d.err().unwrap();
    assert_eq!(podnet::exit::code_of(&e), podnet::exit::USAGE);
}

// The resume of a link through `pod-session` (T-263): the carrier is lost,
// the session keeps the stream table for 10 minutes, and a new carrier
// resumes it. The clock of the far end is the `now` of `Far::expire_at`.
mod resume {
    use std::sync::atomic::{AtomicI64, Ordering};
    use std::sync::Mutex;

    use podnet::exit::code_of;
    use podnet::session::{self, Far, Next, Note, ResumeExpired, Session, RESUME_WINDOW};
    use tokio::io::{AsyncRead, AsyncWrite, DuplexStream};
    use tokio::sync::{mpsc, oneshot};
    use tokio::time::Instant;

    use super::*;

    const PIPE: usize = 256 * 1024;
    const MIB: usize = 1 << 20;

    fn sha256(bytes: &[u8]) -> Vec<u8> {
        ring::digest::digest(&ring::digest::SHA256, bytes)
            .as_ref()
            .to_vec()
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

    /// Copy until `left` bytes (both ways together) have passed; the rest
    /// never goes through.
    async fn copy_until<R, W>(from: &mut R, to: &mut W, left: &AtomicI64)
    where
        R: AsyncRead + Unpin,
        W: AsyncWrite + Unpin,
    {
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            let n = match from.read(&mut buf).await {
                Ok(0) | Err(_) => return,
                Ok(n) => n,
            };
            let before = left.fetch_sub(n as i64, Ordering::SeqCst);
            if before <= n as i64 {
                let _ = to.write_all(&buf[..before.max(0) as usize]).await;
                return;
            }
            if to.write_all(&buf[..n]).await.is_err() {
                return;
            }
        }
    }

    /// A carrier through a stand-in host that drops both ends, with no word,
    /// after `cut` bytes. `None` never cuts.
    fn relayed(cut: Option<usize>) -> (DuplexStream, DuplexStream) {
        let (client, relay_client) = tokio::io::duplex(PIPE);
        let (relay_far, far) = tokio::io::duplex(PIPE);
        tokio::spawn(async move {
            let left = AtomicI64::new(cut.map_or(i64::MAX, |c| c as i64));
            let (mut cr, mut cw) = tokio::io::split(relay_client);
            let (mut fr, mut fw) = tokio::io::split(relay_far);
            tokio::select! {
                _ = copy_until(&mut cr, &mut fw, &left) => {}
                _ = copy_until(&mut fr, &mut cw, &left) => {}
            }
        });
        (client, far)
    }

    /// A far end that takes each carrier from the channel; the link of a new
    /// session goes to the receiver.
    fn far_end(
        far: Arc<Far>,
    ) -> (
        mpsc::UnboundedSender<DuplexStream>,
        mpsc::UnboundedReceiver<Link>,
    ) {
        let (carriers, mut incoming) = mpsc::unbounded_channel::<DuplexStream>();
        let (links, out) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            while let Some(io) = incoming.recv().await {
                let (far, links) = (far.clone(), links.clone());
                tokio::spawn(async move {
                    let settings = Settings::default();
                    if let Ok(Some(link)) = far.accept(io, settings).await {
                        let _ = links.send(link);
                    }
                });
            }
        });
        (carriers, out)
    }

    /// A session whose first carrier is cut after `cut` bytes. A new
    /// carrier comes only after the returned gate is released.
    async fn cut_session(
        far: &Arc<Far>,
        cut: usize,
        window: Duration,
    ) -> (Session, Link, oneshot::Sender<()>, Arc<Mutex<Vec<Note>>>) {
        let (carriers, mut links) = far_end(far.clone());
        let (client, far_side) = relayed(Some(cut));
        carriers.send(far_side).unwrap();
        let (gate, gate_rx) = oneshot::channel::<()>();
        let gate_rx = Arc::new(tokio::sync::Mutex::new(Some(gate_rx)));
        let connect = move |_: &session::Ended| {
            let (gate_rx, carriers) = (gate_rx.clone(), carriers.clone());
            async move {
                if let Some(rx) = gate_rx.lock().await.take() {
                    let _ = rx.await;
                }
                let (client, far_side) = relayed(None);
                carriers.send(far_side).unwrap();
                Next::Link(client)
            }
        };
        let notes: Arc<Mutex<Vec<Note>>> = Arc::default();
        let kept = notes.clone();
        let session = Session::dial(client, connect, window, Settings::default(), move |n| {
            kept.lock().unwrap().push(n)
        })
        .await
        .unwrap();
        let listener = timeout(LIMIT, links.recv()).await.unwrap().unwrap();
        (session, listener, gate, notes)
    }

    /// Wait until the far end saw the loss of a carrier.
    async fn lost(far: &Far) {
        for _ in 0..200 {
            if far.lost() > 0 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("the far end never saw the loss of the carrier");
    }

    #[test]
    fn the_window_is_ten_minutes_and_the_link_outlives_it() {
        assert_eq!(RESUME_WINDOW, Duration::from_secs(600));
        assert_eq!(
            session::settings(RESUME_WINDOW).resume_deadline,
            Duration::from_secs(600)
        );
        let link = session::link_settings(Settings::default(), RESUME_WINDOW);
        assert!(link.dead_after >= RESUME_WINDOW, "{:?}", link.dead_after);
        assert!(link.dead_after > link.keepalive);
        link.check().unwrap();
        // A longer dead time of the caller stays.
        let long = Settings {
            dead_after: Duration::from_secs(900),
            ..Settings::default()
        };
        assert_eq!(
            session::link_settings(long, RESUME_WINDOW).dead_after,
            Duration::from_secs(900)
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn resume_inside_the_window_carries_one_mebibyte_with_an_equal_digest() {
        let far = Arc::new(Far::new(RESUME_WINDOW));
        let (session, listener, gate, notes) = cut_session(&far, 300_000, RESUME_WINDOW).await;
        echo(Arc::new(listener));
        let data = pseudo_random(MIB, 7);
        let mut stream = session.link().open().unwrap();
        let send = data.clone();
        let (mut r, mut w) = tokio::io::split(&mut stream);
        let writer = async move {
            w.write_all(&send).await.unwrap();
            w.shutdown().await.unwrap();
        };
        let reader = async {
            let mut got = Vec::new();
            r.read_to_end(&mut got).await.unwrap();
            got
        };
        let run = async {
            // The carrier is lost; the far end keeps the session.
            lost(&far).await;
            // 5 s before the end of the window the session is kept.
            let inside = Instant::now() + RESUME_WINDOW - Duration::from_secs(5);
            assert_eq!(far.expire_at(inside).await, 0);
            gate.send(()).unwrap();
        };
        let (_, got, _) = timeout(LIMIT, async { tokio::join!(writer, reader, run) })
            .await
            .expect("the bytes came through");
        assert_eq!(got.len(), MIB);
        assert_eq!(sha256(&got), sha256(&data), "the digest differs");
        assert!(session.link().ended().is_none(), "the link did not end");
        let notes = notes.lock().unwrap().clone();
        assert!(
            notes.iter().any(|n| matches!(n, Note::Lost { .. })),
            "{notes:?}"
        );
        assert!(
            notes.iter().any(|n| matches!(n, Note::Resumed { .. })),
            "{notes:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn resume_after_the_window_is_refused_with_resume_expired() {
        let far = Arc::new(Far::new(RESUME_WINDOW));
        let (session, listener, gate, _notes) = cut_session(&far, 300_000, RESUME_WINDOW).await;
        echo(Arc::new(listener));
        let stream = session.link().open().unwrap();
        let big = vec![3u8; 2 * MIB];
        // The echo cannot end: the carrier is cut. It is not awaited.
        tokio::spawn(async move {
            let (mut r, mut w) = tokio::io::split(stream);
            let write = async {
                let _ = w.write_all(&big).await;
            };
            let read = async {
                let mut sink = Vec::new();
                let _ = r.read_to_end(&mut sink).await;
            };
            tokio::join!(write, read);
        });
        lost(&far).await;
        // The clock of the far end runs past the window.
        let after = Instant::now() + RESUME_WINDOW + Duration::from_secs(1);
        assert_eq!(far.expire_at(after).await, 1, "the far end forgets it");
        gate.send(()).unwrap();
        let error = timeout(LIMIT, session.finish()).await.unwrap().unwrap_err();
        assert_eq!(code_of(&error), podnet::exit::UNAVAILABLE, "{error:#}");
        let expired = error
            .chain()
            .find_map(|e| e.downcast_ref::<ResumeExpired>())
            .unwrap_or_else(|| panic!("not ResumeExpired: {error:#}"));
        assert_eq!(expired.window, RESUME_WINDOW);
        assert!(format!("{error:#}").contains("600 s"), "{error:#}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn resume_expires_at_the_client_when_no_carrier_comes() {
        let window = Duration::from_secs(2);
        let far = Arc::new(Far::new(window));
        let (carriers, mut links) = far_end(far.clone());
        let (client, far_side) = relayed(Some(5_000));
        carriers.send(far_side).unwrap();
        let connect = |_: &session::Ended| async { Next::<DuplexStream>::Retry("no route".into()) };
        let session = Session::dial(client, connect, window, Settings::default(), |_| {})
            .await
            .unwrap();
        let _listener = timeout(LIMIT, links.recv()).await.unwrap().unwrap();
        let s = session.link().open().unwrap();
        tokio::spawn(async move {
            let mut s = s;
            let _ = s.write_all(&vec![1u8; MIB]).await;
        });
        let error = timeout(LIMIT, session.finish()).await.unwrap().unwrap_err();
        let expired = error
            .chain()
            .find_map(|e| e.downcast_ref::<ResumeExpired>())
            .unwrap_or_else(|| panic!("not ResumeExpired: {error:#}"));
        assert_eq!(expired.window, window);
        assert_eq!(code_of(&error), podnet::exit::UNAVAILABLE);
    }
}
