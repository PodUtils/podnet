//! The middle hop of a mesh sees sealed records only (T-298, T-287): a
//! stream from A reaches `echo` of C through B over unix links in one
//! process. Each node taps the bytes that it reads from its links.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use podnet::identity::{self, AllowList, Identity};
use podnet::link::{self, Link, Role, Settings as LinkSettings};
use podnet::link_announce::{self as announce, Guard, Mesh};
use podnet::link_forward::{self as forward, Arrived};
use podnet::link_record::{self as record, Options, Sealed};
use podnet::link_service as service;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};

async fn echo(mut s: Sealed) {
    let mut buf = vec![0u8; 8192];
    loop {
        let n = s.read(&mut buf).await.unwrap_or(0);
        if n == 0 {
            break;
        }
        s.write_all(&buf[..n]).await.unwrap();
    }
    let _ = s.shutdown().await;
}

async fn round(mut s: Sealed, data: Vec<u8>) -> (Vec<u8>, Sealed) {
    let (mut r, mut w) = tokio::io::split(&mut s);
    let send = async {
        w.write_all(&data).await.unwrap();
        w.shutdown().await.unwrap();
    };
    let mut back = Vec::new();
    let (_, got) = tokio::join!(send, r.read_to_end(&mut back));
    got.unwrap();
    (back, s)
}

/// Bytes that a node read from its links.
#[derive(Clone, Default)]
struct Log(Arc<Mutex<Vec<u8>>>);

struct Tap<S> {
    inner: S,
    log: Log,
}

