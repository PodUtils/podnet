//! `--seal` of `podnet link listen` and `podnet link dial` (T-298, T-299,
//! T-300): the streams of the service `raw` run over the records of
//! `pod-e2e`. The link crosses a unix socket and the origin is a unix
//! socket, so no test needs a loopback.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};

use serde_json::Value;

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

    fn fingerprint(&self) -> String {
        let out = self
            .command(&["link", "status", "--create", "--json"])
            .output()
            .unwrap();
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

fn start(cmd: &mut Command) -> Proc {
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
        panic!("no first line: {err}");
    }
    Proc(child)
}

/// A listener with an echo origin, and the key of its dialer.
struct Setup {
    server: Host,
    client: Host,
    sfp: String,
    sock: PathBuf,
    _listener: Proc,
}

fn setup(listen_args: &[&str]) -> Setup {
    let (server, client) = (Host::new(), Host::new());
    let (sfp, cfp) = (server.fingerprint(), client.fingerprint());
    let origin = server.path("origin.sock");
    let echo = UnixListener::bind(&origin).unwrap();
    std::thread::spawn(move || {
        for conn in echo.incoming().flatten() {
            std::thread::spawn(move || {
                let (mut r, mut w) = (&conn, &conn);
                let _ = std::io::copy(&mut r, &mut w);
                let _ = conn.shutdown(std::net::Shutdown::Write);
            });
        }
    });
    let allow = server.path("allow");
    std::fs::write(&allow, &cfp).unwrap();
    let sock = server.path("l.sock");
    let to = format!("unix:{}", origin.display());
    let mut args = vec![
        "link",
        "listen",
        sock.to_str().unwrap(),
        "--allow",
        allow.to_str().unwrap(),
        "--to",
        &to,
        "--json",
    ];
    args.extend_from_slice(listen_args);
    let listener = start(&mut server.command(&args));
    Setup {
        server,
        client,
        sfp,
        sock,
        _listener: listener,
    }
}

impl Setup {
    fn dial(&self, address_suffix: &str, extra: &[&str], input: &[u8]) -> Output {
        let address = format!("{}{address_suffix}", self.sock.display());
        let mut args = vec!["link", "dial", &address, "--peer", &self.sfp];
        args.extend_from_slice(extra);
        let mut child = self
            .client
            .command(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut stdin = child.stdin.take().unwrap();
        let input = input.to_vec();
        let feeder = std::thread::spawn(move || {
            let _ = stdin.write_all(&input);
        });
        let out = child.wait_with_output().unwrap();
        let _ = feeder.join();
        out
    }
}

#[test]
fn a_sealed_stream_of_the_service_raw_echoes() {
    let s = setup(&["--seal"]);
    let data = b"sealed over a unix link ".repeat(2000);
    let out = s.dial("", &["--seal"], &data);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.stdout, data);
    let _ = &s.server;
}

#[test]
fn the_query_keys_pad_and_compress_work_with_seal() {
    let s = setup(&["--seal"]);
    let data = b"padded and compressed ".repeat(3000);
    let out = s.dial("?pad=on&compress=snappy", &["--seal"], &data);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.stdout, data);
}

#[test]
fn the_keys_pad_and_compress_need_seal() {
    let s = setup(&[]);
    for key in ["pad=on", "compress=snappy"] {
        let out = s.dial(&format!("?{key}"), &[], b"x");
        assert_eq!(out.status.code(), Some(64), "{key}");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(err.contains("--seal"), "{err}");
    }
}

#[test]
fn a_dialer_that_seals_does_not_talk_to_a_listener_that_does_not() {
    let s = setup(&[]);
    let out = s.dial("", &["--seal"], b"hello");
    assert!(!out.status.success());
    assert!(out.stdout.is_empty());
}

/// A sealed client of the service `echo` of the connector, through the
/// `forward` of the listener: the keys of the listener's install.
async fn sealed_echo(socket: &std::path::Path, server: &Host, cfp: &str, data: &[u8]) {
    use podnet::link_record::{self as record, Options};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let me = podnet::identity::create(&server.path("state")).unwrap();
    let stream = tokio::net::UnixStream::connect(socket).await.unwrap();
    let mut sealed = record::dial(stream, &me, None, Options::default())
        .await
        .unwrap();
    assert_eq!(sealed.peer().display_fingerprint(), cfp);
    sealed.write_all(data).await.unwrap();
    let mut got = vec![0u8; data.len()];
    sealed.read_exact(&mut got).await.unwrap();
    assert_eq!(got, data);
}

#[tokio::test]
async fn reconnect_of_a_sealed_link_connect_gives_a_sealed_new_link() {
    let (server, client) = (Host::new(), Host::new());
    let (sfp, cfp) = (server.fingerprint(), client.fingerprint());
    let origin = client.path("origin.sock");
    let echo = UnixListener::bind(&origin).unwrap();
    std::thread::spawn(move || {
        for conn in echo.incoming().flatten() {
            std::thread::spawn(move || {
                let (mut r, mut w) = (&conn, &conn);
                let _ = std::io::copy(&mut r, &mut w);
            });
        }
    });
    let allow = server.path("allow");
    std::fs::write(&allow, &cfp).unwrap();
    let sock = server.path("l.sock");
    let control = server.path("ctl.sock");
    let _listener = start(&mut server.command(&[
        "link",
        "listen",
        sock.to_str().unwrap(),
        "--allow",
        allow.to_str().unwrap(),
        "--control",
        control.to_str().unwrap(),
        "--json",
    ]));
    let service = format!("echo=unix:{}", origin.display());
    let out = client
        .command(&[
            "link",
            "connect",
            sock.to_str().unwrap(),
            "--peer",
            &sfp,
            "--seal",
            "--service",
            &service,
            "--detach",
            "--name",
            "cn",
        ])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let rule = format!("unix:{}=peer:{cfp}/echo", server.path("f.sock").display());
    let mut forward = server.command(&["forward", "-L", &rule, "--run-for", "60"]);
    forward.env("PODNET_LINK_CONTROL", &control);
    let _forward = start(&mut forward);
    let socket = server.path("f.sock");
    sealed_echo(&socket, &server, &cfp, b"before the reconnect").await;
    // A plain stream to the sealed service gets no echo.
    {
        use std::io::{Read, Write};
        let mut s = std::os::unix::net::UnixStream::connect(&socket).unwrap();
        s.set_read_timeout(Some(std::time::Duration::from_secs(3)))
            .unwrap();
        let _ = s.write_all(b"plain bytes");
        let mut buf = [0u8; 11];
        assert!(s.read_exact(&mut buf).is_err() || &buf != b"plain bytes");
    }
    let out = client
        .command(&["link", "reconnect", "cn"])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("reconnect link 1 -> 2"));
    // The new link serves the sealed service again, and still seals only.
    sealed_echo(&socket, &server, &cfp, b"after the reconnect").await;
    let _ = client.command(&["stop", "cn"]).output();
}
