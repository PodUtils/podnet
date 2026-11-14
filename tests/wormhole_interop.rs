//! T-106 and T-103: `podnet send` and `podnet receive` with the Python
//! magic-wormhole (`wormhole send`, `wormhole receive`) at a local mailbox,
//! in both directions: a text, a file and a directory.
//!
//! The mailbox is the server of magic-wormhole (`twist wormhole-mailbox`,
//! the package `magic-wormhole-mailbox-server`) when `twist` is on the
//! PATH, else the stand-in mailbox of `tests/transfer_harness/mod.rs`. A
//! host with no `wormhole` program on the PATH skips each test with a line
//! `skipping, ...`.
//!
//! A file and a directory go over the transit: each end has the stand-in
//! TCP transit relay of `tests/transfer_harness/mod.rs` (podnet
//! `--transit-relay`, Python `--transit-helper`), and the Python end also
//! listens for a direct connection. The two ends must derive one transit
//! key (`derive_key` of `pod-wormhole`), else no handshake of the transit
//! passes.

#[path = "transfer_harness/mod.rs"]
mod harness;

mod test_host;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const PROXIES: [&str; 7] = [
    "HTTPS_PROXY",
    "https_proxy",
    "HTTP_PROXY",
    "http_proxy",
    "ALL_PROXY",
    "all_proxy",
    "PODNET_PROXY",
];

/// The longest time of one step.
const WAIT: Duration = Duration::from_secs(60);

/// The path of `name` in a directory of the PATH.
fn on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
}

/// The `wormhole` program, or `None` after the skip line.
fn wormhole() -> Option<PathBuf> {
    match on_path("wormhole") {
        Some(path) => Some(path),
        None => {
            test_host::skip_line(
                "no `wormhole` program on the PATH (pip install magic-wormhole; T-106)",
            );
            None
        }
    }
}

/// A command with no proxy of the host: each end talks to the loopback.
fn command(program: &std::path::Path, args: &[&str]) -> Command {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .env_remove("PODNET_MAILBOX")
        .env("NO_PROXY", "*")
        .env("no_proxy", "*");
    for key in PROXIES {
        cmd.env_remove(key);
    }
    cmd
}

fn podnet(args: &[&str]) -> Command {
    command(std::path::Path::new(env!("CARGO_BIN_EXE_podnet")), args)
}

/// A local mailbox: the server of magic-wormhole, or the stand-in.
enum Mailbox {
    Server {
        url: String,
        child: Child,
        _dir: tempfile::TempDir,
    },
    StandIn {
        mailbox: harness::Mailbox,
        _rt: tokio::runtime::Runtime,
    },
}

impl Mailbox {
    fn start() -> Mailbox {
        match on_path("twist") {
            Some(twist) => Mailbox::server(&twist),
            None => {
                eprintln!("no `twist` on the PATH: the stand-in mailbox of podnet");
                let rt = tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .build()
                    .unwrap();
                let mailbox = rt.block_on(harness::Mailbox::start());
                Mailbox::StandIn { mailbox, _rt: rt }
            }
        }
    }

