//! The split session of the link (T-302): one session over an up carrier
//! and a down carrier, through `podnet::link_split`, the glue of
//! `--up-carrier` and `--down-carrier`. The halves of the in-process tests
//! are paths of memory under a switch (open, slow, silent, closed). 1 MiB
//! goes down a fast half and 64 KiB up a slow half on one stream; the loss
//! of each half ends the session on each end with `up-carrier lost` or
//! `down-carrier lost`, and a read of a stream then exits 69. The CLI test
//! runs `link listen` and `link dial --check` over two unix sockets.

#[path = "link_peer/mod.rs"]
mod link_peer;

use std::collections::HashMap;
use std::future::Future as _;
use std::io::{self, BufRead, BufReader};
use std::pin::Pin;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use link_peer::{install, list_of};
use pod_link::split::{Half, Split};
use pod_link::{
    BoxIo, Carrier, CarrierError, CarrierErrorKind, Facts, Listener, Pending, Settings,
};
use podnet::exit::code_of;
use podnet::identity::Identity;
use podnet::link::{Door, Link};
use podnet::link_split;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, DuplexStream, ReadBuf};
use tokio::sync::mpsc;
use tokio::time::{timeout, Sleep};

const LIMIT: Duration = Duration::from_secs(30);

/// The state of each path of a carrier of the test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Open,
    /// Each write moves 1 KiB at most, and waits 2 ms before the next.
    Slow,
    /// Reads wait for ever and writes go nowhere: a cut network.
    Silent,
    /// Each read and write fails.
    Closed,
}

struct Switch {
    mode: Mutex<Mode>,
    wakers: Mutex<Vec<Waker>>,
}

impl Switch {
    fn new(mode: Mode) -> Arc<Switch> {
        Arc::new(Switch {
            mode: Mutex::new(mode),
            wakers: Mutex::new(Vec::new()),
        })
    }

    fn mode(&self) -> Mode {
        *self.mode.lock().unwrap()
    }

    fn set(&self, mode: Mode) {
        *self.mode.lock().unwrap() = mode;
        self.wakers.lock().unwrap().drain(..).for_each(Waker::wake);
    }

    fn wait(&self, cx: &Context<'_>) {
        self.wakers.lock().unwrap().push(cx.waker().clone());
    }
}

/// One end of a path, under its switch.
struct Cut {
    inner: DuplexStream,
    switch: Arc<Switch>,
    pause: Option<Pin<Box<Sleep>>>,
}

impl AsyncRead for Cut {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.switch.mode() {
            Mode::Open | Mode::Slow => {
                let polled = Pin::new(&mut self.inner).poll_read(cx, buf);
                if polled.is_pending() {
                    self.switch.wait(cx);
                }
                polled
            }
            Mode::Silent => {
                self.switch.wait(cx);
                Poll::Pending
            }
            Mode::Closed => Poll::Ready(Err(io::ErrorKind::BrokenPipe.into())),
        }
    }
}

impl AsyncWrite for Cut {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.switch.mode() {
            Mode::Open => Pin::new(&mut self.inner).poll_write(cx, data),
            Mode::Slow => {
                if let Some(pause) = self.pause.as_mut() {
                    if pause.as_mut().poll(cx).is_pending() {
                        return Poll::Pending;
                    }
                    self.pause = None;
                }
                let n = data.len().min(1024);
                let polled = Pin::new(&mut self.inner).poll_write(cx, &data[..n]);
                if polled.is_ready() {
                    self.pause = Some(Box::pin(tokio::time::sleep(Duration::from_millis(2))));
                }
                polled
            }
            Mode::Silent => Poll::Ready(Ok(data.len())),
            Mode::Closed => Poll::Ready(Err(io::ErrorKind::BrokenPipe.into())),
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

type Doors = Arc<Mutex<HashMap<String, mpsc::UnboundedSender<BoxIo>>>>;

/// A carrier of the test: a dial of `NAME` reaches the door `NAME`. It
/// proves no key, so the hello of podnet proves the keys.
struct Paths {
    name: &'static str,
    doors: Doors,
    switch: Arc<Switch>,
}

impl Paths {
    fn new(name: &'static str, mode: Mode) -> Arc<Paths> {
        Arc::new(Paths {
            name,
            doors: Doors::default(),
            switch: Switch::new(mode),
        })
    }
}

struct PathDoor(mpsc::UnboundedReceiver<BoxIo>, String);

impl Listener for PathDoor {
    fn address(&self) -> String {
        self.1.clone()
    }