impl<S: AsyncRead + Unpin> AsyncRead for Tap<S> {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let r = std::pin::Pin::new(&mut self.inner).poll_read(cx, buf);
        if let Poll::Ready(Ok(())) = &r {
            let new = buf.filled()[before..].to_vec();
            self.log.0.lock().unwrap().extend_from_slice(&new);
        }
        r
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Tap<S> {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        std::pin::Pin::new(&mut self.inner).poll_write(cx, buf)
    }
    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

struct Node {
    id: Arc<Identity>,
    mesh: Arc<Mesh>,
    guards: Vec<Guard>,
    seen: Log,
}

fn node(dir: &Path, name: &str) -> Node {
    let sub = dir.join(name);
    std::fs::create_dir(&sub).unwrap();
    let id = Arc::new(identity::create(&sub).unwrap());
    let mesh = Mesh::new(id.clone(), vec!["echo".into()], announce::now_ms());
    Node {
        id,
        mesh,
        guards: Vec::new(),
        seen: Log::default(),
    }
}

/// What a node does with a stream: the announcements, the forwarding, and
/// the service `echo`, which seals the stream when `sealed` is on.
async fn serve(
    mesh: Arc<Mesh>,
    id: Arc<Identity>,
    trusted: AllowList,
    link: Arc<Link>,
    peer: pod_identity::PublicKey,
    sealed: bool,
    opens: Arc<AtomicUsize>,
) {
    while let Ok(mut stream) = link.accept().await {
        let (mesh, link, id, trusted, opens) = (
            mesh.clone(),
            link.clone(),
            id.clone(),
            trusted.clone(),
            opens.clone(),
        );
        tokio::spawn(async move {
            let Ok(mut name) = service::accept(&mut stream).await else {
                return;
            };
            if name == announce::NAME {
                announce::receive_stream(mesh, link, peer, Some(1), &mut stream).await;
                return;
            }
            if name == forward::NAME {
                match forward::accept(&mesh, &mut stream).await {
                    Ok(Arrived::Local(local)) => name = local,
                    _ => return,
                }
            }
            if name != "echo" {
                let _ = service::reply(&mut stream, service::NO_SERVICE).await;
                return;
            }
            let _ = service::reply(&mut stream, service::OPEN).await;
            opens.fetch_add(1, Ordering::SeqCst);
            if sealed {
                if let Ok(s) = record::accept(stream, &id, &trusted, Options::default()).await {
                    echo(s).await;
                }
            } else {
                let (mut r, mut w) = tokio::io::split(stream);
                let _ = tokio::io::copy(&mut r, &mut w).await;
                let _ = w.shutdown().await;
            }
        });
    }
}

/// A link between two nodes. Each node taps what it reads from the link.
fn join(x: &mut Node, y: &mut Node, xt: &AllowList, yt: &AllowList, sealed: bool) {
    let (a, b) = pod_link::unix::pair().unwrap();
    let (a, b) = (
        Tap {
            inner: a,
            log: x.seen.clone(),
        },
        Tap {
            inner: b,
            log: y.seen.clone(),
        },
    );
    let lx = Arc::new(link::from_io(a, Role::Dialer, LinkSettings::default()).unwrap());
    let ly = Arc::new(link::from_io(b, Role::Listener, LinkSettings::default()).unwrap());
    let opens = Arc::new(AtomicUsize::new(0));
    tokio::spawn(serve(
        x.mesh.clone(),
        x.id.clone(),
        xt.clone(),
        lx.clone(),
        y.id.public(),
        sealed,
        opens.clone(),
    ));
    tokio::spawn(serve(
        y.mesh.clone(),
        y.id.clone(),
        yt.clone(),
        ly.clone(),
        x.id.public(),
        sealed,
        opens,
    ));
    x.guards
        .push(Guard::start(&x.mesh, &lx, y.id.public(), Some(1), 1000));
    y.guards
        .push(Guard::start(&y.mesh, &ly, x.id.public(), Some(1), 1000));
}

async fn route_ready(from: &Node, to: &Node) {
    let end = Instant::now() + Duration::from_secs(15);
    loop {
        let doc = from.mesh.routes(announce::now_ms());
        let found = doc["routes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["origin"] == to.id.public().display_fingerprint() && r["hops"] == 2);
        if found {
            return;
        }
        assert!(Instant::now() < end, "no route");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn has(log: &Log, needle: &[u8]) -> bool {
    log.0
        .lock()
        .unwrap()
        .windows(needle.len())
        .any(|w| w == needle)
}

async fn three_nodes(sealed: bool) -> (Node, Node, Node, Vec<u8>) {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, mut b, mut c) = (
        node(dir.path(), "a"),
        node(dir.path(), "b"),
        node(dir.path(), "c"),
    );
    let all = |ids: &[&Node]| {
        let text: Vec<String> = ids.iter().map(|n| n.id.fingerprint()).collect();
        AllowList::parse(&text.join("\n")).unwrap()
    };
    let (ta, tb, tc) = (all(&[&c, &b]), all(&[&a, &c]), all(&[&a, &b]));
    join(&mut a, &mut b, &ta, &tb, sealed);
    join(&mut b, &mut c, &tb, &tc, sealed);
    route_ready(&a, &c).await;
    route_ready(&c, &a).await;
    let marker: Vec<u8> = b"PLAINTEXT-MARKER-0123456789-ABCDEF-".repeat(8);
    std::mem::forget(dir);
    (a, b, c, marker)
}

#[tokio::test]
async fn a_middle_hop_sees_no_plaintext_of_a_sealed_stream() {
    let (a, b, c, marker) = three_nodes(true).await;
    let stream = forward::open(&a.mesh, &c.id.public(), "echo", forward::DEFAULT_HOPS)
        .await
        .unwrap();
    let sealed = record::dial(stream, &a.id, Some(&c.id.public()), Options::default())
        .await
        .unwrap();
    assert_eq!(sealed.peer(), c.id.public());
    let (back, _) = round(sealed, marker.repeat(50)).await;
    assert_eq!(back, marker.repeat(50));
    // B carried the bytes of the stream: it read the records, not the text.
    assert!(
        b.seen.0.lock().unwrap().len() > marker.len() * 50,
        "B carried the stream"
    );
    assert!(!has(&b.seen, &marker[..32]), "B read the plaintext");
}

#[tokio::test]
async fn without_the_layer_the_middle_hop_sees_the_plaintext() {
    // The control of the test above: the tap sees the marker when the
    // stream is not sealed.
    let (a, b, c, marker) = three_nodes(false).await;
    let stream = forward::open(&a.mesh, &c.id.public(), "echo", forward::DEFAULT_HOPS)
        .await
        .unwrap();
    let (mut r, mut w) = tokio::io::split(stream);
    let data = marker.repeat(50);
    let d2 = data.clone();
    tokio::spawn(async move {
        w.write_all(&d2).await.unwrap();
        w.shutdown().await.unwrap();
    });
    let mut back = Vec::new();
    r.read_to_end(&mut back).await.unwrap();
    assert_eq!(back, data);
    assert!(has(&b.seen, &marker[..32]), "the tap works");
}
