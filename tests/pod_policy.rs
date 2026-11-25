//! GitHub #725: `podnet pod` applies the egress policy of the process
//! (`PODNET_POLICY`) to each flow of the program and of `--serve`, before
//! the rules of `-r`. A refused flow gets the answer of a refusal of a
//! rule: `ECONNREFUSED` (netns: a reset), SOCKS5 `0x02`, HTTP 403.
//!
//! The proxy and the door listen on unix sockets, so these tests need no
//! loopback.

#[path = "pod_harness/mod.rs"]
mod pod_harness;

use pod_harness::*;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// A policy that refuses the address of [`CLIENT`] and the name
/// `refused.example`, and lets each other destination through.
const POLICY: &str = r#"{"default":"direct","rules":[
  {"name":"no-ip","action":"refuse","host":"93.184.216.34"},
  {"name":"no-name","action":"refuse","host":"refused.example"}]}"#;

/// A policy that lets each destination through.
const OPEN: &str = r#"{"default":"direct","rules":[]}"#;

/// A temporary directory with a SOCKS5 proxy that greets and echoes on a
/// unix socket; each target goes to the receiver.
struct Lab {
    dir: tempfile::TempDir,
    proxy: PathBuf,
    targets: mpsc::Receiver<String>,
}

impl Lab {
    fn new() -> Lab {
        let dir = tempfile::tempdir().unwrap();
        let proxy = dir.path().join("socks.sock");
        let listener = UnixListener::bind(&proxy).unwrap();
        let (tx, targets) = mpsc::channel();
        std::thread::spawn(move || socks5_proxy(listener, tx, false));
        Lab {
            dir,
            proxy,
            targets,
        }
    }

    /// The policy file with `text`.
    fn policy(&self, text: &str) -> PathBuf {
        let path = self.dir.path().join("policy.json");
        std::fs::write(&path, text).unwrap();
        path
    }

    /// Run [`CLIENT`] in a pod of `backend` under the policy `text`.
    fn run(&self, backend: &str, text: &str) -> std::process::Output {
        let program = build_program(self.dir.path(), "client", CLIENT, false);
        pod_command()
            .env("PODNET_POLICY", self.policy(text))
            .env_remove("PODNET_NETNS_QUIET")
            .args(["--backend", backend, "-r", "domain:.example=direct", "-x"])
            .arg(format!("unix:{}", self.proxy.display()))
            .arg("--")
            .arg(&program)
            .output()
            .unwrap()
    }

    /// Start `pod --serve` on a unix socket under the policy `text`.
    fn serve(&self, text: &str) -> (Child, PathBuf) {
        let door = self.dir.path().join("door.sock");
        let mut child = pod_command()
            .env("PODNET_POLICY", self.policy(text))
            .arg("--serve")
            .arg(format!("unix:{}", door.display()))
            .arg("-x")
            .arg(format!("unix:{}", self.proxy.display()))
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(self.dir.path().join("serve.err")).unwrap())
            .spawn()
            .unwrap();
        let start = Instant::now();
        while !door.exists() {
            if start.elapsed() > Duration::from_secs(10) || child.try_wait().unwrap().is_some() {
                let _ = child.kill();
                panic!("no door: {}", self.serve_log());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        (child, door)
    }

    /// The stderr of `pod --serve`.
    fn serve_log(&self) -> String {
        std::fs::read_to_string(self.dir.path().join("serve.err")).unwrap_or_default()
    }
}

/// The SOCKS5 reply code of a CONNECT to `host:80` through `door`.
fn socks_reply(door: &Path, host: &str) -> u8 {
    let mut client = UnixStream::connect(door).unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    client.write_all(&[5, 1, 0]).unwrap();
    let mut method = [0u8; 2];
    client.read_exact(&mut method).unwrap();
    let mut request = vec![5, 1, 0, 3, host.len() as u8];
    request.extend_from_slice(host.as_bytes());
    request.extend_from_slice(&80u16.to_be_bytes());
    client.write_all(&request).unwrap();
    let mut reply = [0u8; 10];
    client.read_exact(&mut reply).unwrap();
    reply[1]
}

/// The status line of an HTTP CONNECT to `host:80` through `door`.
fn http_status(door: &Path, host: &str) -> String {
    let mut client = UnixStream::connect(door).unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    write!(
        client,
        "CONNECT {host}:80 HTTP/1.1\r\nHost: {host}:80\r\n\r\n"
    )
    .unwrap();
    let mut status = String::new();
    BufReader::new(client).read_line(&mut status).unwrap();
    status.trim_end().to_string()
}

/// A program in the pod: the policy refuses its destination on each
/// backend, also where a rule of `-r` would send it direct, and the line
/// names the rule of the policy. An open policy lets it through.
#[test]
fn the_policy_refuses_a_destination_of_the_program() {
    if !have_compiler() {
        eprintln!("skipping: no C compiler");
        return;
    }
    let backends = [
        ("seccomp", seccomp_available()),
        ("netns", netns_available()),
    ];
    let mut faults = Vec::new();
    for (backend, available) in backends {
        if !available {
            eprintln!("skipping {backend}: the backend cannot run on this host");
            continue;
        }
        let lab = Lab::new();
        let refused = lab.run(backend, POLICY);
        let stderr = String::from_utf8_lossy(&refused.stderr);
        if refused.status.code() != Some(1) || !stderr.contains("connect: Connection refused") {
            faults.push(format!("{backend}: the connect is not refused: {stderr}"));
        }
        if !stderr.contains("refused by PODNET_POLICY: the rule \"no-ip\" refuse") {
            faults.push(format!("{backend}: no line names the policy: {stderr}"));
        }
        if let Ok(target) = lab.targets.try_recv() {
            faults.push(format!("{backend}: the proxy got {target}"));
        }
        let open = lab.run(backend, OPEN);
        let stdout = String::from_utf8_lossy(&open.stdout);
        if !open.status.success() || !stdout.contains("HELLO-FROM-TARGET") {
            faults.push(format!(
                "{backend}: an open policy refuses: {stdout} {}",
                String::from_utf8_lossy(&open.stderr)
            ));
        }
    }
    assert!(faults.is_empty(), "{}", faults.join("\n"));
}

/// `pod --serve`: a destination that the policy refuses gets SOCKS5 0x02
/// and HTTP 403, and never reaches the upstream; another one goes through.
#[test]
fn the_policy_refuses_a_destination_of_the_door() {
    let lab = Lab::new();
    let (mut child, door) = lab.serve(POLICY);
    let rows = [
        ("refused.example", 0x02, " 403 "),
        ("allowed.example", 0x00, " 200 "),
    ];
    let mut faults = Vec::new();
    for (host, code, status) in rows {
        let got = socks_reply(&door, host);
        if got != code {
            faults.push(format!("{host}: SOCKS5 {got:#04x} (want {code:#04x})"));
        }
        let line = http_status(&door, host);
        if !line.contains(status) {
            faults.push(format!("{host}: HTTP {line:?} (want {status:?})"));
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    let seen: Vec<String> = lab.targets.try_iter().collect();
    if seen.iter().any(|t| t.starts_with("refused.example")) {
        faults.push(format!("the upstream got a refused target: {seen:?}"));
    }
    let log = lab.serve_log();
    if !log.contains("refused by PODNET_POLICY: the rule \"no-name\" refuse") {
        faults.push(format!("no line names the policy: {log}"));
    }
    assert!(faults.is_empty(), "{}", faults.join("\n"));
}
