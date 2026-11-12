//! The tests of `--early-reply` in the door (T-237).

use std::sync::atomic::AtomicUsize;
use std::sync::Mutex;

use pod_core::{BoxFuture, Class, Connected, Failure, Stage};
use tokio::io::DuplexStream;
use tokio::sync::{mpsc, Notify};

use super::*;

/// A dialer that waits for `gate`, then gives the far end of a stream to
/// the test, or fails (`fail`), or refuses the name `blocked.example` like
/// a rule.
struct Gated {
    gate: Notify,
    fail: bool,
    far: mpsc::UnboundedSender<DuplexStream>,
    dials: Mutex<Vec<String>>,
}

impl Dial for Gated {
    fn label(&self) -> String {
        "gated".into()
    }

    fn dial<'a>(
        &'a self,
        target: &'a Target,
        _deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(async move {
            self.dials.lock().unwrap().push(target.to_string());
            if target.host() == "blocked.example" {
                return Err(Failure::new(
                    Stage::Proxy,
                    Class::Policy,
                    "refused by a rule",
                ));
            }
            self.gate.notified().await;
            if self.fail {
                return Err(Failure::new(Stage::Connect, Class::Refused, "refused"));
            }
            let (near, far) = tokio::io::duplex(1 << 21);
            let _ = self.far.send(far);
            Ok(Connected::new(Box::new(near)))
        })
    }
}

type Far = mpsc::UnboundedReceiver<DuplexStream>;

fn gated(early: bool, fail: bool) -> (Arc<Door>, Arc<Gated>, Far) {
    let (far, rx) = mpsc::unbounded_channel();
    let dial = Arc::new(Gated {
        gate: Notify::new(),
        fail,
        far,
        dials: Mutex::new(Vec::new()),
    });
    let mut door = Door::new(dial.clone());
    if early {
        let admit: Admit = Arc::new(|target: &Target| target.host() != "blocked.example");
        door = door.with_early_reply(admit);
    }
    (Arc::new(door), dial, rx)
}

/// A client that has sent the greeting and the SOCKS5 request for `host`.
async fn request(door: &Arc<Door>, host: &str, buffer: usize) -> DuplexStream {
    let (mut client, server) = tokio::io::duplex(buffer);
    door.clone().spawn(server);
    client.write_all(&[5, 1, 0]).await.unwrap();
    let mut method = [0u8; 2];
    client.read_exact(&mut method).await.unwrap();
    let mut req = vec![5, 1, 0, 3, host.len() as u8];
    req.extend_from_slice(host.as_bytes());
    req.extend_from_slice(&443u16.to_be_bytes());
    client.write_all(&req).await.unwrap();
    client
}

async fn reply_within(client: &mut DuplexStream, limit: Duration) -> Option<[u8; 10]> {
    let mut reply = [0u8; 10];
    tokio::time::timeout(limit, client.read_exact(&mut reply))
        .await
        .ok()
        .map(|read| {
            read.unwrap();
            reply
        })
}

const SHORT: Duration = Duration::from_millis(150);

/// With the option off, the reply waits for the dial.
#[tokio::test]
async fn early_reply_is_off_by_default() {
    let (door, dial, mut far) = gated(false, false);
    let mut client = request(&door, "slow.example", 4096).await;
    assert!(reply_within(&mut client, SHORT).await.is_none());
    dial.gate.notify_one();
    let reply = reply_within(&mut client, Duration::from_secs(5)).await;
    assert_eq!(reply.unwrap()[1], 0);
    assert!(far.recv().await.is_some());
}

/// With the option on, the reply comes before the dial, for SOCKS5 and for
/// HTTP CONNECT, and the held bytes follow the dial.
#[tokio::test]
async fn early_reply_answers_before_the_dial_and_forwards_the_held_bytes() {
    let (door, dial, mut far) = gated(true, false);
    let mut client = request(&door, "slow.example", 4096).await;
    let reply = reply_within(&mut client, Duration::from_secs(5)).await;
    assert_eq!(reply.unwrap()[1], 0, "the reply came with the gate shut");
    client.write_all(b"first flight").await.unwrap();
    tokio::time::sleep(SHORT).await;
    dial.gate.notify_one();
    let mut far = far.recv().await.unwrap();
    let mut got = [0u8; 12];
    far.read_exact(&mut got).await.unwrap();
    assert_eq!(&got, b"first flight");
    far.write_all(b"back").await.unwrap();
    let mut back = [0u8; 4];
    client.read_exact(&mut back).await.unwrap();
    assert_eq!(&back, b"back");

    let (mut client, server) = tokio::io::duplex(4096);
    door.clone().spawn(server);
    client
        .write_all(b"CONNECT slow.example:443 HTTP/1.1\r\n\r\n")
        .await
        .unwrap();
    let mut head = [0u8; 12];
    tokio::time::timeout(Duration::from_secs(5), client.read_exact(&mut head))
        .await
        .expect("the HTTP answer came with the gate shut")
        .unwrap();
    assert_eq!(&head, b"HTTP/1.1 200");
}

