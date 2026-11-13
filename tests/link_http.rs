//! The service `http` of a peer (T-292, GitHub #471 row E5-2): an HTTP
//! proxy on a stream of a link, in `CONNECT` and in absolute form. The link
//! crosses a unix socket; the origins are on the loopback and the tests
//! skip without one. A target is the name of a service of the listener.

mod test_host;

use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};

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

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
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

/// A listener with the services `http` and `echo` (an origin on the
/// loopback), and a stranger-proof allow list.
struct Pair {
    server: Host,
    client: Host,
    server_fp: String,
    _listener: Proc,
}

fn pair(origin: u16) -> Pair {
    let (server, client) = (Host::new(), Host::new());
    let (sfp, cfp) = (server.fingerprint(), client.fingerprint());
    let allow = server.path("allow");
    std::fs::write(&allow, &cfp).unwrap();
    let sock = server.path("l.sock");
    let target = format!("echo=127.0.0.1:{origin}");
    let (l, line) = start(&mut server.command(&[
        "link",
        "listen",
        sock.to_str().unwrap(),
        "--allow",
        allow.to_str().unwrap(),
        "--service",
        "http",
        "--service",
        &target,
        "--json",
    ]));
    let ready: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(ready["services"]["http"], "http", "{ready}");
    Pair {
        server,
        client,
        server_fp: sfp,
        _listener: l,
    }
}

impl Pair {
    fn dial(&self, host: &Host) -> Child {
        let sock = self.server.path("l.sock");
        host.command(&[
            "link",
            "dial",
            sock.to_str().unwrap(),
            "--peer",
            &self.server_fp,
            "--service",
            "http",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
    }
}

/// A TCP origin that echoes. It gives the port.
fn echo() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for conn in listener.incoming().flatten() {
            std::thread::spawn(move || {
                let mut conn = conn;
                let mut buf = [0u8; 4096];
                while let Ok(n) = conn.read(&mut buf) {
                    if n == 0 || conn.write_all(&buf[..n]).is_err() {
                        break;
                    }
                }
            });
        }
    });
    port
}

/// An HTTP origin that gives the request head it got as the body.
fn http_origin() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for conn in listener.incoming().flatten() {
            let mut conn = conn;
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") && conn.read(&mut byte).unwrap_or(0) == 1 {
                head.push(byte[0]);
            }
            let reply = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                head.len()
            );
            let _ = conn.write_all(reply.as_bytes());
            let _ = conn.write_all(&head);
        }
    });
    port
}

fn read_all(dial: &mut Child) -> String {
    let mut got = Vec::new();
    dial.stdout.as_mut().unwrap().read_to_end(&mut got).unwrap();
    String::from_utf8_lossy(&got).into_owned()
}

/// Send `request`, close stdin, and give the whole answer.
fn once(pair: &Pair, request: &[u8]) -> String {
    let mut dial = pair.dial(&pair.client);
    dial.stdin.as_mut().unwrap().write_all(request).unwrap();
    let answer = read_all(&mut dial);
    drop(dial.stdin.take());
    assert_eq!(dial.wait().unwrap().code(), Some(0));
    answer
}

#[test]
fn a_connect_through_the_peer_reaches_an_origin_at_the_peer() {
    test_host::need_loopback!();
    let pair = pair(echo());
    let mut dial = pair.dial(&pair.client);
    dial.stdin
        .as_mut()
        .unwrap()
        .write_all(b"CONNECT echo:80 HTTP/1.1\r\nHost: echo:80\r\n\r\n")
        .unwrap();
    let out = dial.stdout.as_mut().unwrap();
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    while !line.ends_with(b"\r\n\r\n") {
        out.read_exact(&mut byte).unwrap();
        line.push(byte[0]);
    }
    assert!(line.starts_with(b"HTTP/1.1 200"), "{:?}", text(&line));
    for chunk in [&b"through http"[..], b"and again"] {
        dial.stdin.as_mut().unwrap().write_all(chunk).unwrap();
        let mut got = vec![0u8; chunk.len()];
        dial.stdout.as_mut().unwrap().read_exact(&mut got).unwrap();
        assert_eq!(got, chunk);
    }
    drop(dial.stdin.take());
    assert_eq!(dial.wait().unwrap().code(), Some(0));
}

#[test]
fn an_absolute_form_request_reaches_an_http_origin_in_origin_form() {
    test_host::need_loopback!();
    let pair = pair(http_origin());
    let answer = once(
        &pair,
        b"GET http://echo/path?a=1 HTTP/1.1\r\nHost: echo\r\nProxy-Connection: keep-alive\r\n\r\n",
    );
    assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
    assert!(answer.contains("GET /path?a=1 HTTP/1.1"), "{answer}");
    assert!(
        !answer.to_lowercase().contains("proxy-connection"),
        "{answer}"
    );
    assert!(answer.contains("Connection: close"), "{answer}");
}

#[test]
fn a_host_that_is_no_service_name_gets_403_and_no_lookup() {
    test_host::need_loopback!();
    let port = echo();
    let pair = pair(port);
    for target in [
        "CONNECT nobody:80 HTTP/1.1\r\n\r\n".to_string(),
        format!("CONNECT 127.0.0.1:{port} HTTP/1.1\r\n\r\n"),
        "GET http://example.com/ HTTP/1.1\r\nHost: example.com\r\n\r\n".to_string(),
    ] {
        let answer = once(&pair, target.as_bytes());
        assert!(answer.starts_with("HTTP/1.1 403"), "{target:?}: {answer}");
    }
}

#[test]
fn a_request_the_service_does_not_take_is_refused() {
    test_host::need_loopback!();
    let pair = pair(echo());
    // Origin form: no proxy request.
    let answer = once(&pair, b"GET /x HTTP/1.1\r\nHost: echo\r\n\r\n");
    assert!(answer.starts_with("HTTP/1.1 400"), "{answer}");
    // A CONNECT has no body.
    let answer = once(
        &pair,
        b"CONNECT echo:80 HTTP/1.1\r\nContent-Length: 4\r\n\r\nabcd",
    );
    assert!(answer.starts_with("HTTP/1.1 400"), "{answer}");
    // A head of more than 8 KiB.
    let mut long = b"GET http://echo/ HTTP/1.1\r\nX-Fill: ".to_vec();
    long.extend(std::iter::repeat_n(b'a', 9000));
    long.extend_from_slice(b"\r\n\r\n");
    let answer = once(&pair, &long);
    assert!(answer.starts_with("HTTP/1.1 431"), "{answer}");
}

#[test]
fn a_peer_that_is_not_on_the_allow_list_gets_77_before_the_service() {
    let pair = pair(9);
    let stranger = Host::new();
    let sock = pair.server.path("l.sock");
    let out = stranger
        .command(&[
            "link",
            "dial",
            sock.to_str().unwrap(),
            "--peer",
            &pair.server_fp,
            "--service",
            "http",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(77), "{}", text(&out.stderr));
}

#[test]
fn the_service_http_takes_no_target() {
    let host = Host::new();
    let allow = host.path("allow");
    std::fs::write(&allow, "").unwrap();
    let out = host.run(&[
        "link",
        "listen",
        host.path("l.sock").to_str().unwrap(),
        "--allow",
        allow.to_str().unwrap(),
        "--service",
        "http=127.0.0.1:3128",
    ]);
    assert_eq!(out.status.code(), Some(64), "{}", text(&out.stderr));
    assert!(text(&out.stderr).contains("http"), "{}", text(&out.stderr));
}
