//! EGRESS-1 to EGRESS-5 and RUN-1 at run time: with
//! `{"default":"refuse","rules":[]}` installed, every path that would dial
//! returns the policy's reason and opens no connection.
//!
//! The function-level checks cover `egress::dial_addrs`, `cover`,
//! `defer::cache::HttpOrigin`, `networks::netbird::greeting`, the probes of
//! `agent`, and the far end of `e2e`. The command-level checks drive
//! `podnet tunnel --e2e` and the `link` services over a unix socket. Each
//! one holds a bound listener and asserts that it never accepts.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::Duration;

use pod_e2e::record::Suite;
use pod_identity::Identity;
use podnet::policy::{self, Policy};
use tokio::io::AsyncWriteExt;

const REFUSE: &str = r#"{"default":"refuse","rules":[]}"#;

/// Install the refuse policy of this test process. A second call has no
/// effect, and every call installs the same policy.
fn install_refuse() {
    policy::install(Policy::parse(REFUSE).unwrap());
}

/// The listener accepted nothing: no connection was attempted.
fn no_connection(listener: &TcpListener) {
    listener.set_nonblocking(true).unwrap();
    match listener.accept() {
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
        other => panic!("a connection was attempted: {other:?}"),
    }
}

fn listener() -> (TcpListener, u16) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    (listener, port)
}

fn policy_file(dir: &Path) -> PathBuf {
    let path = dir.join("refuse.json");
    std::fs::write(&path, REFUSE).unwrap();
    path
}

#[tokio::test]
async fn dial_addrs_refuses_before_a_connect() {
    install_refuse();
    let (listener, port) = listener();
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let err = podnet::egress::dial_addrs(&[addr], Duration::from_secs(1))
        .await
        .unwrap_err();
    let text = format!("{err:#}");
    assert!(text.contains("no rule matches"), "{text}");
    no_connection(&listener);
}

#[tokio::test]
async fn the_cover_origin_is_refused_before_a_connect() {
    install_refuse();
    let (listener, port) = listener();
    let err = podnet::cover::open_target(&format!("127.0.0.1:{port}"))
        .await
        .unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
    assert!(err.to_string().contains("no rule matches"), "{err}");
    no_connection(&listener);
}

