//! The service `ss` of a peer (T-294, GitHub #471 row E5-5): the accept
//! side of the AEAD form of Shadowsocks (SIP004). The peer is the client
//! side of `pod-egress`: its codec in the tests of the library, and the
//! `ss://` network of podnet (T-127) through `forward -L
//! LISTEN=peer:FINGERPRINT/ss` in the test of the commands. The published
//! SIP004 vectors of `pod-testkit` open at the service: a round trip
//! through one codec alone proves nothing of the protocol.
#![cfg(feature = "ss")]

mod test_host;

use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pod_core::BoxedIo;
use pod_egress::shadowsocks::{address, master_key, Decoder, Encoder, Method};
use podnet::exit::code_of;
use podnet::link_ss::{serve, serve_with, Key, Server, KEY_FILE_VARIABLE, KEY_VARIABLE};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// A made-up password of the tests: no server has it.
const PASSWORD: &str = "made-up-ss-test-password";

/// The password of the published vectors.
const VECTOR_PASSWORD: &str = "foobar";

fn server(method: Method, password: &str) -> Server {
    let key = Key::parse(&format!("{}:{password}", method.name()), KEY_VARIABLE).unwrap();
    Server::new(key)
}

/// The start of a stream of the client side of `pod-egress`: the salt and
/// the first chunk, which holds the address of `host:port` and `payload`.
fn request(method: Method, password: &str, host: &str, port: u16, payload: &[u8]) -> Vec<u8> {
    let master = master_key(password.as_bytes(), method.key_len());
    let mut first = address(host, port).unwrap();
    first.extend_from_slice(payload);
    let mut wire = Vec::new();
    Encoder::new(method, &master)
        .unwrap()
        .first(&first, &mut wire)
        .unwrap();
    wire
}

/// What one stream gave: the result of the service, the bytes back to the
/// peer, the targets that it dialed and the bytes that the target got.
struct Session {
    served: anyhow::Result<()>,
    back: Vec<u8>,
    dialed: Vec<(String, u16)>,
    got: Vec<u8>,
}

/// Send `wire` and an end to the service, with a target of the test.
async fn session(server: &Server, wire: &[u8]) -> Session {
    let (mut client, far) = tokio::io::duplex(1 << 20);
    let (near_target, mut target) = tokio::io::duplex(1 << 20);
    let dialed = Arc::new(Mutex::new(Vec::new()));
    let seen = dialed.clone();
    let connect = move |host: String, port: u16| {
        seen.lock().unwrap().push((host, port));
        async move { Ok(Box::new(near_target) as BoxedIo) }
    };
    let reader = tokio::spawn(async move {
        let mut got = Vec::new();
        let _ = target.read_to_end(&mut got).await;
        got
    });
    client.write_all(wire).await.unwrap();
    client.shutdown().await.unwrap();
    let served = tokio::time::timeout(Duration::from_secs(20), serve_with(server, far, connect))
        .await
        .expect("the service ends");
    let mut back = Vec::new();
    client.read_to_end(&mut back).await.unwrap();
    let got = reader.await.unwrap();
    let dialed = dialed.lock().unwrap().clone();
    Session {
        served,
        back,
        dialed,
        got,
    }
}

/// The published SIP004 vectors (`pod_testkit::vectors::SHADOWSOCKS`): the
/// salt and the sealed address of `example.com:22` and `hello`, sealed by
/// OpenSSL. The service dials `example.com:22` and sends `hello` to it.
#[tokio::test]
async fn the_published_sip004_requests_open_at_the_service() {
    for vector in pod_testkit::vectors::SHADOWSOCKS {
        let method = Method::parse(vector.method).unwrap();
        let master = master_key(VECTOR_PASSWORD.as_bytes(), method.key_len());
        assert_eq!(master.as_slice(), vector.master_key, "{}", vector.method);
        let server = server(method, VECTOR_PASSWORD);
        let done = session(&server, vector.wire).await;
        assert!(done.served.is_ok(), "{}: {:?}", vector.method, done.served);
        assert_eq!(
            done.dialed,
            [("example.com".to_string(), 22)],
            "{}",
            vector.method
        );
        assert_eq!(done.got, b"hello", "{}", vector.method);
        assert!(
            done.back.is_empty(),
            "{}: the target said nothing",
            vector.method
        );
    }
}

/// An echo target on the loopback.
async fn echo() -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut conn, _)) = listener.accept().await {
            tokio::spawn(async move {
                let (mut r, mut w) = conn.split();
                let _ = tokio::io::copy(&mut r, &mut w).await;
            });
        }
    });
    port
}

