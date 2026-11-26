//! The direction split of the TCP flows of a router group (T-611). A far
//! `podnet link listen` joins a split session on the loopback and serves
//! the service `socks5`. The group has two stand-in members that dial TCP
//! on this host and count the bytes of each direction. A TCP flow of the
//! group sends its request bytes by the member `up` only and gets the
//! response bytes by the member `down` only. A lost half ends the session:
//! the next flow takes the pick of the group and counts as a fallback, and
//! the router dials a new session after the back-off. A bad value of
//! `splits:` exits 78 and names `splits.NAME`.
#![cfg(feature = "tcp")]

#[path = "link_peer/mod.rs"]
mod link_peer;

use std::io::{BufRead, BufReader};
use std::pin::Pin;
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use link_peer::install;
use podnet::network::{
    Balance, BoxFuture, Class, Connected, Dial, Failure, Member, Stage, Target, Way,
};
use podnet::router::split_tcp::{Spec, TcpSplit};
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::time::{timeout, Instant};

const LIMIT: Duration = Duration::from_secs(30);
/// The bytes of a request, and of a response.
const REQUEST: usize = 192 << 10;
const RESPONSE: usize = 640 << 10;
/// The most bytes that the other direction of a half carries for a flow:
/// the TLS handshake, the beats and the credits.
const OTHER: u64 = 48 << 10;

/// A stand-in member: it dials TCP on this host and counts the bytes that
/// it sends and receives. `cut` shuts each of its connections, and
/// `refuse` refuses each new dial of one target.
#[derive(Default)]
struct Counter {
    name: &'static str,
    sent: Arc<AtomicU64>,
    received: Arc<AtomicU64>,
    sockets: Mutex<Vec<std::net::TcpStream>>,
    refused: Mutex<Option<String>>,
}

impl Counter {
    fn new(name: &'static str) -> Arc<Counter> {
        Arc::new(Counter {
            name,
            ..Counter::default()
        })
    }

    fn bytes(&self) -> (u64, u64) {
        (
            self.sent.load(Ordering::Relaxed),
            self.received.load(Ordering::Relaxed),
        )
    }

    fn refuse(&self, target: Option<&str>) {
        *self.refused.lock().unwrap() = target.map(str::to_string);
    }

    fn cut(&self) {
        for s in self.sockets.lock().unwrap().drain(..) {
            let _ = s.shutdown(std::net::Shutdown::Both);
        }
    }
}

struct Counted {
    io: tokio::net::TcpStream,
    sent: Arc<AtomicU64>,
    received: Arc<AtomicU64>,
}

impl AsyncRead for Counted {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let polled = Pin::new(&mut self.io).poll_read(cx, buf);
        let n = (buf.filled().len() - before) as u64;
        self.received.fetch_add(n, Ordering::Relaxed);
        polled
    }
}

impl AsyncWrite for Counted {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let polled = Pin::new(&mut self.io).poll_write(cx, data);
        if let Poll::Ready(Ok(n)) = polled {
            self.sent.fetch_add(n as u64, Ordering::Relaxed);
        }
        polled
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.io).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.io).poll_shutdown(cx)
    }
}

impl Dial for Counter {
    fn label(&self) -> String {
        format!("counter {}", self.name)
    }

    fn dial<'a>(
        &'a self,
        target: &'a Target,
        _deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(async move {
            let fail =
                |e: std::io::Error| Failure::new(Stage::Connect, Class::Refused, e.to_string());
            if self.refused.lock().unwrap().as_deref() == Some(&target.to_string()) {
                return Err(fail(std::io::ErrorKind::ConnectionRefused.into()));
            }
            let std = std::net::TcpStream::connect(target.to_string()).map_err(fail)?;
            std.set_nonblocking(true).map_err(fail)?;
            self.sockets
                .lock()
                .unwrap()
                .push(std.try_clone().map_err(fail)?);
            let io = tokio::net::TcpStream::from_std(std).map_err(fail)?;
            Ok(Connected::new(Box::new(Counted {
                io,
                sent: self.sent.clone(),
                received: self.received.clone(),
            })))
        })
    }
}