    fn accept(&mut self) -> Pending<'_, BoxIo> {
        Box::pin(async move {
            self.0
                .recv()
                .await
                .ok_or_else(|| CarrierError::new("paths", CarrierErrorKind::NotFound, "no more"))
        })
    }
}

impl Carrier for Paths {
    fn name(&self) -> &'static str {
        self.name
    }

    fn facts(&self) -> Facts {
        Facts::STREAM
    }

    fn available(&self) -> Result<(), CarrierError> {
        Ok(())
    }

    fn dial<'a>(&'a self, address: &'a str) -> Pending<'a, BoxIo> {
        Box::pin(async move {
            let door = self.doors.lock().unwrap().get(address).cloned();
            let door = door.ok_or_else(|| {
                CarrierError::new("paths", CarrierErrorKind::NotFound, address.to_string())
            })?;
            let (a, b) = tokio::io::duplex(256 << 10);
            let cut = |inner| Cut {
                inner,
                switch: self.switch.clone(),
                pause: None,
            };
            door.send(Box::new(cut(b)))
                .map_err(|_| CarrierError::new("paths", CarrierErrorKind::NotFound, "gone"))?;
            Ok(Box::new(cut(a)) as BoxIo)
        })
    }

    fn listen<'a>(&'a self, address: &'a str) -> Pending<'a, Box<dyn Listener>> {
        Box::pin(async move {
            let (tx, rx) = mpsc::unbounded_channel();
            self.doors.lock().unwrap().insert(address.to_string(), tx);
            Ok(Box::new(PathDoor(rx, address.to_string())) as Box<dyn Listener>)
        })
    }
}

/// A split of `up` and `down` with short beats.
fn split(up: &Arc<Paths>, down: &Arc<Paths>) -> Split {
    let mut s = Split::new(up.clone(), down.clone());
    (s.beat, s.lost_after, s.join_timeout) = (
        Duration::from_millis(50),
        Duration::from_millis(600),
        Duration::from_secs(5),
    );
    s
}

/// A session of `a` (the dialer) to `b` over `up` and `down`, each end
/// proved with the hello.
async fn session(up: &Arc<Paths>, down: &Arc<Paths>, a: &Identity, b: &Identity) -> (Link, Link) {
    let s = split(up, down);
    let mut door: Door = link_split::listen(&s, "u|d").await.unwrap();
    assert_eq!(door.address(), "u|d");
    let accept = async {
        let (link, proved) = door.accept_peer(Settings::default()).await.unwrap();
        assert_eq!(proved, None, "the door gives no key: the hello proves it");
        let peer = podnet::hello::accept(&link, b, list_of(&[a]))
            .await
            .unwrap();
        assert_eq!(peer, a.public());
        link
    };
    let dial = link_split::dial(&s, a, list_of(&[b]), "u|d", Settings::default());
    let (dialed, far) = timeout(LIMIT, async { tokio::join!(dial, accept) })
        .await
        .expect("the session in time");
    let (near, peer) = dialed.unwrap();
    assert_eq!(peer, b.public());
    (near, far)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_fast_down_half_and_a_slow_up_half_move_data_both_ways() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (install(dir.path(), "a"), install(dir.path(), "b"));
    let (up, down) = (
        Paths::new("slow", Mode::Slow),
        Paths::new("fast", Mode::Open),
    );
    let (near, far) = session(&up, &down, &a, &b).await;
    let up_bytes: Vec<u8> = (0..64 << 10).map(|i| (i % 251) as u8).collect();
    let down_bytes: Vec<u8> = (0..1 << 20).map(|i| (i % 241) as u8).collect();
    let (sent_up, sent_down) = (up_bytes.clone(), down_bytes.clone());
    let mut stream = near.open().unwrap();
    let dialer = async move {
        let (mut reader, mut writer) = tokio::io::split(&mut stream);
        let send = async {
            writer.write_all(&sent_up).await.unwrap();
            writer.shutdown().await.unwrap();
        };
        let mut got = Vec::new();
        let (_, read) = tokio::join!(send, reader.read_to_end(&mut got));
        read.unwrap();
        got
    };
    let listener = async {
        let mut stream = far.accept().await.unwrap();
        let (mut reader, mut writer) = tokio::io::split(&mut stream);
        let send = async {
            writer.write_all(&sent_down).await.unwrap();
            writer.shutdown().await.unwrap();
        };
        let mut got = Vec::new();
        let (_, read) = tokio::join!(send, reader.read_to_end(&mut got));
        read.unwrap();
        got
    };
    let (down_got, up_got) = timeout(LIMIT, async { tokio::join!(dialer, listener) })
        .await
        .expect("the bytes in time");
    assert!(up_got == up_bytes, "64 KiB went up whole");
    assert!(down_got == down_bytes, "1 MiB came down whole");
    near.close().await;
}