/// Each method of SIP004 carries a session both ways through the direct
/// dial: the client side of `pod-egress` seals, the service opens, the echo
/// answers, and the answer opens with the codec of the client.
#[tokio::test]
async fn each_method_carries_a_session_both_ways() {
    test_host::need_loopback!();
    let port = echo().await;
    let data: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();
    for method in Method::ALL {
        let server = Arc::new(server(method, PASSWORD));
        let master = master_key(PASSWORD.as_bytes(), method.key_len());
        let (mut client, far) = tokio::io::duplex(1 << 20);
        let served = tokio::spawn({
            let server = server.clone();
            async move { serve(&server, far).await }
        });
        let mut encoder = Encoder::new(method, &master).unwrap();
        let mut wire = Vec::new();
        encoder
            .first(&address("127.0.0.1", port).unwrap(), &mut wire)
            .unwrap();
        encoder.seal(&data, &mut wire).unwrap();
        client.write_all(&wire).await.unwrap();
        client.shutdown().await.unwrap();
        let mut back = Vec::new();
        tokio::time::timeout(Duration::from_secs(20), client.read_to_end(&mut back))
            .await
            .expect("the answer within the limit")
            .unwrap();
        let mut decoder = Decoder::new(method, &master);
        decoder.feed(&back);
        let mut plain = Vec::new();
        while let Some(piece) = decoder.chunk().unwrap() {
            plain.extend_from_slice(&piece);
        }
        assert_eq!(decoder.buffered(), 0, "{method:?}");
        assert!(plain == data, "{method:?}: the echo came back changed");
        served.await.unwrap().unwrap();
    }
}

/// One rule for each stream that does not open: the service ends it with
/// no byte back, dials no target where the fault comes first, and says why
/// in one line with no password.
#[tokio::test]
async fn a_stream_that_does_not_open_ends_with_no_byte_and_one_line() {
    let method = Method::Aes256Gcm;
    let good = request(method, PASSWORD, "10.0.0.2", 22, b"x");
    let salt = &good[..method.salt_len()];
    let mut no_address = Vec::new();
    Encoder::new(method, &master_key(PASSWORD.as_bytes(), 32))
        .unwrap()
        .seal(&[9, 1, 2, 3], &mut no_address)
        .unwrap();
    // A whole first chunk, then a second chunk with its last 3 bytes cut.
    let mut cut = Vec::new();
    let mut encoder = Encoder::new(method, &master_key(PASSWORD.as_bytes(), 32)).unwrap();
    encoder
        .first(&address("10.0.0.2", 22).unwrap(), &mut cut)
        .unwrap();
    encoder.seal(b"more", &mut cut).unwrap();
    cut.truncate(cut.len() - 3);
    let other_key = request(method, "another made-up password", "10.0.0.2", 22, b"x");
    // The class of each line: 77 for a key that is not the service's, 65
    // for bytes that are no Shadowsocks stream.
    let cases: [(&str, Vec<u8>, &str, i32, usize); 5] = [
        ("another key", other_key, "did not open", 77, 0),
        ("no address", no_address, "no target address", 65, 0),
        ("no salt", salt[..5].to_vec(), "before its salt", 65, 0),
        (
            "no first chunk",
            good[..method.salt_len() + 3].to_vec(),
            "before its first chunk",
            65,
            0,
        ),
        // The first chunk opened and the target was dialed; the rest is
        // cut inside a chunk.
        ("cut short", cut, "inside a chunk", 65, 1),
    ];
    for (name, wire, words, code, dials) in cases {
        let server = server(method, PASSWORD);
        let done = session(&server, &wire).await;
        let err = done.served.expect_err(name);
        assert_eq!(code_of(&err), code, "{name}: {err:#}");
        let err = format!("{err:#}");
        assert!(err.contains(words), "{name}: {err}");
        assert!(!err.contains(PASSWORD), "{name}: {err}");
        assert!(done.back.is_empty(), "{name}: bytes back");
        assert_eq!(done.dialed.len(), dials, "{name}");
    }
    // A stream with no byte (the reach probe of a client) is no fault.
    let empty = session(&server(method, PASSWORD), b"").await;
    assert!(empty.served.is_ok() && empty.dialed.is_empty() && empty.back.is_empty());
    // A replay: the same stream again is refused after its first chunk.
    let server = server(method, PASSWORD);
    assert!(session(&server, &good).await.served.is_ok());
    let again = session(&server, &good).await;
    let err = again.served.expect_err("a replay");
    assert_eq!(code_of(&err), 77, "{err:#}");
    assert!(format!("{err:#}").contains("a replay"), "{err:#}");
    assert!(again.dialed.is_empty() && again.back.is_empty());
}