#[tokio::test]
async fn a_deferred_origin_is_refused_before_a_connect() {
    use podnet::defer::Origin as _;
    install_refuse();
    let (listener, port) = listener();
    let mut origin = podnet::defer::HttpOrigin::new(format!("127.0.0.1:{port}"));
    let err = origin
        .fetch(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n")
        .await
        .unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
    assert!(err.to_string().contains("no rule matches"), "{err}");
    no_connection(&listener);
}

#[tokio::test]
async fn the_netbird_greeting_is_refused_before_a_connect() {
    install_refuse();
    let (listener, port) = listener();
    let err = podnet::networks::netbird_greeting(port).await.unwrap_err();
    assert!(err.contains("refused by the egress policy"), "{err}");
    assert!(err.contains("no rule matches"), "{err}");
    no_connection(&listener);
}

#[tokio::test]
async fn the_agent_probes_are_refused_with_their_reason() {
    use podnet::agent::Outcome;
    install_refuse();
    let report = podnet::agent::run(Duration::from_millis(50)).await;
    for (name, outcome) in [
        ("udp_dns", &report.udp_dns),
        ("ipv6", &report.ipv6),
        ("localhost", &report.localhost),
    ] {
        match outcome {
            Outcome::Failed(reason) => {
                assert!(reason.contains("no rule matches"), "{name}: {reason}")
            }
            other => panic!("{name} is {other:?}, not a refusal"),
        }
    }
}

/// A relay that copies both directions of two duplex streams.
async fn pump(a: tokio::io::DuplexStream, b: tokio::io::DuplexStream) {
    let (mut a, mut b) = (a, b);
    let _ = tokio::io::copy_bidirectional(&mut a, &mut b).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_e2e_far_end_refuses_the_destination_from_the_relay() {
    install_refuse();
    let (listener, port) = listener();
    let destination = ("127.0.0.1".to_string(), port);
    let client_id = Identity::from_seed(&[21; 32]);
    let server_id = Identity::from_seed(&[22; 32]);
    let config = |allowed| podnet::e2e::Config {
        relay: "relay-a".into(),
        pair: "pair-1".into(),
        suites: vec![Suite::ChaCha20Poly1305],
        host: destination.0.clone(),
        port: destination.1,
        service: "svc".into(),
        allowed,
        compat_destination: false,
    };
    let client_cfg = config(vec![server_id.public()]);
    let server_cfg = config(vec![client_id.public()]);

    let (client_data, relay_client_data) = tokio::io::duplex(64 * 1024);
    let (relay_server_data, server_data) = tokio::io::duplex(64 * 1024);
    let (client_control, relay_client_control) = tokio::io::duplex(1024);
    let (relay_server_control, server_control) = tokio::io::duplex(1024);
    tokio::spawn(pump(relay_client_data, relay_server_data));
    tokio::spawn(pump(relay_client_control, relay_server_control));
    let (app_client, mut app) = tokio::io::duplex(64 * 1024);

    let client = tokio::spawn(async move {
        let mut control = client_control;
        podnet::e2e::client_with(
            client_data,
            &mut control,
            app_client,
            &client_cfg,
            &client_id,
        )
        .await
    });
    let server = tokio::spawn(async move {
        let mut control = server_control;
        podnet::e2e::server_with(server_data, &mut control, &server_cfg, &server_id).await
    });

    let err = tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .expect("the far end finished")
        .expect("the far end task")
        .expect_err("the far end dials a refused destination");
    assert_eq!(podnet::exit::code_of(&err), podnet::exit::UNAVAILABLE);
    let text = format!("{err:#}");
    assert!(text.contains("no rule matches"), "{text}");
    client.abort();
    let _ = app.write_all(b"x").await;
    no_connection(&listener);
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// One install for a `link` process: a state directory and a run directory.
struct Host {
    dir: tempfile::TempDir,
    policy: PathBuf,
}

impl Host {
    fn new() -> Host {
        let dir = tempfile::tempdir().unwrap();
        let policy = policy_file(dir.path());
        Host { dir, policy }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_podnet"));
        cmd.args(args)
            .env("PODNET_STATE_DIR", self.path("state"))
            .env("PODNET_RUN_DIR", self.path("run"))
            .env("PODNET_POLICY", &self.policy)
            .env_remove("PODNET_LINK_CONTROL")
            .env_remove("RUST_LOG");
        cmd
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command(args).output().unwrap()
    }

    fn fingerprint(&self) -> String {
        let out = self.run(&["link", "status", "--create", "--json"]);
        let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
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

struct Pair {
    server: Host,
    client: Host,
    server_fp: String,
    _listener: Proc,
}

fn pair(services: &[&str]) -> Pair {
    let (server, client) = (Host::new(), Host::new());
    let (sfp, cfp) = (server.fingerprint(), client.fingerprint());
    let allow = server.path("allow");
    std::fs::write(&allow, &cfp).unwrap();
    let sock = server.path("l.sock");
    let mut args = vec![
        "link".to_string(),
        "listen".to_string(),
        sock.to_str().unwrap().to_string(),
        "--allow".to_string(),
        allow.to_str().unwrap().to_string(),
        "--json".to_string(),
    ];
    for service in services {
        args.push("--service".to_string());
        args.push((*service).to_string());
    }
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let (l, line) = start(&mut server.command(&refs));
    let _ready: serde_json::Value = serde_json::from_str(&line).unwrap();
    Pair {
        server,
        client,
        server_fp: sfp,
        _listener: l,
    }
}

impl Pair {
    /// Stop the listener and give what it wrote on stderr.
    fn listener_error(&mut self) -> String {
        let _ = self._listener.0.kill();
        let mut err = String::new();
        if let Some(mut pipe) = self._listener.0.stderr.take() {
            let _ = pipe.read_to_string(&mut err);
        }
        err
    }

    fn dial(&self, service: &str) -> Child {
        let sock = self.server.path("l.sock");
        self.client
            .command(&[
                "link",
                "dial",
                sock.to_str().unwrap(),
                "--peer",
                &self.server_fp,
                "--service",
                service,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    }
}

fn request(command: u8, atyp: u8, addr: &[u8], port: u16) -> Vec<u8> {
    let mut out = vec![5, 1, 0, 5, command, 0, atyp];
    out.extend_from_slice(addr);
    out.extend_from_slice(&port.to_be_bytes());
    out
}

#[test]
fn the_link_socks5_service_refuses_a_peer_target() {
    let (listener, port) = listener();
    let pair = pair(&["socks5"]);
    let mut dial = pair.dial("socks5");
    let bytes = request(1, 1, &[127, 0, 0, 1], port);
    dial.stdin.as_mut().unwrap().write_all(&bytes).unwrap();
    let out = dial.stdout.as_mut().unwrap();
    let mut method = [0u8; 2];
    let mut reply = [0u8; 10];
    out.read_exact(&mut method).unwrap();
    out.read_exact(&mut reply).unwrap();
    assert_eq!(method, [5, 0]);
    assert_eq!(reply[0], 5);
    assert_ne!(reply[1], 0, "a refused CONNECT must not say success");
    let _ = dial.stdin.take();
    let _ = dial.wait();
    no_connection(&listener);
}

#[test]
fn the_link_service_target_is_refused_before_connect() {
    let (listener, port) = listener();
    let target = format!("origin=127.0.0.1:{port}");
    let mut pair = pair(&[target.as_str()]);
    let mut dial = pair.dial("origin");
    let status = dial.wait().unwrap();
    assert!(!status.success(), "the dial must not succeed: {status:?}");
    let err = pair.listener_error();
    assert!(err.contains("no rule matches"), "{err}");
    no_connection(&listener);
}

#[test]
fn the_link_http_service_refuses_a_connect() {
    let (listener, port) = listener();
    let target = format!("echo=127.0.0.1:{port}");
    let pair = pair(&["http", &target]);
    let mut dial = pair.dial("http");
    dial.stdin
        .as_mut()
        .unwrap()
        .write_all(b"CONNECT echo:80 HTTP/1.1\r\nHost: echo:80\r\n\r\n")
        .unwrap();
    let mut answer = Vec::new();
    let _ = dial.stdout.as_mut().unwrap().read_to_end(&mut answer);
    let _ = dial.stdin.take();
    let _ = dial.wait();
    let answer = text(&answer);
    assert!(answer.contains("HTTP/1.1 502"), "{answer}");
    assert!(answer.contains("no rule matches"), "{answer}");
    no_connection(&listener);
}

#[test]
fn the_e2e_near_end_refuses_before_a_connect() {
    let dir = tempfile::tempdir().unwrap();
    let policy = policy_file(dir.path());
    let (_data, data_port) = listener();
    let (_control, control_port) = listener();
    let out = Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args([
            "tunnel",
            "--e2e",
            "--pair",
            "relay/pair",
            "--destination",
            "127.0.0.1:443",
            "--connect",
            &format!("127.0.0.1:{data_port}"),
            "--control",
            &format!("127.0.0.1:{control_port}"),
        ])
        .env("PODNET_POLICY", &policy)
        .env_remove("PODNET_LINK_CONTROL")
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(podnet::exit::UNAVAILABLE),
        "{}",
        text(&out.stderr)
    );
    let all = format!("{}{}", text(&out.stdout), text(&out.stderr));
    assert!(all.contains("no rule matches"), "{all}");
    no_connection(&_data);
    no_connection(&_control);
}
