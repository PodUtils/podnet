//! `podnet forward -L LISTEN=peer:FINGERPRINT/SERVICE` (T-266, GitHub #466
//! row E0-5 and #471 row E5-4): one stream of a link for each accepted
//! connection, to a service of a peer. The listener listens on a unix
//! socket, the link crosses a unix socket, and the far origin is a unix
//! socket: only the test with a TCP forward needs a loopback.

mod test_host;

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

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

    fn allow(&self, lines: &[&str]) -> PathBuf {
        let path = self.path("allow");
        std::fs::write(&path, lines.join("\n")).unwrap();
        path
    }
}

struct Proc(Child);

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Start `args` and read its first line of stdout.
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

/// An echo service on a unix socket.
fn echo(path: &Path) {
    let listener = std::os::unix::net::UnixListener::bind(path).unwrap();
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut conn) = conn else { return };
            std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                while let Ok(n) = conn.read(&mut buf) {
                    if n == 0 || conn.write_all(&buf[..n]).is_err() {
                        break;
                    }
                }
            });
        }
    });
}

/// The two ends of a link: a listener with the service `echo` (and the
/// service `gone`, whose origin does not exist), and a connector that
/// holds the link and runs a control socket.
struct Pair {
    server: Host,
    client: Host,
    server_fp: String,
    control: PathBuf,
    _listener: Proc,
    _connector: Proc,
}

fn pair() -> Pair {
    let (server, client) = (Host::new(), Host::new());
    let (sfp, cfp) = (server.fingerprint(), client.fingerprint());
    let origin = server.path("origin.sock");
    echo(&origin);
    let allow = server.allow(&[&cfp]);
    let sock = server.path("l.sock");
    let echo_to = format!("echo=unix:{}", origin.display());
    let gone_to = format!("gone=unix:{}", server.path("none.sock").display());
    let (listener, _) = start(&mut server.command(&[
        "link",
        "listen",
        sock.to_str().unwrap(),
        "--allow",
        allow.to_str().unwrap(),
        "--service",
        &echo_to,
        "--service",
        &gone_to,
        "--json",
    ]));
    let control = client.path("ctl.sock");
    let (connector, line) = start(&mut client.command(&[
        "link",
        "connect",
        sock.to_str().unwrap(),
        "--peer",
        &sfp,
        "--control",
        control.to_str().unwrap(),
        "--json",
    ]));
    let ready: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(ready["peer"], sfp.as_str(), "{ready}");
    assert_eq!(ready["control"], control.to_str().unwrap(), "{ready}");
    Pair {
        server,
        client,
        server_fp: sfp,
        control,
        _listener: listener,
        _connector: connector,
    }
}

impl Pair {
    /// `podnet forward` of the client install, with the rule `-L
    /// unix:SOCKET=TARGET`; the first line says that it listens.
    fn forward(&self, name: &str, target: &str) -> (Proc, PathBuf) {
        let socket = self.client.path(name);
        let rule = format!("unix:{}={target}", socket.display());
        let mut cmd = self
            .client
            .command(&["forward", "-L", &rule, "--run-for", "60"]);
        cmd.env("PODNET_LINK_CONTROL", &self.control);
        let (proc, line) = start(&mut cmd);
        assert!(line.contains("forward") && line.contains(target), "{line}");
        (proc, socket)
    }

    /// The `opened` counter of the link of the connector.
    fn opened(&self) -> u64 {
        let out = self.client.run(&[
            "link",
            "status",
            "--control",
            self.control.to_str().unwrap(),
            "--json",
        ]);
        assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
        let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
        doc["live"]["links"][0]["opened"].as_u64().unwrap()
    }
}

fn exchange(socket: &Path, message: &[u8]) -> Vec<u8> {
    let mut s = UnixStream::connect(socket).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
    s.write_all(message).unwrap();
    let mut got = vec![0u8; message.len()];
    s.read_exact(&mut got).unwrap();
    got
}

#[test]
fn a_forward_through_a_peer_reaches_a_far_unix_origin() {
    let pair = pair();
    let (_forward, socket) = pair.forward("f.sock", &format!("peer:{}/echo", pair.server_fp));
    assert_eq!(exchange(&socket, b"over a peer"), b"over a peer");
}

#[test]
fn one_stream_opens_for_each_accepted_connection() {
    let pair = pair();
    let (_forward, socket) = pair.forward("f.sock", &format!("peer:{}/echo", pair.server_fp));
    assert_eq!(pair.opened(), 0, "the forward opens no stream by itself");
    // Five connections at one time, each with its own bytes.
    let mut held: Vec<UnixStream> = Vec::new();
    for n in 0..5u8 {
        let mut s = UnixStream::connect(&socket).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
        s.write_all(&[b'a' + n; 8]).unwrap();
        held.push(s);
    }
    for (n, s) in held.iter_mut().enumerate() {
        let mut got = [0u8; 8];
        s.read_exact(&mut got).unwrap();
        assert_eq!(got, [b'a' + n as u8; 8]);
    }
    assert_eq!(pair.opened(), 5);
    drop(held);
    assert_eq!(exchange(&socket, b"sixth"), b"sixth");
    assert_eq!(pair.opened(), 6);
}

