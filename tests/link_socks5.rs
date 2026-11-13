//! The service `socks5` of a peer (T-291, GitHub #471 row E5-1): the server
//! side of SOCKS5 on a stream of a link. The link crosses a unix socket;
//! the origins of `CONNECT` are on the loopback and the tests skip without
//! one.

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

/// A listener with the service `socks5` and a stranger-proof allow list.
struct Pair {
    server: Host,
    client: Host,
    server_fp: String,
    _listener: Proc,
}

fn pair() -> Pair {
    let (server, client) = (Host::new(), Host::new());
    let (sfp, cfp) = (server.fingerprint(), client.fingerprint());
    let allow = server.path("allow");
    std::fs::write(&allow, &cfp).unwrap();
    let sock = server.path("l.sock");
    let (l, line) = start(&mut server.command(&[
        "link",
        "listen",
        sock.to_str().unwrap(),
        "--allow",
        allow.to_str().unwrap(),
        "--service",
        "socks5",
        "--json",
    ]));
    let ready: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(ready["services"]["socks5"], "socks5", "{ready}");
    Pair {
        server,
        client,
        server_fp: sfp,
        _listener: l,
    }
}

impl Pair {
    /// `link dial --service socks5` of the client install, as the
    /// opener of the stream.
    fn dial(&self, host: &Host) -> Child {
        let sock = self.server.path("l.sock");
        host.command(&[
            "link",
            "dial",
            sock.to_str().unwrap(),
            "--peer",
            &self.server_fp,
            "--service",
            "socks5",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
    }
}

/// The greeting and a request of `command` to `atyp` with `addr`.
fn request(command: u8, atyp: u8, addr: &[u8], port: u16) -> Vec<u8> {
    let mut out = vec![5, 1, 0, 5, command, 0, atyp];
    out.extend_from_slice(addr);
    out.extend_from_slice(&port.to_be_bytes());
    out
}

fn by_name(command: u8, name: &str, port: u16) -> Vec<u8> {
    let mut addr = vec![name.len() as u8];
    addr.extend_from_slice(name.as_bytes());
    request(command, 3, &addr, port)
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

/// Send `bytes`, read the method answer and the reply of the request.
fn handshake(dial: &mut Child, bytes: &[u8]) -> ([u8; 2], [u8; 10]) {
    dial.stdin.as_mut().unwrap().write_all(bytes).unwrap();
    let out = dial.stdout.as_mut().unwrap();
    let (mut method, mut reply) = ([0u8; 2], [0u8; 10]);
    out.read_exact(&mut method).unwrap();
    out.read_exact(&mut reply).unwrap();
    (method, reply)
}

fn through(pair: &Pair, request: &[u8]) -> Child {
    let mut dial = pair.dial(&pair.client);
    let (method, reply) = handshake(&mut dial, request);
    assert_eq!(method, [5, 0]);
    assert_eq!(&reply[..2], &[5, 0], "the CONNECT is not open: {reply:?}");
    dial
}

fn say(dial: &mut Child, text: &[u8]) {
    dial.stdin.as_mut().unwrap().write_all(text).unwrap();
    let mut got = vec![0u8; text.len()];
    dial.stdout.as_mut().unwrap().read_exact(&mut got).unwrap();
    assert_eq!(got, text);
}

#[test]
fn a_connect_through_the_peer_reaches_an_origin_at_the_peer() {
    test_host::need_loopback!();
    let pair = pair();
    let port = echo();
    let mut dial = through(&pair, &request(1, 1, &[127, 0, 0, 1], port));
    say(&mut dial, b"through socks5");
    say(&mut dial, b"and again");
}

#[test]
fn a_name_resolves_at_the_peer() {
    test_host::need_loopback!();
    test_host::need_localhost!();
    let pair = pair();
    let port = echo();
    let mut dial = through(&pair, &by_name(1, "localhost", port));
    say(&mut dial, b"by name");
}

#[test]
fn a_connect_to_a_closed_port_gets_a_failure_reply_and_the_stream_ends() {
    test_host::need_loopback!();
    let pair = pair();
    let closed = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let mut dial = pair.dial(&pair.client);
    let (method, reply) = handshake(&mut dial, &request(1, 1, &[127, 0, 0, 1], closed));
    assert_eq!(method, [5, 0]);
    assert_eq!(reply[0], 5);
    assert_ne!(reply[1], 0, "a failed CONNECT must not say success");
    drop(dial.stdin.take());
    assert_eq!(dial.wait().unwrap().code(), Some(0));
}

#[test]
fn bind_and_udp_associate_are_not_served_and_say_command_not_supported() {
    let pair = pair();
    let mut dial = pair.dial(&pair.client);
    let (_, reply) = handshake(&mut dial, &request(2, 1, &[127, 0, 0, 1], 9));
    assert_eq!(&reply[..2], &[5, 7], "BIND: {reply:?}");
    drop(dial.stdin.take());
    assert_eq!(dial.wait().unwrap().code(), Some(0));
}

#[test]
fn a_stream_that_does_not_speak_socks5_ends_with_no_answer() {
    let pair = pair();
    let mut dial = pair.dial(&pair.client);
    dial.stdin
        .as_mut()
        .unwrap()
        .write_all(b"CONNECT 127.0.0.1:9 HTTP/1.1\r\n\r\n")
        .unwrap();
    let mut got = Vec::new();
    dial.stdout.as_mut().unwrap().read_to_end(&mut got).unwrap();
    assert!(got.is_empty(), "{:?}", String::from_utf8_lossy(&got));
    // The dial ends with stdin still open.
    assert_eq!(dial.wait().unwrap().code(), Some(0));
}

#[test]
fn a_peer_that_is_not_on_the_allow_list_gets_77_before_the_service() {
    let pair = pair();
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
            "socks5",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(77), "{}", text(&out.stderr));
}

#[test]
fn the_service_socks5_takes_no_target() {
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
        "socks5=127.0.0.1:1080",
    ]);
    assert_eq!(out.status.code(), Some(64), "{}", text(&out.stderr));
    assert!(
        text(&out.stderr).contains("socks5"),
        "{}",
        text(&out.stderr)
    );
}