    /// `twist wormhole-mailbox` on a free port of the loopback.
    fn server(twist: &std::path::Path) -> Mailbox {
        let dir = tempfile::tempdir().unwrap();
        let port = {
            let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            probe.local_addr().unwrap().port()
        };
        let child = command(
            twist,
            &[
                "wormhole-mailbox",
                &format!("--port=tcp:{port}:interface=127.0.0.1"),
                &format!("--channel-db={}", dir.path().join("relay.sqlite").display()),
            ],
        )
        .current_dir(dir.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("twist starts");
        let end = Instant::now() + WAIT;
        while TcpStream::connect(("127.0.0.1", port)).is_err() {
            assert!(Instant::now() < end, "the mailbox server did not listen");
            std::thread::sleep(Duration::from_millis(100));
        }
        eprintln!("the mailbox server of magic-wormhole on port {port}");
        Mailbox::Server {
            url: format!("ws://127.0.0.1:{port}/v1"),
            child,
            _dir: dir,
        }
    }

    fn url(&self) -> &str {
        match self {
            Mailbox::Server { url, .. } => url,
            Mailbox::StandIn { mailbox, .. } => &mailbox.url,
        }
    }
}

impl Drop for Mailbox {
    fn drop(&mut self) {
        if let Mailbox::Server { child, .. } = self {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// The flags of a podnet end at `mailbox`: no transit (a text needs none).
fn at(mailbox: &str) -> Vec<&str> {
    vec![
        "--mailbox",
        mailbox,
        "--transit-relay",
        "none",
        "--no-listen",
        "--json",
    ]
}

/// A pipe of a child: as it came, or a reader that read a part of it.
type Pipe = Box<dyn Read + Send>;

/// The output of `child`, or a panic after [`WAIT`] (the child is killed).
fn wait(mut child: Child, what: &str) -> Output {
    let out: Pipe = Box::new(child.stdout.take().unwrap());
    let err: Pipe = Box::new(child.stderr.take().unwrap());
    wait_with(child, out, err, what)
}

/// [`wait`] with the pipes of the child that a caller read a part of.
fn wait_with(mut child: Child, mut out: Pipe, mut err: Pipe, what: &str) -> Output {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut o = Vec::new();
        let _ = out.read_to_end(&mut o);
        let _ = tx.send(o);
    });
    let errs = std::thread::spawn(move || {
        let mut e = Vec::new();
        let _ = err.read_to_end(&mut e);
        e
    });
    let end = Instant::now() + WAIT;
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() > end {
            let _ = child.kill();
            let _ = child.wait();
            panic!("{what}: no end in {} s", WAIT.as_secs());
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    Output {
        status,
        stdout: rx.recv_timeout(WAIT).unwrap_or_default(),
        stderr: errs.join().unwrap_or_default(),
    }
}

fn text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// A child with the readers of its stdout and stderr.
struct Started {
    child: Child,
    out: Pipe,
    err: Pipe,
}

impl Started {
    fn wait(self, what: &str) -> Output {
        wait_with(self.child, self.out, self.err, what)
    }
}

/// A podnet sender of `message`, and the code that it printed.
fn podnet_sender(mailbox: &str, message: &str) -> (Started, String) {
    let mut args = vec!["send", "--text", message];
    args.extend(at(mailbox));
    let mut child = podnet(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    out.read_line(&mut line).unwrap();
    let event: serde_json::Value =
        serde_json::from_str(&line).unwrap_or_else(|_| panic!("a JSON line: {line:?}"));
    let code = event["code"].as_str().expect("the code").to_string();
    let err: Pipe = Box::new(child.stderr.take().unwrap());
    let started = Started {
        child,
        out: Box::new(out),
        err,
    };
    (started, code)
}

/// A Python sender of `message` at `mailbox`, and the code that it printed
/// ("Wormhole code is: CODE" on stderr).
fn python_sender(wormhole: &std::path::Path, mailbox: &str, message: &str) -> (Started, String) {
    let mut child = command(
        wormhole,
        &["--relay-url", mailbox, "send", "--text", message],
    )
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()
    .expect("wormhole starts");
    let mut err = BufReader::new(child.stderr.take().unwrap());
    let end = Instant::now() + WAIT;
    let code = loop {
        let mut line = String::new();
        let n = err.read_line(&mut line).unwrap();
        assert!(n > 0, "wormhole send ended with no code");
        if let Some(code) = line.trim().strip_prefix("Wormhole code is:") {
            break code.trim().to_string();
        }
        assert!(Instant::now() < end, "no code from wormhole send");
    };
    let out: Pipe = Box::new(child.stdout.take().unwrap());
    let started = Started {
        child,
        out,
        err: Box::new(err),
    };
    (started, code)
}

/// A podnet receiver at `mailbox` that reads `code` on stdin.
fn podnet_receiver(mailbox: &str, code: &str) -> Child {
    let mut args = vec!["receive"];
    args.extend(at(mailbox));
    let mut child = podnet(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    writeln!(child.stdin.take().unwrap(), "{code}").unwrap();
    child
}

// T-106: podnet sends a text, and `wormhole receive` of Python takes it.
#[test]
fn wormhole_interop_podnet_sends_a_text_to_python() {
    let Some(wormhole) = wormhole() else { return };
    test_host::need_loopback!();
    let mailbox = Mailbox::start();
    let message = "a text from podnet to Python, T-106";
    let (sender, code) = podnet_sender(mailbox.url(), message);
    let got = wait(
        command(
            &wormhole,
            // The code in argv: a test host has no other user to see it.
            &[
                "--relay-url",
                mailbox.url(),
                "receive",
                "--only-text",
                "--hide-progress",
                &code,
            ],
        )
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap(),
        "wormhole receive",
    );
    assert_eq!(got.status.code(), Some(0), "{}", text(&got));
    assert_eq!(String::from_utf8_lossy(&got.stdout).trim_end(), message);
    let sent = sender.wait("podnet send");
    assert_eq!(sent.status.code(), Some(0), "{}", text(&sent));
    assert!(
        String::from_utf8_lossy(&sent.stdout).contains(r#""event":"sent""#),
        "{}",
        text(&sent)
    );
}

// T-106: `wormhole send --text` of Python sends, and podnet receives.
#[test]
fn wormhole_interop_python_sends_a_text_to_podnet() {
    let Some(wormhole) = wormhole() else { return };
    test_host::need_loopback!();
    let mailbox = Mailbox::start();
    let message = "a text from Python to podnet, T-106";
    let (sender, code) = python_sender(&wormhole, mailbox.url(), message);
    let got = wait(podnet_receiver(mailbox.url(), &code), "podnet receive");
    assert_eq!(got.status.code(), Some(0), "{}", text(&got));
    let event: serde_json::Value = serde_json::from_slice(&got.stdout)
        .unwrap_or_else(|_| panic!("one JSON line: {}", text(&got)));
    assert_eq!(event["event"], "received");
    assert_eq!(event["text"], message);
    let sent = sender.wait("wormhole send");
    assert_eq!(sent.status.code(), Some(0), "{}", text(&sent));
    assert!(text(&sent).contains("text message sent"), "{}", text(&sent));
}

// T-106: a wrong code. podnet receives with a code that is not the code of
// the Python sender: podnet exits 77, and `wormhole send` fails too.
#[test]
fn wormhole_interop_a_wrong_code_fails_at_both_ends() {
    let Some(wormhole) = wormhole() else { return };
    test_host::need_loopback!();
    let mailbox = Mailbox::start();
    let (sender, code) = python_sender(&wormhole, mailbox.url(), "not for a wrong code");
    let (head, last) = code.rsplit_once('-').unwrap();
    // Another word of the same list, so that the code is well formed.
    let list = if pod_wormhole::words::EVEN.contains(&last) {
        &pod_wormhole::words::EVEN
    } else {
        &pod_wormhole::words::ODD
    };
    let other = if last == list[0] { list[1] } else { list[0] };
    let got = wait(
        podnet_receiver(mailbox.url(), &format!("{head}-{other}")),
        "podnet receive",
    );
    assert_eq!(got.status.code(), Some(77), "{}", text(&got));
    assert!(
        !text(&got).contains("not for a wrong code"),
        "{}",
        text(&got)
    );
    let sent = sender.wait("wormhole send");
    assert_ne!(sent.status.code(), Some(0), "{}", text(&sent));
    assert!(
        text(&sent).to_lowercase().contains("wrong") || text(&sent).contains("WrongPassword"),
        "{}",
        text(&sent)
    );
}

/// A stand-in transit relay of magic-wormhole on the loopback.
struct TransitRelay {
    relay: harness::Relay,
    _rt: tokio::runtime::Runtime,
}

impl TransitRelay {
    fn start() -> TransitRelay {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let relay = rt.block_on(harness::Relay::start());
        TransitRelay { relay, _rt: rt }
    }

    fn url(&self) -> &str {
        &self.relay.url
    }
}

/// The bytes of a file of `len` bytes that differ from file to file.
fn data(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u32).wrapping_mul(2_654_435_761).to_le_bytes()[3] ^ seed)
        .collect()
}

/// A directory with files at two levels, under `root/NAME`.
fn tree(root: &std::path::Path, name: &str) -> PathBuf {
    let top = root.join(name);
    std::fs::create_dir_all(top.join("inner/deep")).unwrap();
    std::fs::write(top.join("a.txt"), b"the first file\n").unwrap();
    std::fs::write(top.join("inner/b.bin"), data(150_000, 3)).unwrap();
    std::fs::write(top.join("inner/deep/c.bin"), data(4_096, 9)).unwrap();
    std::fs::write(top.join("inner/empty"), b"").unwrap();
    top
}

/// Each file under `dir`: its path from `dir`, and its bytes.
fn files(dir: &std::path::Path) -> std::collections::BTreeMap<String, Vec<u8>> {
    let mut out = std::collections::BTreeMap::new();
    let mut todo = vec![dir.to_path_buf()];
    while let Some(at) = todo.pop() {
        for entry in std::fs::read_dir(&at).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                todo.push(path);
            } else {
                let name = path
                    .strip_prefix(dir)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                out.insert(name, std::fs::read(&path).unwrap());
            }
        }
    }
    out
}

/// A podnet sender of the file or the directory `path`, with the relay,
/// and the code that it printed.
fn podnet_path_sender(mailbox: &str, relay: &str, path: &std::path::Path) -> (Started, String) {
    let path = path.to_str().unwrap();
    let args = [
        "send",
        path,
        "--mailbox",
        mailbox,
        "--transit-relay",
        relay,
        "--no-listen",
        "--json",
    ];
    let mut child = podnet(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    out.read_line(&mut line).unwrap();
    let event: serde_json::Value =
        serde_json::from_str(&line).unwrap_or_else(|_| panic!("a JSON line: {line:?}"));
    let code = event["code"].as_str().expect("the code").to_string();
    let err: Pipe = Box::new(child.stderr.take().unwrap());
    let started = Started {
        child,
        out: Box::new(out),
        err,
    };
    (started, code)
}

/// `wormhole receive` of `code` into `out`, with the relay: it accepts the
/// file or the directory with no question.
fn python_receive(
    wormhole: &std::path::Path,
    mailbox: &str,
    relay: &str,
    code: &str,
    out: &std::path::Path,
) -> Output {
    let child = command(
        wormhole,
        // The code in argv: a test host has no other user to see it.
        &[
            "--relay-url",
            mailbox,
            "--transit-helper",
            relay,
            "receive",
            "--accept-file",
            "--hide-progress",
            "-o",
            out.to_str().unwrap(),
            code,
        ],
    )
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()
    .expect("wormhole starts");
    wait(child, "wormhole receive")
}

/// A Python sender of the file or the directory `path`, with the relay,
/// and the code that it printed.
fn python_path_sender(
    wormhole: &std::path::Path,
    mailbox: &str,
    relay: &str,
    path: &std::path::Path,
) -> (Started, String) {
    let mut child = command(
        wormhole,
        &[
            "--relay-url",
            mailbox,
            "--transit-helper",
            relay,
            "send",
            "--hide-progress",
            path.to_str().unwrap(),
        ],
    )
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()
    .expect("wormhole starts");
    let mut err = BufReader::new(child.stderr.take().unwrap());
    let end = Instant::now() + WAIT;
    let code = loop {
        let mut line = String::new();
        let n = err.read_line(&mut line).unwrap();
        assert!(n > 0, "wormhole send ended with no code");
        if let Some(code) = line.trim().strip_prefix("Wormhole code is:") {
            break code.trim().to_string();
        }
        assert!(Instant::now() < end, "no code from wormhole send");
    };
    let out: Pipe = Box::new(child.stdout.take().unwrap());
    let started = Started {
        child,
        out,
        err: Box::new(err),
    };
    (started, code)
}

/// `podnet receive --yes` of `code` into the directory `out`, with the
/// relay.
fn podnet_receive(mailbox: &str, relay: &str, code: &str, out: &std::path::Path) -> Output {
    let args = [
        "receive",
        "--yes",
        "--out",
        out.to_str().unwrap(),
        "--mailbox",
        mailbox,
        "--transit-relay",
        relay,
        "--no-listen",
        "--json",
    ];
    let mut child = podnet(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    writeln!(child.stdin.take().unwrap(), "{code}").unwrap();
    wait(child, "podnet receive")
}

// T-103: podnet sends a file, and `wormhole receive` of Python takes it.
#[test]
fn wormhole_interop_podnet_sends_a_file_to_python() {
    let Some(wormhole) = wormhole() else { return };
    test_host::need_loopback!();
    let (mailbox, relay) = (Mailbox::start(), TransitRelay::start());
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("from-podnet.bin");
    let bytes = data(300_000, 1);
    std::fs::write(&file, &bytes).unwrap();
    let (sender, code) = podnet_path_sender(mailbox.url(), relay.url(), &file);
    let got = dir.path().join("got.bin");
    let received = python_receive(&wormhole, mailbox.url(), relay.url(), &code, &got);
    assert_eq!(received.status.code(), Some(0), "{}", text(&received));
    assert!(std::fs::read(&got).unwrap() == bytes, "the bytes differ");
    let sent = sender.wait("podnet send");
    assert_eq!(sent.status.code(), Some(0), "{}", text(&sent));
    assert!(text(&sent).contains(r#""event":"sent""#), "{}", text(&sent));
}

// T-103: `wormhole send` of Python sends a file, and podnet receives it.
#[test]
fn wormhole_interop_python_sends_a_file_to_podnet() {
    let Some(wormhole) = wormhole() else { return };
    test_host::need_loopback!();
    let (mailbox, relay) = (Mailbox::start(), TransitRelay::start());
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("from-python.bin");
    let bytes = data(300_000, 2);
    std::fs::write(&file, &bytes).unwrap();
    let (sender, code) = python_path_sender(&wormhole, mailbox.url(), relay.url(), &file);
    let out = dir.path().join("out");
    std::fs::create_dir(&out).unwrap();
    let got = podnet_receive(mailbox.url(), relay.url(), &code, &out);
    assert_eq!(got.status.code(), Some(0), "{}", text(&got));
    assert!(
        std::fs::read(out.join("from-python.bin")).unwrap() == bytes,
        "the bytes differ"
    );
    let sent = sender.wait("wormhole send");
    assert_eq!(sent.status.code(), Some(0), "{}", text(&sent));
}

// T-103: podnet sends a directory (a zip), and Python unpacks it.
#[test]
fn wormhole_interop_podnet_sends_a_directory_to_python() {
    let Some(wormhole) = wormhole() else { return };
    test_host::need_loopback!();
    let (mailbox, relay) = (Mailbox::start(), TransitRelay::start());
    let dir = tempfile::tempdir().unwrap();
    let top = tree(dir.path(), "from-podnet");
    let (sender, code) = podnet_path_sender(mailbox.url(), relay.url(), &top);
    let got = dir.path().join("got");
    let received = python_receive(&wormhole, mailbox.url(), relay.url(), &code, &got);
    assert_eq!(received.status.code(), Some(0), "{}", text(&received));
    assert!(
        files(&got) == files(&top),
        "the trees differ: {:?}",
        files(&got).keys()
    );
    let sent = sender.wait("podnet send");
    assert_eq!(sent.status.code(), Some(0), "{}", text(&sent));
}

// T-103: Python sends a directory (a deflated zip), and podnet lands it.
#[test]
fn wormhole_interop_python_sends_a_directory_to_podnet() {
    let Some(wormhole) = wormhole() else { return };
    test_host::need_loopback!();
    let (mailbox, relay) = (Mailbox::start(), TransitRelay::start());
    let dir = tempfile::tempdir().unwrap();
    let top = tree(dir.path(), "from-python");
    let (sender, code) = python_path_sender(&wormhole, mailbox.url(), relay.url(), &top);
    let out = dir.path().join("out");
    std::fs::create_dir(&out).unwrap();
    let got = podnet_receive(mailbox.url(), relay.url(), &code, &out);
    assert_eq!(got.status.code(), Some(0), "{}", text(&got));
    let landed = files(&out.join("from-python"));
    assert!(
        landed == files(&top),
        "the trees differ: {:?}",
        landed.keys()
    );
    let sent = sender.wait("wormhole send");
    assert_eq!(sent.status.code(), Some(0), "{}", text(&sent));
}