#[test]
fn a_peer_that_is_not_the_named_fingerprint_exits_77_before_it_listens() {
    let pair = pair();
    let stranger = Host::new().fingerprint();
    let socket = pair.client.path("s.sock");
    let rule = format!("unix:{}=peer:{stranger}/echo", socket.display());
    let out = pair
        .client
        .command(&["forward", "-L", &rule, "--run-for", "5"])
        .env("PODNET_LINK_CONTROL", &pair.control)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(77), "{}", text(&out.stderr));
    assert!(!socket.exists(), "no listener before the check");
    // The same through the flag --network: a peer way takes no HOST:PORT.
    let spec = format!("peer:{}/echo", pair.server_fp);
    let out = pair
        .client
        .command(&["forward", "-L", "9=example.com:80", "--network", &spec])
        .env("PODNET_LINK_CONTROL", &pair.control)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(64), "{}", text(&out.stderr));
    assert!(text(&out.stderr).contains("peer:"), "{}", text(&out.stderr));
}

#[test]
fn no_instance_that_runs_exits_69() {
    let host = Host::new();
    let fp = host.fingerprint();
    let socket = host.path("f.sock");
    let rule = format!("unix:{}=peer:{fp}/echo", socket.display());
    let out = host
        .command(&["forward", "-L", &rule, "--run-for", "5"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(69), "{}", text(&out.stderr));
    let out = host
        .command(&["forward", "-L", &rule, "--run-for", "5"])
        .env("PODNET_LINK_CONTROL", host.path("none.sock"))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(69), "{}", text(&out.stderr));
}

#[test]
fn a_service_that_the_peer_lacks_closes_the_connection_and_the_forward_stays() {
    let pair = pair();
    for service in ["nope", "gone"] {
        let (_forward, socket) =
            pair.forward("f.sock", &format!("peer:{}/{service}", pair.server_fp));
        let mut s = UnixStream::connect(&socket).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
        let _ = s.write_all(b"hello");
        let mut got = Vec::new();
        let _ = s.read_to_end(&mut got);
        assert!(got.is_empty(), "{service}: no bytes come back");
        drop(_forward);
        let _ = std::fs::remove_file(&socket);
    }
    // The link is up: the next forward reaches the echo.
    let (_forward, socket) = pair.forward("g.sock", &format!("peer:{}/echo", pair.server_fp));
    assert_eq!(exchange(&socket, b"still up"), b"still up");
}

#[test]
fn dial_asks_for_a_service_and_a_missing_one_exits_69() {
    let pair = pair();
    let sock = pair.server.path("l.sock");
    let mut dial = pair
        .client
        .command(&[
            "link",
            "dial",
            sock.to_str().unwrap(),
            "--peer",
            &pair.server_fp,
            "--service",
            "echo",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    dial.stdin.take().unwrap().write_all(b"by name").unwrap();
    let out = dial.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert_eq!(text(&out.stdout), "by name");
    let out = pair.client.run(&[
        "link",
        "dial",
        sock.to_str().unwrap(),
        "--peer",
        &pair.server_fp,
        "--service",
        "nope",
    ]);
    assert_eq!(out.status.code(), Some(69), "{}", text(&out.stderr));
    assert!(text(&out.stderr).contains("nope"), "{}", text(&out.stderr));
    let out = pair.client.run(&[
        "link",
        "dial",
        sock.to_str().unwrap(),
        "--peer",
        &pair.server_fp,
        "--service",
        "bad name",
    ]);
    assert_eq!(out.status.code(), Some(64), "{}", text(&out.stderr));
}

#[test]
fn a_connect_to_a_listener_that_is_not_trusted_exits_77_and_the_end_of_the_link_exits_69() {
    let (server, client) = (Host::new(), Host::new());
    let (sfp, cfp) = (server.fingerprint(), client.fingerprint());
    let stranger = Host::new().fingerprint();
    let allow = server.allow(&[&cfp]);
    let sock = server.path("l.sock");
    let (listener, _) = start(&mut server.command(&[
        "link",
        "listen",
        sock.to_str().unwrap(),
        "--allow",
        allow.to_str().unwrap(),
        "--json",
    ]));
    let out = client.run(&[
        "link",
        "connect",
        sock.to_str().unwrap(),
        "--peer",
        &stranger,
    ]);
    assert_eq!(out.status.code(), Some(77), "{}", text(&out.stderr));
    // The link ends with the listener: the connector exits 69.
    let (mut connector, _) = start(&mut client.command(&[
        "link",
        "connect",
        sock.to_str().unwrap(),
        "--peer",
        &sfp,
        "--json",
    ]));
    drop(listener);
    let deadline = Instant::now() + Duration::from_secs(20);
    let code = loop {
        if let Some(status) = connector.0.try_wait().unwrap() {
            break status.code();
        }
        assert!(Instant::now() < deadline, "the connector did not end");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(code, Some(69));
}

#[test]
fn a_tcp_forward_reaches_a_service_of_a_peer() {
    test_host::need_loopback!();
    let pair = pair();
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let rule = format!("{port}=peer:{}/echo", pair.server_fp);
    let (_forward, line) = start(
        pair.client
            .command(&["forward", "-L", &rule, "--run-for", "60"])
            .env("PODNET_LINK_CONTROL", &pair.control),
    );
    assert!(line.contains(&format!("127.0.0.1:{port}")), "{line}");
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
    s.write_all(b"tcp to a peer").unwrap();
    let mut got = [0u8; 13];
    s.read_exact(&mut got).unwrap();
    assert_eq!(&got, b"tcp to a peer");
}