/// A free TCP port of the loopback.
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// A target that reads [`REQUEST`] bytes and answers [`RESPONSE`] bytes.
async fn origin() -> Target {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut s, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut request = vec![0u8; REQUEST];
                s.read_exact(&mut request).await?;
                s.write_all(&vec![7u8; RESPONSE]).await?;
                s.shutdown().await
            });
        }
    });
    Target::Addr(addr)
}

struct Far(Child);

impl Drop for Far {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// The listen address and the dial address (with no key) of a half of the
/// carrier `name` at `port`.
fn forms(name: &str, port: u16) -> (String, String) {
    let at = format!("127.0.0.1:{port}");
    match name {
        "ws" => (at.clone(), format!("wss://{at}/link")),
        "h2" => (at.clone(), format!("https://{at}/link")),
        _ => (at.clone(), at),
    }
}

/// One flow of `split`: the request goes out and the whole response comes
/// back.
async fn flow(split: &TcpSplit, to: &Target) {
    let mut c = timeout(LIMIT, split.dial(to, Instant::now() + LIMIT))
        .await
        .expect("a dial in time")
        .expect("a dial");
    let run = async {
        c.io.write_all(&vec![1u8; REQUEST]).await.unwrap();
        let mut got = Vec::new();
        c.io.read_to_end(&mut got).await.unwrap();
        got
    };
    let got = timeout(LIMIT, run).await.expect("the flow in time");
    assert_eq!(got.len(), RESPONSE, "the whole response");
}

/// Wait until the status of `split` gives `want` for the key `key` of the
/// session.
async fn until(split: &TcpSplit, key: &str, want: Value) -> Value {
    let end = Instant::now() + LIMIT;
    loop {
        let status = split.status();
        if status["session"][key] == want {
            return status;
        }
        assert!(Instant::now() < end, "no {key} {want} in time: {status}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// T-611, the Prove: for each pair of carriers, the bytes up go by `up`
/// only and the bytes down by `down` only; a lost half falls back to the
/// pick and counts; a new session comes after the back-off.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn router_split_tcp_puts_each_direction_on_its_member_and_falls_back() {
    pod_testkit::need_loopback!();
    let mut cases = vec![("tcp", "tcp")];
    if cfg!(feature = "ws") && cfg!(feature = "h2") {
        cases.push(("ws", "h2"));
    }
    for (up_carrier, down_carrier) in cases {
        one_case(up_carrier, down_carrier).await;
    }
}

async fn one_case(up_carrier: &str, down_carrier: &str) {
    let case = format!("{up_carrier}|{down_carrier}");
    let dir = tempfile::tempdir().unwrap();
    let (near, far) = (install(dir.path(), "near"), install(dir.path(), "far"));
    let allow = link_peer::write_allow(dir.path(), "allow", &[&near]);
    let (up_listen, up_dial) = forms(up_carrier, free_port());
    let down_port = free_port();
    let (down_listen, down_dial) = forms(down_carrier, down_port);
    let down_target = format!("127.0.0.1:{down_port}");
    let listen = format!("{up_listen}|{down_listen}");
    let args = [
        "link",
        "listen",
        &listen,
        "--up-carrier",
        up_carrier,
        "--down-carrier",
        down_carrier,
        "--allow",
        allow.to_str().unwrap(),
        "--service",
        "socks5",
        "--json",
    ];
    let mut child = link_peer::podnet(dir.path(), "far", &args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let _far_process = Far(child);
    assert!(line.contains("split"), "{case}: {line}");

    let (a, b) = (Counter::new("a"), Counter::new("b"));
    let group = Balance::new(
        "g",
        vec![
            Member::new("a", Way::destination(a.clone()), 1),
            Member::new("b", Way::destination(b.clone()), 1),
        ],
    )
    .unwrap()
    .with_split("a", "b")
    .unwrap();
    let spec = Spec {
        far: far.fingerprint(),
        up: up_dial,
        down: down_dial,
        up_carrier: up_carrier.into(),
        down_carrier: down_carrier.into(),
    };
    let split = Arc::new(
        TcpSplit::new(Arc::new(group), spec)
            .unwrap()
            .with_backoff(Duration::from_millis(200), Duration::from_millis(400)),
    );
    split.check().unwrap();
    split.start(Arc::new(near));
    until(&split, "state", "up".into()).await;
    let to = origin().await;

    // The bytes of each direction go by their member.
    let (a0, b0) = (a.bytes(), b.bytes());
    flow(&split, &to).await;
    let ((a_sent, a_got), (b_sent, b_got)) = (a.bytes(), b.bytes());
    let (a_sent, a_got) = (a_sent - a0.0, a_got - a0.1);
    let (b_sent, b_got) = (b_sent - b0.0, b_got - b0.1);
    assert!(
        a_sent >= REQUEST as u64,
        "{case}: up sends the request: {a_sent}"
    );
    assert!(a_got < OTHER, "{case}: up gets no response: {a_got}");
    assert!(
        b_got >= RESPONSE as u64,
        "{case}: down gets the response: {b_got}"
    );
    assert!(b_sent < OTHER, "{case}: down sends no request: {b_sent}");
    let status = split.status();
    assert_eq!(status["mode"], "tcp", "{case}: {status}");
    assert_eq!(status["flows"], 1, "{case}: {status}");
    assert_eq!(status["tcp_fallbacks"], 0, "{case}: {status}");
    assert_eq!(status["session"]["sessions"], 1, "{case}: {status}");

    // A lost down half ends the session. The far end of the down half
    // refuses the member until the end of this step, so the session stays
    // down; the next flow takes the pick and counts as a fallback.
    b.refuse(Some(&down_target));
    b.cut();
    let status = until(&split, "lost", 1.into()).await;
    assert!(
        status["session"]["error"]
            .as_str()
            .is_some_and(|e| e.contains("carrier lost")),
        "{case}: {status}"
    );
    until(&split, "state", "down".into()).await;
    let (a1, b1) = (a.bytes(), b.bytes());
    flow(&split, &to).await;
    let status = split.status();
    assert_eq!(status["tcp_fallbacks"], 1, "{case}: {status}");
    assert_eq!(status["fallbacks"], 1, "{case}: {status}");
    assert_eq!(status["flows"], 1, "{case}: {status}");
    let one = |now: (u64, u64), was: (u64, u64)| now.0 - was.0 >= REQUEST as u64;
    assert!(
        one(a.bytes(), a1) ^ one(b.bytes(), b1),
        "{case}: one member carries the whole flow"
    );

    // The router dials a new session after the back-off.
    b.refuse(None);
    until(&split, "sessions", 2.into()).await;
    until(&split, "state", "up".into()).await;
    flow(&split, &to).await;
    let status = split.status();
    assert_eq!(status["flows"], 2, "{case}: {status}");
    assert_eq!(status["streams"], 0, "{case}: each stream ended: {status}");
}

/// T-611: one rule for each bad value of `splits:`: the read of the router
/// file exits 78 and names `splits.NAME`. A good value passes `--check`.
#[test]
fn router_split_tcp_bad_values_exit_78_with_the_key() {
    let dir = tempfile::tempdir().unwrap();
    let key = install(dir.path(), "far").fingerprint();
    let file = dir.path().join("route.yaml");
    let run = |value: &str| {
        let text = format!(
            "networks:\n  a: direct\n  b: direct\ngroups:\n  g: a, b\nsplits:\n  g: {value}\ndoors:\n  - listen: unix:{}\n    via: g\n",
            dir.path().join("door.sock").display()
        );
        std::fs::write(&file, text).unwrap();
        link_peer::podnet(
            dir.path(),
            "near",
            &["route", "--config", file.to_str().unwrap(), "--check"],
        )
        .output()
        .unwrap()
    };
    let at = format!("tcp={key}@127.0.0.1:7001|127.0.0.1:7002");
    for value in [
        at.clone(),
        format!("up=a down=b tcp={key}@127.0.0.1:7001"),
        "up=a down=b tcp=nokey@x|y".to_string(),
        format!("up=a down=b {at} carriers=unix|tcp"),
        "up=a down=b carriers=tcp|tcp".to_string(),
        "up=a down=b tcp".to_string(),
    ] {
        let out = run(&value);
        let err = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(78), "{value}: {err}");
        assert!(err.contains("splits.g"), "{value}: {err}");
    }
    let out = run(&format!("up=a down=b {at} carriers=tcp|tcp"));
    assert_eq!(out.status.code(), Some(0), "{out:?}");
}
