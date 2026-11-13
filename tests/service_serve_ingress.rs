//! `serve DIR --ingress peer:FP/SERVICE` (T-297, GitHub #471 row E5-8): the
//! file server answers on the streams that the peer opens through a running
//! `link connect`. The link crosses a unix socket; no loopback is needed.

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

/// The listener (the opener of the streams), the connector that holds
/// the link and a control socket, and the tree.
struct Ends {
    listener_host: Host,
    connector_host: Host,
    listener_fp: String,
    connector_fp: String,
    listener_control: PathBuf,
    control: PathBuf,
    tree: PathBuf,
    _listener: Proc,
    _connector: Proc,
}

/// `grant` follows the key of the listener in the allow list of the
/// connector.
fn ends(grant: &str, service_args: &[&str]) -> Ends {
    let (l, c) = (Host::new(), Host::new());
    let (lfp, cfp) = (l.fingerprint(), c.fingerprint());
    let sock = l.path("l.sock");
    let listener_control = l.path("ctl.sock");
    let (listener, _) = start(&mut l.command(&[
        "link",
        "listen",
        sock.to_str().unwrap(),
        "--allow",
        l.allow(&[&cfp]).to_str().unwrap(),
        "--control",
        listener_control.to_str().unwrap(),
        "--json",
    ]));
    let control = c.path("ctl.sock");
    let theirs = c.allow(&[format!("{lfp} {grant}").trim()]);
    let mut args = vec![
        "link",
        "connect",
        sock.to_str().unwrap(),
        "--allow",
        theirs.to_str().unwrap(),
        "--control",
        control.to_str().unwrap(),
        "--json",
    ];
    args.extend_from_slice(service_args);
    let (connector, _) = start(&mut c.command(&args));
    let tree = c.path("tree");
    std::fs::create_dir_all(&tree).unwrap();
    std::fs::write(tree.join("a.txt"), "0123456789abcdef").unwrap();
    // The listener holds the link when its status shows one.
    let up = Instant::now();
    loop {
        let out = l.run(&[
            "link",
            "status",
            "--control",
            listener_control.to_str().unwrap(),
            "--json",
        ]);
        let doc: Value = serde_json::from_slice(&out.stdout).unwrap_or(Value::Null);
        if doc["live"]["links"]
            .as_array()
            .is_some_and(|x| !x.is_empty())
        {
            break;
        }
        assert!(up.elapsed() < Duration::from_secs(10), "no link");
        std::thread::sleep(Duration::from_millis(50));
    }
    Ends {
        listener_host: l,
        connector_host: c,
        listener_fp: lfp,
        connector_fp: cfp,
        listener_control,
        control,
        tree,
        _listener: listener,
        _connector: connector,
    }
}

impl Ends {
    /// `podnet serve TREE --ingress peer:LFP/SERVICE` at the connector.
    fn serve(&self, service: &str, extra: &[&str]) -> Command {
        let at = format!("peer:{}/{service}", self.listener_fp);
        let mut args = vec!["serve", self.tree.to_str().unwrap(), "--ingress", &at];
        args.extend_from_slice(extra);
        let mut cmd = self.connector_host.command(&args);
        cmd.env("PODNET_LINK_CONTROL", &self.control);
        cmd
    }

    /// A forward at the listener: a unix socket whose connections open a
    /// stream to the service of the connector.
    fn forward(&self, service: &str) -> (Proc, PathBuf) {
        let socket = self.listener_host.path(&format!("f-{service}.sock"));
        let rule = format!(
            "unix:{}=peer:{}/{service}",
            socket.display(),
            self.connector_fp
        );
        let mut cmd = self
            .listener_host
            .command(&["forward", "-L", &rule, "--run-for", "60"]);
        cmd.env("PODNET_LINK_CONTROL", &self.listener_control);
        let (proc, _) = start(&mut cmd);
        (proc, socket)
    }
}

/// Send one HTTP request and give the whole answer.
fn get(socket: &Path, line: &str, header: &str) -> String {
    let mut s = UnixStream::connect(socket).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
    let request = format!("{line} HTTP/1.1\r\nHost: f\r\nConnection: close\r\n{header}\r\n");
    s.write_all(request.as_bytes()).unwrap();
    let mut got = Vec::new();
    let _ = s.read_to_end(&mut got);
    text(&got)
}

fn body(answer: &str) -> &str {
    answer.split_once("\r\n\r\n").map_or("", |(_, b)| b)
}

#[test]
fn a_get_and_a_range_answer_on_streams_that_the_peer_opens() {
    let ends = ends("services=files", &[]);
    let (_serve, _) = start(&mut ends.serve("files", &[]));
    let (_forward, socket) = ends.forward("files");
    let answer = get(&socket, "GET /a.txt", "");
    assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
    assert_eq!(body(&answer), "0123456789abcdef");
    let answer = get(&socket, "GET /a.txt", "Range: bytes=2-5\r\n");
    assert!(answer.starts_with("HTTP/1.1 206"), "{answer}");
    assert_eq!(body(&answer), "2345");
    let answer = get(&socket, "HEAD /a.txt", "");
    assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
    assert_eq!(body(&answer), "");
    // No listing unless asked.
    let answer = get(&socket, "GET /", "");
    assert!(!answer.contains("a.txt"), "{answer}");
}

#[test]
fn a_grant_that_does_not_name_the_service_refuses_the_stream() {
    let ends = ends("services=other", &[]);
    let (_serve, _) = start(&mut ends.serve("files", &[]));
    let (_forward, socket) = ends.forward("files");
    let answer = get(&socket, "GET /a.txt", "");
    assert!(answer.is_empty(), "{answer}");
}

#[test]
fn the_service_ends_with_the_serve_process() {
    let ends = ends("", &[]);
    let (serve, _) = start(&mut ends.serve("files", &[]));
    let (_forward, socket) = ends.forward("files");
    assert!(get(&socket, "GET /a.txt", "").starts_with("HTTP/1.1 200"));
    drop(serve);
    std::thread::sleep(Duration::from_millis(300));
    assert!(get(&socket, "GET /a.txt", "").is_empty());
}

#[test]
fn a_name_that_the_connector_serves_already_exits_64() {
    let ends = ends("", &["--service", "files=unix:/nonexistent"]);
    let out = ends.serve("files", &[]).output().unwrap();
    assert_eq!(out.status.code(), Some(64), "{}", text(&out.stderr));
}

#[test]
fn with_no_link_instance_serve_exits_69() {
    let host = Host::new();
    let tree = host.path("tree");
    std::fs::create_dir_all(&tree).unwrap();
    let fp = host.fingerprint();
    let at = format!("peer:{fp}/files");
    let out = host
        .command(&["serve", tree.to_str().unwrap(), "--ingress", &at])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(69), "{}", text(&out.stderr));
}

#[test]
fn a_bad_spec_or_a_second_road_exits_64() {
    let host = Host::new();
    let tree = host.path("tree");
    std::fs::create_dir_all(&tree).unwrap();
    let t = tree.to_str().unwrap();
    for args in [
        vec!["serve", t, "--ingress", "peer:AB"],
        vec![
            "serve",
            t,
            "--ingress",
            "peer:AB/files",
            "--local",
            "127.0.0.1:0",
        ],
        vec!["serve", t, "--ingress", "nowhere"],
    ] {
        let out = host.run(&args);
        assert_eq!(
            out.status.code(),
            Some(64),
            "{args:?}: {}",
            text(&out.stderr)
        );
    }
}
