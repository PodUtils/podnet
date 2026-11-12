//! The in-process memory carrier of the link (T-275): a ring for each
//! direction, a full ring blocks the writer and truncates nothing, and the
//! end of one side is the end of stream of the other. The tests bind no TCP
//! port, so they run on the restricted host and on the host with no
//! loopback.

use std::time::Duration;

use pod_link::memory;
use podnet::carriers::{self, Options};
use podnet::exit::{self, code_of};
use podnet::link::{self, Error, Settings};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::timeout;

const LIMIT: Duration = Duration::from_secs(20);

#[tokio::test]
async fn a_link_crosses_the_memory_carrier_by_name() {
    let carrier = carriers::open("memory", &Options::default()).unwrap();
    let mut door = link::listen(&*carrier, "peer-a").await.unwrap();
    assert_eq!(door.address(), "peer-a");
    let (d, l) = tokio::join!(
        link::dial(&*carrier, "peer-a", Settings::default()),
        door.accept(Settings::default())
    );
    let (d, l) = (d.unwrap(), l.unwrap());
    let mut near = d.open().unwrap();
    near.write_all(b"in one process").await.unwrap();
    let mut far = timeout(LIMIT, l.accept()).await.unwrap().unwrap();
    let mut got = vec![0; 14];
    timeout(LIMIT, far.read_exact(&mut got))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got, b"in one process");
}

#[tokio::test]
async fn a_name_with_no_listener_exits_69_and_a_held_name_exits_69() {
    let carrier = carriers::open("memory", &Options::default()).unwrap();
    let e = link::dial(&*carrier, "nobody", Settings::default())
        .await
        .err()
        .unwrap();
    assert_eq!(code_of(&e), exit::UNAVAILABLE, "{e:#}");
    let _door = link::listen(&*carrier, "held").await.unwrap();
    let e = link::listen(&*carrier, "held").await.err().unwrap();
    assert_eq!(code_of(&e), exit::UNAVAILABLE, "{e:#}");
    let e = link::listen(&*carrier, "").await.err().unwrap();
    assert_eq!(code_of(&e), exit::USAGE, "{e:#}");
}

#[tokio::test]
async fn the_size_of_the_ring_is_64_kib_to_64_mib() {
    for bad in [0, memory::MIN_RING - 1, memory::MAX_RING + 1] {
        let options = Options {
            ring: bad,
            ..Options::default()
        };
        let e = carriers::open("memory", &options).err().unwrap();
        assert_eq!(code_of(&e), exit::USAGE, "ring {bad}: {e:#}");
    }
    for good in [memory::MIN_RING, memory::DEFAULT_RING, memory::MAX_RING] {
        let options = Options {
            ring: good,
            ..Options::default()
        };
        assert!(carriers::open("memory", &options).is_ok(), "ring {good}");
    }
}

#[tokio::test]
async fn a_full_ring_blocks_the_writer_and_truncates_nothing() {
    let ring = memory::MIN_RING;
    let (mut a, mut b) = memory::pair(ring);
    let data: Vec<u8> = (0..4 * ring + 5).map(|i| (i % 251) as u8).collect();
    let send = data.clone();
    let writer = tokio::spawn(async move {
        a.write_all(&send).await.unwrap();
        a
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!writer.is_finished(), "a full ring blocks the writer");
    let mut got = vec![0; data.len()];
    timeout(LIMIT, b.read_exact(&mut got))
        .await
        .unwrap()
        .unwrap();
    assert!(got == data, "no byte is lost");
    drop(writer.await.unwrap());
}

#[tokio::test]
async fn the_end_of_one_side_is_the_end_of_stream_of_the_other() {
    let (mut a, mut b) = memory::pair(memory::MIN_RING);
    a.write_all(b"last").await.unwrap();
    drop(a);
    let mut got = Vec::new();
    timeout(LIMIT, b.read_to_end(&mut got))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got, b"last");
}

#[tokio::test]
async fn a_link_over_a_memory_pair_ends_when_a_side_drops() {
    let (dialer, listener) = link::memory_pair(memory::MIN_RING, Settings::default()).unwrap();
    let mut s = dialer.open().unwrap();
    s.write_all(b"x").await.unwrap();
    let mut far = timeout(LIMIT, listener.accept()).await.unwrap().unwrap();
    let mut one = [0u8; 1];
    far.read_exact(&mut one).await.unwrap();
    drop(dialer);
    let e = timeout(LIMIT, listener.closed()).await.unwrap();
    assert!(
        matches!(
            e,
            Error::CarrierEnded | Error::Io { .. } | Error::Closed { .. }
        ),
        "{e:?}"
    );
}