/// A target that refuses: one line that names the target, no byte back.
#[tokio::test]
async fn a_target_that_refuses_is_named() {
    test_host::need_loopback!();
    let closed = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let server = Arc::new(server(Method::Chacha20IetfPoly1305, PASSWORD));
    let wire = request(
        Method::Chacha20IetfPoly1305,
        PASSWORD,
        "127.0.0.1",
        closed,
        b"x",
    );
    let (mut client, far) = tokio::io::duplex(1 << 16);
    client.write_all(&wire).await.unwrap();
    client.shutdown().await.unwrap();
    let err = serve(&server, far).await.expect_err("a refused target");
    assert_eq!(code_of(&err), 69, "{err:#}");
    assert!(
        format!("{err:#}").contains(&format!("the target 127.0.0.1:{closed}")),
        "{err:#}"
    );
    let mut back = Vec::new();
    client.read_to_end(&mut back).await.unwrap();
    assert!(back.is_empty());
}

/// One rule for each key: METHOD:PASSWORD with a method of SIP004. A bad
/// key exits 64, names the variable, and holds no password.
#[test]
fn a_key_is_a_method_of_sip004_and_a_password() {
    for method in Method::ALL {
        let key = Key::parse(&format!("{}:{PASSWORD}", method.name()), KEY_VARIABLE).unwrap();
        assert_eq!(key.method(), method);
        assert!(!format!("{key:?}").contains(PASSWORD));
    }
    let password_in = |m: &str| format!("{m}:{PASSWORD}");
    let cases = [
        (password_in("2022-blake3-aes-256-gcm"), "SIP004 alone"),
        (password_in("rc4-md5"), "is not one of"),
        (format!("{PASSWORD}-and-no-colon"), "METHOD:PASSWORD"),
        (
            "aes-256-gcm:".to_string(),
            "the password after the colon is empty",
        ),
        (format!("aes-256-gcm:{PASSWORD}\nsecond line"), "one line"),
    ];
    for (text, words) in cases {
        let err = Key::parse(&text, KEY_VARIABLE).expect_err(&text);
        let line = format!("{err:#}");
        assert_eq!(code_of(&err), 64, "{line}");
        assert!(
            line.starts_with(KEY_VARIABLE) && line.contains(words),
            "{line}"
        );
        assert!(!line.contains(PASSWORD), "{line}");
    }
    let none = |_: &str| None;
    let err = Key::from_env(&none).unwrap_err();
    assert_eq!(code_of(&err), 64);
    assert!(format!("{err:#}").contains(KEY_FILE_VARIABLE), "{err:#}");
}

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
            .env("PODNET_RUN_DIR", self.path("run"));
        for name in [
            "PODNET_LINK_CONTROL",
            "RUST_LOG",
            KEY_FILE_VARIABLE,
            KEY_VARIABLE,
            "PODNET_SHADOWSOCKS_KEY",
            "PODNET_SHADOWSOCKS_KEY_FILE",
            "PODNET_PROXY",
            "HTTPS_PROXY",
            "https_proxy",
            "ALL_PROXY",
            "all_proxy",
        ] {
            cmd.env_remove(name);
        }
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

    /// A file of mode 0600 with `text`.
    fn secret(&self, name: &str, text: &str) -> String {
        use std::os::unix::fs::OpenOptionsExt;
        let path = self.path(name);
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .unwrap();
        writeln!(file, "{text}").unwrap();
        path.to_str().unwrap().to_string()
    }
}

struct Proc(Child);

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Start `cmd` and read its first line of stdout.
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

fn free_tcp_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// An echo target on the loopback, in threads.
fn echo_thread() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
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
    port
}