/// The loss of `half` (its carrier goes to `mode`) ends the session on
/// each end with the report of that half; a read of a stream exits 69.
async fn a_lost_half_ends_the_session(half: Half, mode: Mode) {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (install(dir.path(), "a"), install(dir.path(), "b"));
    let (up, down) = (Paths::new("up", Mode::Open), Paths::new("down", Mode::Open));
    let (near, far) = session(&up, &down, &a, &b).await;
    let mut stream = near.open().unwrap();
    stream.write_all(b"one").await.unwrap();
    let mut accepted = far.accept().await.unwrap();
    let mut three = [0u8; 3];
    accepted.read_exact(&mut three).await.unwrap();
    match half {
        Half::Up => up.switch.set(mode),
        Half::Down => down.switch.set(mode),
    }
    let want = format!("{}-carrier lost", half.name());
    for (end, link) in [("dialer", &near), ("listener", &far)] {
        let why = timeout(LIMIT, link.closed())
            .await
            .expect("the end in time");
        assert!(why.to_string().contains(&want), "{end}: {why}");
        let lost = link_split::lost(link).expect("a lost half");
        assert!(lost.contains(&want), "{end}: {lost}");
    }
    let mut buf = [0u8; 16];
    let e = timeout(LIMIT, stream.read(&mut buf))
        .await
        .expect("the read ends")
        .expect_err("the read fails");
    assert!(link_split::is_lost(&e), "{e}");
    let e = link_split::ended(e);
    assert_eq!(code_of(&e), 69, "{e:#}");
    assert!(format!("{e:#}").contains(&want), "{e:#}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_closed_up_half_ends_the_session_with_up_carrier_lost() {
    a_lost_half_ends_the_session(Half::Up, Mode::Closed).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_closed_down_half_ends_the_session_with_down_carrier_lost() {
    a_lost_half_ends_the_session(Half::Down, Mode::Closed).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_silent_up_half_ends_the_session_after_the_wait() {
    a_lost_half_ends_the_session(Half::Up, Mode::Silent).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_silent_down_half_ends_the_session_after_the_wait() {
    a_lost_half_ends_the_session(Half::Down, Mode::Silent).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_stranger_is_refused_with_77() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b, c) = (
        install(dir.path(), "a"),
        install(dir.path(), "b"),
        install(dir.path(), "c"),
    );
    let (up, down) = (Paths::new("up", Mode::Open), Paths::new("down", Mode::Open));
    let s = split(&up, &down);
    let mut door = link_split::listen(&s, "u|d").await.unwrap();
    let accept = async {
        let (link, _) = door.accept_peer(Settings::default()).await.unwrap();
        let refused = podnet::hello::accept(&link, &b, list_of(&[&a])).await;
        link.close().await;
        refused
    };
    let dial = link_split::dial(&s, &c, list_of(&[&b]), "u|d", Settings::default());
    let (dialed, accepted) = timeout(LIMIT, async { tokio::join!(dial, accept) })
        .await
        .unwrap();
    let Err(e) = accepted else {
        panic!("the listener refuses a stranger")
    };
    assert_eq!(code_of(&e), 77, "{e:#}");
    let Err(e) = dialed else {
        panic!("the dialer is refused")
    };
    assert_eq!(code_of(&e), 77, "{e:#}");
}

/// The two unix sockets of a listener in `dir`.
fn sockets(dir: &std::path::Path) -> String {
    format!(
        "{}|{}",
        dir.join("up.sock").display(),
        dir.join("down.sock").display()
    )
}

#[test]
fn link_listen_and_dial_check_meet_over_two_unix_sockets() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b, c) = (
        install(dir.path(), "a"),
        install(dir.path(), "b"),
        install(dir.path(), "c"),
    );
    let allow_b = link_peer::write_allow(dir.path(), "allow-b", &[&a]);
    let allow_a = link_peer::write_allow(dir.path(), "allow-a", &[&b]);
    let address = sockets(dir.path());
    let halves = ["--up-carrier", "unix", "--down-carrier", "unix"];
    let mut args = vec![
        "link",
        "listen",
        &address,
        "--allow",
        allow_b.to_str().unwrap(),
        "--json",
    ];
    args.extend(halves);
    let mut listener = link_peer::podnet(dir.path(), "b", &args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(listener.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let doc: serde_json::Value = serde_json::from_str(&line).expect(&line);
    assert_eq!(doc["carrier"], "split");
    assert_eq!(doc["listen"], address.as_str());
    let run = |state: &str| {
        let mut args = vec![
            "link",
            "dial",
            &address,
            "--allow",
            allow_a.to_str().unwrap(),
            "--check",
        ];
        args.extend(halves);
        link_peer::podnet(dir.path(), state, &args)
            .output()
            .unwrap()
    };
    let ok = run("a");
    let stranger = run("c");
    let _ = listener.kill();
    let _ = listener.wait();
    assert_eq!(ok.status.code(), Some(0), "{ok:?}");
    assert!(
        String::from_utf8_lossy(&ok.stdout).contains(&b.fingerprint()),
        "{ok:?}"
    );
    assert_eq!(stranger.status.code(), Some(77), "{stranger:?}");
    let _ = c;
}

#[test]
fn the_halves_take_both_flags_and_an_address_of_two_parts() {
    let dir = tempfile::tempdir().unwrap();
    let allow = link_peer::write_allow(dir.path(), "allow", &[]);
    let allow = allow.to_str().unwrap();
    let run = |args: &[&str]| link_peer::podnet(dir.path(), "s", args).output().unwrap();
    // An address of one part exits 64.
    let out = run(&[
        "link",
        "listen",
        "/tmp/one",
        "--up-carrier",
        "unix",
        "--down-carrier",
        "unix",
        "--allow",
        allow,
    ]);
    assert_eq!(out.status.code(), Some(64), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("UP|DOWN"),
        "{out:?}"
    );
    // An unknown half exits 64.
    let out = run(&[
        "link",
        "listen",
        "/tmp/a|/tmp/b",
        "--up-carrier",
        "pigeon",
        "--down-carrier",
        "unix",
        "--allow",
        allow,
    ]);
    assert_eq!(out.status.code(), Some(64), "{out:?}");
    // One flag with no other exits 64.
    let out = run(&[
        "link",
        "listen",
        "/tmp/a|/tmp/b",
        "--up-carrier",
        "unix",
        "--allow",
        allow,
    ]);
    assert_eq!(out.status.code(), Some(64), "{out:?}");
}
