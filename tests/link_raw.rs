//! The service `raw` of a peer (T-293, GitHub #471 row E5-3): raw bytes
//! with no framing, half-close in both directions, and a large stream.
//! The link crosses a unix socket and the origin is a unix socket, so no
//! test needs a loopback.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use serde_json::Value;

/// One install: a state directory and a run directory of its own.
struct Host {
    dir: tempfile::TempDir,
}

impl Host {
    fn new() -> Host {
        Host {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_podnet"));
        cmd.args(args)
            .env("PODNET_STATE_DIR", self.path("state"))
            .env("PODNET_RUN_DIR", self.path("run"))
            .env_remove("PODNET_LINK_CONTROL")
            .env_remove("RUST_LOG");
        cmd
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command(args).output().unwrap()
    }

    fn fingerprint(&self) -> String {
        let out = self.run(&["link", "status", "--create", "--json"]);
        let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
        doc["identity"]["fingerprint"].as_str().unwrap().to_string()
    }
}

struct Proc(Child);

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start(cmd: &mut Command) -> (Proc, String) {
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.as_mut().unwrap())
        .read_line(&mut line)
        .unwrap();
    if line.is_empty() {
        let mut err = String::new();
        let _ = child.stderr.take().unwrap().read_to_string(&mut err);
        let _ = child.kill();
        panic!("no first line: {err}");
    }
    (Proc(child), line)
}

/// A listener, a connector and the origin of the service `raw`. The origin
/// calls `serve` once for each connection.
struct Pair {
    server: Host,
    client: Host,
    server_fp: String,
    control: PathBuf,
    _listener: Proc,
    _connector: Proc,
}

fn pair(serve: impl Fn(UnixStream) + Send + Sync + 'static) -> Pair {
    let (server, client) = (Host::new(), Host::new());
    let (sfp, cfp) = (server.fingerprint(), client.fingerprint());
    let origin = server.path("origin.sock");
    let listener = UnixListener::bind(&origin).unwrap();
    let serve = std::sync::Arc::new(serve);
    std::thread::spawn(move || {
        for conn in listener.incoming().flatten() {
            let serve = serve.clone();
            std::thread::spawn(move || serve(conn));
        }
    });
    let allow = server.path("allow");
    std::fs::write(&allow, &cfp).unwrap();
    let sock = server.path("l.sock");
    let to = format!("unix:{}", origin.display());
    let (l, _) = start(&mut server.command(&[
        "link",
        "listen",
        sock.to_str().unwrap(),
        "--allow",
        allow.to_str().unwrap(),
        "--to",
        &to,
        "--json",
    ]));
    let control = client.path("ctl.sock");
    let (c, _) = start(&mut client.command(&[
        "link",
        "connect",
        sock.to_str().unwrap(),
        "--peer",
        &sfp,
        "--control",
        control.to_str().unwrap(),
        "--json",
    ]));
    Pair {
        server,
        client,
        server_fp: sfp,
        control,
        _listener: l,
        _connector: c,
    }
}

impl Pair {
    /// `link dial` with the default service (`raw`).
    fn dial(&self) -> Child {
        let sock = self.server.path("l.sock");
        self.client
            .command(&[
                "link",
                "dial",
                sock.to_str().unwrap(),
                "--peer",
                &self.server_fp,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    }

    /// A unix forward socket to the service `raw` of the peer.
    fn forward(&self) -> (Proc, PathBuf) {
        let socket = self.client.path("f.sock");
        let rule = format!("unix:{}=peer:{}/raw", socket.display(), self.server_fp);
        let mut cmd = self
            .client
            .command(&["forward", "-L", &rule, "--run-for", "60"]);
        cmd.env("PODNET_LINK_CONTROL", &self.control);
        let (proc, _) = start(&mut cmd);
        (proc, socket)
    }
}

fn pattern(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
        .collect()
}

#[test]
fn a_close_of_the_opener_reaches_the_origin_and_the_answer_still_comes() {
    // The origin answers only after it saw the end of its input.
    let pair = pair(|mut c| {
        let mut all = Vec::new();
        c.read_to_end(&mut all).unwrap();
        let _ = c.write_all(&[b"seen:", all.as_slice()].concat());
    });
    let mut dial = pair.dial();
    dial.stdin.take().unwrap().write_all(b"request").unwrap();
    let out = dial.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(out.stdout, b"seen:request");
}

#[test]
fn a_close_of_the_origin_reaches_the_opener_and_the_opener_can_still_send() {
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    let tx = std::sync::Mutex::new(tx);
    let pair = pair(move |mut c| {
        c.write_all(b"first").unwrap();
        c.shutdown(std::net::Shutdown::Write).unwrap();
        // The origin reads on after its own close.
        let mut all = Vec::new();
        c.read_to_end(&mut all).unwrap();
        tx.lock().unwrap().send(all).unwrap();
    });
    let (_forward, socket) = pair.forward();
    let mut s = UnixStream::connect(&socket).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
    // The end of the origin output arrives as the end of the file.
    let mut got = Vec::new();
    s.read_to_end(&mut got).unwrap();
    assert_eq!(got, b"first");
    // The connection can still send, and the origin sees all of it.
    s.write_all(b"late").unwrap();
    s.shutdown(std::net::Shutdown::Write).unwrap();
    let seen = rx.recv_timeout(Duration::from_secs(20)).unwrap();
    assert_eq!(seen, b"late");
}

#[test]
fn four_mebibytes_each_way_arrive_unchanged() {
    let pair = pair(|mut c| {
        let mut reader = c.try_clone().unwrap();
        let mut sent = 0usize;
        let mut buf = vec![0u8; 64 * 1024];
        // Echo each chunk, then send the end.
        loop {
            let n = reader.read(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            c.write_all(&buf[..n]).unwrap();
            sent += n;
        }
        assert_eq!(sent, 4 << 20);
    });
    let data = pattern(4 << 20, 7);
    let mut dial = pair.dial();
    let mut stdin = dial.stdin.take().unwrap();
    let mut stdout = dial.stdout.take().unwrap();
    let copy = data.clone();
    let writer = std::thread::spawn(move || stdin.write_all(&copy).unwrap());
    let mut got = Vec::new();
    stdout.read_to_end(&mut got).unwrap();
    writer.join().unwrap();
    assert_eq!(got.len(), data.len());
    assert!(got == data, "the bytes changed on the way");
    assert_eq!(dial.wait().unwrap().code(), Some(0));
}

#[test]
fn the_service_raw_has_no_framing_a_single_byte_stays_a_single_byte() {
    let pair = pair(|mut c| {
        let mut b = [0u8; 1];
        c.read_exact(&mut b).unwrap();
        c.write_all(&b).unwrap();
    });
    let mut dial = pair.dial();
    dial.stdin.as_mut().unwrap().write_all(b"x").unwrap();
    let mut got = [0u8; 1];
    dial.stdout.as_mut().unwrap().read_exact(&mut got).unwrap();
    assert_eq!(&got, b"x");
    drop(dial.stdin.take());
    assert_eq!(dial.wait().unwrap().code(), Some(0));
}