/// A target that the rules refuse gets the real answer, never an early one.
#[tokio::test]
async fn early_reply_keeps_the_refusal_of_a_rule() {
    let (door, dial, _far) = gated(true, false);
    let mut client = request(&door, "blocked.example", 4096).await;
    let reply = reply_within(&mut client, Duration::from_secs(5)).await;
    assert_eq!(reply.unwrap()[1], 0x02);

    let (mut client, server) = tokio::io::duplex(4096);
    door.clone().spawn(server);
    client
        .write_all(b"CONNECT blocked.example:443 HTTP/1.1\r\n\r\n")
        .await
        .unwrap();
    let mut text = String::new();
    client.read_to_string(&mut text).await.unwrap();
    assert!(text.starts_with("HTTP/1.1 403 Forbidden\r\n"), "{text}");
    assert_eq!(dial.dials.lock().unwrap().len(), 2);
}

/// A dial that fails after the reply closes the connection: no error code.
#[tokio::test]
async fn early_reply_closes_the_connection_when_the_dial_fails() {
    let (door, dial, _far) = gated(true, true);
    let mut client = request(&door, "down.example", 4096).await;
    let reply = reply_within(&mut client, Duration::from_secs(5)).await;
    assert_eq!(reply.unwrap()[1], 0);
    dial.gate.notify_one();
    let mut rest = Vec::new();
    client.read_to_end(&mut rest).await.unwrap();
    assert!(rest.is_empty(), "the connection only ends: {rest:?}");
}

/// A client that sends its end before the dial is done: the end goes to the
/// upstream after the held bytes.
#[tokio::test]
async fn early_reply_sends_a_half_close_after_the_held_bytes() {
    let (door, dial, mut far) = gated(true, false);
    let mut client = request(&door, "slow.example", 4096).await;
    reply_within(&mut client, Duration::from_secs(5))
        .await
        .unwrap();
    client.write_all(b"bye").await.unwrap();
    client.shutdown().await.unwrap();
    tokio::time::sleep(SHORT).await;
    dial.gate.notify_one();
    let mut far = far.recv().await.unwrap();
    let mut all = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), far.read_to_end(&mut all))
        .await
        .expect("the end of the client reached the upstream")
        .unwrap();
    assert_eq!(all, b"bye");
}

/// The door holds at most 64 KiB while the dial is not done, and the whole
/// 1 MiB of the client arrives after it.
#[tokio::test]
async fn early_reply_holds_at_most_64_kib() {
    const BUFFER: usize = 100_000;
    const TOTAL: usize = 1 << 20;
    const CHUNK: usize = 1024;
    let (door, dial, mut far) = gated(true, false);
    let mut client = request(&door, "slow.example", BUFFER).await;
    reply_within(&mut client, Duration::from_secs(5))
        .await
        .unwrap();
    let data: Vec<u8> = (0..TOTAL).map(|i| (i % 251) as u8).collect();
    let written = Arc::new(AtomicUsize::new(0));
    let (mut read_half, mut write_half) = tokio::io::split(client);
    let sent = written.clone();
    let body = data.clone();
    let writer = tokio::spawn(async move {
        for chunk in body.chunks(CHUNK) {
            write_half.write_all(chunk).await.unwrap();
            sent.fetch_add(chunk.len(), Ordering::SeqCst);
        }
        write_half.shutdown().await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    // The door holds EARLY_HOLD bytes, the stream holds BUFFER bytes, and
    // the writer is blocked on one more chunk.
    let seen = written.load(Ordering::SeqCst);
    assert!(
        seen <= EARLY_HOLD + BUFFER + CHUNK,
        "the client wrote {seen} bytes with the gate shut"
    );
    assert!(seen >= EARLY_HOLD, "the door held {seen} bytes");
    dial.gate.notify_one();
    let mut far = far.recv().await.unwrap();
    let mut all = Vec::new();
    tokio::time::timeout(Duration::from_secs(20), far.read_to_end(&mut all))
        .await
        .expect("the upstream got the whole input")
        .unwrap();
    assert_eq!(all.len(), TOTAL);
    assert!(all == data, "the bytes are the same and in order");
    writer.await.unwrap();
    far.shutdown().await.unwrap();
    let mut rest = Vec::new();
    read_half.read_to_end(&mut rest).await.unwrap();
}