/// The whole road: `link listen --service ss` at one install; at the other,
/// `link connect`, `forward -L PORT=peer:FINGERPRINT/ss`, and `forward
/// --network ss://127.0.0.1:PORT` (the client of T-127) to an echo target.
/// The echo comes back; another key gets none; no output holds a password.
#[test]
fn the_ss_network_of_podnet_reaches_a_target_through_the_service_of_a_peer() {
    test_host::need_loopback!();
    let (server, client) = (Host::new(), Host::new());
    let (sfp, cfp) = (server.fingerprint(), client.fingerprint());
    let allow = server.path("allow");
    std::fs::write(&allow, &cfp).unwrap();
    let key = format!("aes-256-gcm:{PASSWORD}");
    let sock = server.path("l.sock");
    let mut listen = server.command(&[
        "link",
        "listen",
        sock.to_str().unwrap(),
        "--allow",
        allow.to_str().unwrap(),
        "--service",
        "ss",
        "--json",
    ]);
    listen.env(KEY_FILE_VARIABLE, server.secret("ss.key", &key));
    let (mut listener, line) = start(&mut listen);
    let ready: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(
        ready["services"]["ss"], "shadowsocks aes-256-gcm",
        "{ready}"
    );
    let control = client.path("ctl.sock");
    let (_connector, _) = start(&mut client.command(&[
        "link",
        "connect",
        sock.to_str().unwrap(),
        "--peer",
        &sfp,
        "--control",
        control.to_str().unwrap(),
        "--json",
    ]));
    let at = free_tcp_port();
    let rule = format!("127.0.0.1:{at}=peer:{sfp}/ss");
    let mut door = client.command(&["forward", "-L", &rule, "--run-for", "60"]);
    door.env("PODNET_LINK_CONTROL", &control);
    let (_door, _) = start(&mut door);
    let target = echo_thread();
    let road = |name: &str, key: &str| {
        let port = free_tcp_port();
        let rule = format!("127.0.0.1:{port}=127.0.0.1:{target}");
        let network = format!("ss://127.0.0.1:{at}");
        let mut cmd = client.command(&[
            "forward",
            "-L",
            &rule,
            "--network",
            &network,
            "--run-for",
            "60",
        ]);
        cmd.env("PODNET_SHADOWSOCKS_KEY_FILE", client.secret(name, key));
        let (proc, first) = start(&mut cmd);
        assert!(first.contains(&format!("via {network}")), "{first}");
        (proc, port)
    };
    let (_right, port) = road("right.key", &key);
    for text in ["first", "second"] {
        let mut conn = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        conn.set_read_timeout(Some(Duration::from_secs(20)))
            .unwrap();
        conn.write_all(text.as_bytes()).unwrap();
        let mut back = vec![0u8; text.len()];
        conn.read_exact(&mut back)
            .expect("an echo through the service of the peer");
        assert_eq!(back, text.as_bytes());
    }
    let (_wrong, port) = road("wrong.key", "aes-256-gcm:another made-up password");
    let mut conn = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    conn.set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    conn.write_all(b"third").unwrap();
    let mut back = [0u8; 5];
    assert!(
        conn.read_exact(&mut back).is_err(),
        "another key gets no echo"
    );
    let _ = listener.0.kill();
    let mut err = String::new();
    listener
        .0
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut err)
        .unwrap();
    assert!(
        err.contains("a Shadowsocks stream is refused: a chunk of the peer did not open"),
        "{err}"
    );
    // The reach probe of each `ss://` network opens and ends a stream with
    // no byte: no line.
    assert!(!err.contains("before its salt"), "{err}");
    assert!(!err.contains(PASSWORD), "a password is in the log: {err}");
}

/// A bad setting: the word of `--service`, the variables, the words of
/// the line.
type Case<'a> = (&'a str, &'a [(&'a str, &'a str)], &'a str);

/// One rule for each bad setting of `--service ss`: exit 64 before the
/// listener listens, one line that names the fault, no password.
#[test]
fn a_bad_setting_of_the_service_exits_64_with_no_password() {
    let host = Host::new();
    let allow = host.path("allow");
    std::fs::write(&allow, host.fingerprint()).unwrap();
    let file = host.secret("k", &format!("aes-256-gcm:{PASSWORD}"));
    let sip022 = format!("2022-blake3-aes-256-gcm:{PASSWORD}");
    let cases: [Case; 4] = [
        ("ss", &[], KEY_FILE_VARIABLE),
        ("ss", &[(KEY_VARIABLE, &sip022)], "SIP004 alone"),
        (
            "ss",
            &[(KEY_VARIABLE, PASSWORD), (KEY_FILE_VARIABLE, &file)],
            "not both",
        ),
        (
            "ss=127.0.0.1:9",
            &[(KEY_FILE_VARIABLE, &file)],
            "takes no target",
        ),
    ];
    for (n, (service, vars, words)) in cases.into_iter().enumerate() {
        let sock = host.path(&format!("{n}.sock"));
        let mut cmd = host.command(&[
            "link",
            "listen",
            sock.to_str().unwrap(),
            "--allow",
            allow.to_str().unwrap(),
            "--service",
            service,
        ]);
        for (name, value) in vars {
            cmd.env(name, value);
        }
        let out: Output = cmd.output().unwrap();
        let all = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(out.status.code(), Some(64), "{service} {words}: {all}");
        assert!(all.contains(words), "{words}: {all}");
        assert!(!all.contains(PASSWORD), "{all}");
        assert!(!sock.exists(), "no listener: {all}");
    }
}
