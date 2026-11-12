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
