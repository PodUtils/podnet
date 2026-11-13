//! A connector accepts streams (T-266 follow-up, T-297, T-291): `podnet
//! link connect --service NAME=TARGET` serves the streams that the far end
//! (the listener) opens, with the allow list and its grants applied before
//! any service. The link crosses a unix socket and the origins are unix
//! sockets, except the test of SOCKS5, which needs a loopback.

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

/// The two ends of a link. The listener runs a control socket and no
/// service. The connector serves `--service` and holds the link.
struct Pair {
    server: Host,
    _client: Host,
    client_fp: String,
    control: PathBuf,
    connector: Proc,
    _listener: Proc,
}

/// `grant` is the text after the key of the listener in the allow list of
/// the connector (empty for a bare key).
fn pair(services: &[String], grant: &str) -> Pair {
    let (server, client) = (Host::new(), Host::new());
    let (sfp, cfp) = (server.fingerprint(), client.fingerprint());
    let allow = server.allow(&[&cfp]);
    let sock = server.path("l.sock");
    let control = server.path("ctl.sock");
    let (listener, _) = start(&mut server.command(&[
        "link",
        "listen",
        sock.to_str().unwrap(),
        "--allow",
        allow.to_str().unwrap(),
        "--control",
        control.to_str().unwrap(),
        "--json",
    ]));
    let line = format!("{sfp} {grant}");
    let theirs = client.allow(&[line.trim()]);
    let mut args = vec![
        "link".to_string(),
        "connect".into(),
        sock.display().to_string(),
        "--allow".into(),
        theirs.display().to_string(),
        "--json".into(),
    ];
    for service in services {
        args.push("--service".into());
        args.push(service.clone());
    }
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let (connector, line) = start(&mut client.command(&args));
    let ready: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(ready["peer"], sfp.as_str(), "{ready}");
    // The listener holds the link when its `status` shows one.
    let up = Instant::now();
    loop {
        let out = server.run(&[
            "link",
            "status",
            "--control",
            control.to_str().unwrap(),
            "--json",
        ]);
        let doc: Value = serde_json::from_slice(&out.stdout).unwrap_or(Value::Null);
        if doc["live"]["links"]
            .as_array()
            .is_some_and(|l| !l.is_empty())
        {
            break;
        }
        assert!(up.elapsed() < Duration::from_secs(10), "no link");
        std::thread::sleep(Duration::from_millis(50));
    }
    Pair {
        server,
        _client: client,
        client_fp: cfp,
        control,
        connector,
        _listener: listener,
    }
}

impl Pair {
    /// `podnet forward` of the listener install: a unix listener whose
    /// connections open a stream to a service of the connector.
    fn forward(&self, name: &str, service: &str) -> (Proc, PathBuf) {
        let socket = self.server.path(name);
        let rule = format!(
            "unix:{}=peer:{}/{service}",
            socket.display(),
            self.client_fp
        );
        let mut cmd = self
            .server
            .command(&["forward", "-L", &rule, "--run-for", "60"]);
        cmd.env("PODNET_LINK_CONTROL", &self.control);
        let (proc, line) = start(&mut cmd);
        assert!(line.contains("forward"), "{line}");
        (proc, socket)
    }

    /// A field of the first link row of the listener.
    fn row(&self, key: &str) -> u64 {
        let out = self.server.run(&[
            "link",
            "status",
            "--control",
            self.control.to_str().unwrap(),
            "--json",
        ]);
        let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
        doc["live"]["links"][0][key].as_u64().unwrap()
    }
}

/// Send `message`, and give what comes back until the far end closes (or
/// `want` bytes).
fn exchange(socket: &Path, message: &[u8], want: usize) -> Vec<u8> {
    let mut s = UnixStream::connect(socket).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
    s.write_all(message).unwrap();
    let mut got = vec![0u8; want];
    let mut have = 0;
    while have < want {
        match s.read(&mut got[have..]) {
            Ok(0) | Err(_) => break,
            Ok(n) => have += n,
        }
    }
    got.truncate(have);
    got
}

#[test]
fn a_stream_that_the_far_end_opens_reaches_the_service_of_the_connector() {
    let client_origin = std::env::temp_dir().join(format!("pn-acc-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&client_origin);
    echo(&client_origin);
    let pair = pair(&[format!("echo=unix:{}", client_origin.display())], "");
    let (_forward, socket) = pair.forward("f.sock", "echo");
    assert_eq!(
        exchange(&socket, b"to the connector", 16),
        b"to the connector"
    );
    assert_eq!(exchange(&socket, b"again", 5), b"again");
    assert_eq!(pair.row("opened"), 2);
    let _ = std::fs::remove_file(&client_origin);
}

#[test]
fn a_service_that_the_connector_lacks_closes_the_connection_and_the_link_stays() {
    let origin = std::env::temp_dir().join(format!("pn-acc2-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&origin);
    echo(&origin);
    let pair = pair(&[format!("echo=unix:{}", origin.display())], "");
    {
        let (_forward, socket) = pair.forward("f.sock", "missing");
        assert!(exchange(&socket, b"x", 1).is_empty());
    }
    let (_forward, socket) = pair.forward("g.sock", "echo");
    assert_eq!(exchange(&socket, b"still", 5), b"still");
    let _ = std::fs::remove_file(&origin);
}

#[test]
fn a_connector_with_no_service_refuses_each_stream() {
    let pair = pair(&[], "");
    let (_forward, socket) = pair.forward("f.sock", "echo");
    assert!(exchange(&socket, b"x", 1).is_empty());
    assert!(pair.connector.0.id() > 0);
}

#[test]
fn a_grant_of_the_allow_list_applies_before_the_service() {
    let origin = std::env::temp_dir().join(format!("pn-acc3-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&origin);
    echo(&origin);
    let both = [
        format!("echo=unix:{}", origin.display()),
        format!("other=unix:{}", origin.display()),
    ];
    let pair = pair(&both, "services=echo");
    let (_forward, socket) = pair.forward("f.sock", "echo");
    assert_eq!(exchange(&socket, b"granted", 7), b"granted");
    assert_eq!(pair.row("refused_service"), 0);
    drop(_forward);
    let (_forward, socket) = pair.forward("g.sock", "other");
    assert!(exchange(&socket, b"denied", 6).is_empty());
    let _ = std::fs::remove_file(&origin);
}

#[test]
fn the_connector_gives_a_bad_service_flag_exit_64() {
    let host = Host::new();
    for bad in ["no equals sign", "bad name=unix:/x", "socks5=unix:/x"] {
        let out = host.run(&[
            "link",
            "connect",
            "/nonexistent",
            "--peer",
            "AB",
            "--service",
            bad,
        ]);
        assert_eq!(out.status.code(), Some(64), "{bad}: {}", text(&out.stderr));
    }
}
