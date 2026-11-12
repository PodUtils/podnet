//! Test support (T-316): a stand-in of the quick-tunnel service and of the
//! Cloudflare edge, so that a test runs the quick tunnel with no call to
//! Cloudflare.
//!
//! * [`api`]: `POST /tunnel` on the loopback, with a 429 mode.
//! * [`edge`]: TLS on TCP, HTTP/2, `registerConnection`, visitor streams.
//! * [`front`]: an HTTP server that maps the `Host` of a visitor to a tunnel.
//! * [`ca`]: a test CA, made at run time. podnet trusts it through
//!   `SSL_CERT_FILE`, which podnet already reads. No hook is in the code of
//!   podnet.
//! * [`proxy`]: a CONNECT proxy that keeps the authorities.
//!
//! The stand-in tests podnet against the protocol as podnet reads it. It
//! does not show that the real edge accepts a change (`docs/research/podnet/
//! cf-edge-standins.md`, section 7). It has no QUIC leg yet.
//!
//! Include it in a test file with `mod standin;`. Each test that uses it
//! starts with `test_host::need_loopback!()`.

#![allow(dead_code, unused_imports)]

pub mod api;
pub mod ca;
pub mod edge;
pub mod front;
pub mod proxy;

use std::io::{BufRead as _, Read as _, Write as _};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub use api::Answer;
pub use edge::Reply;

/// The stand-ins, on the loopback, and the directory of a test.
pub struct Standin {
    runtime: Option<tokio::runtime::Runtime>,
    pub api: Arc<api::Api>,
    pub api_url: String,
    pub edge: Arc<edge::Edge>,
    pub edge_addr: SocketAddr,
    pub front_addr: SocketAddr,
    pub dir: tempfile::TempDir,
    pub ca_file: PathBuf,
}

impl Standin {
    pub fn start() -> Standin {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("a runtime for the stand-in");
        let dir = tempfile::tempdir().expect("a directory");
        let ca = ca::Ca::new();
        let ca_file = dir.path().join("standin-ca.pem");
        std::fs::write(&ca_file, &ca.pem).unwrap();
        let api = api::Api::new();
        let edge = edge::Edge::new(api.clone());
        let (api_addr, edge_addr, front_addr) = runtime.block_on(async {
            let api_addr = api::serve(api.clone()).await;
            let (listener, edge_addr) = edge::listen().await;
            let names = [podnet::cloudflare::h2_sni(), podnet::cloudflare::quic_sni()];
            edge::serve(edge.clone(), listener, ca.server_config(&names));
            let front_addr = front::serve(edge.clone()).await;
            (api_addr, edge_addr, front_addr)
        });
        Standin {
            runtime: Some(runtime),
            api,
            api_url: format!("http://{api_addr}"),
            edge,
            edge_addr,
            front_addr,
            dir,
            ca_file,
        }
    }

    /// Run a future of the stand-in to its end.
    pub fn block_on<F: std::future::Future>(&self, future: F) -> F::Output {
        self.runtime.as_ref().unwrap().block_on(future)
    }

    /// A CONNECT proxy on the loopback.
    pub fn proxy(&self) -> (SocketAddr, proxy::Log) {
        self.block_on(proxy::serve())
    }

    /// The directory of `podnet ps` and `podnet stop` for this test.
    pub fn run_dir(&self) -> PathBuf {
        self.dir.path().join("run")
    }

    /// `podnet ARGS` against the stand-ins: the service, the edge, the test
    /// CA, and no proxy from the environment of the test.
    pub fn podnet(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_podnet"));
        command.args(args);
        for name in [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "NO_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
            "no_proxy",
            "PODNET_PROXY",
            "RUST_LOG",
        ] {
            command.env_remove(name);
        }
        command
            .env("SSL_CERT_FILE", &self.ca_file)
            .env("PODNET_EDGE", self.edge_addr.to_string())
            .env("PODNET_RUN_DIR", self.run_dir())
            .env("HOME", self.dir.path())
            .env("PODNET_UDP", "off")
            .stdin(Stdio::null());
        command
    }

    /// `podnet tunnel` with the flags of the HTTP/2 leg against the
    /// stand-ins: `ARGS` follow. No verify: the public name is not in the
    /// DNS.
    pub fn tunnel(&self, args: &[&str]) -> Command {
        let mut all = vec![
            "tunnel",
            "--service-url",
            &self.api_url,
            "--protocol",
            "http2",
            "--transport",
            "direct",
            "--no-verify",
        ];
        all.extend_from_slice(args);
        self.podnet(&all)
    }

    /// A request from a visitor, through the front. `host` is the `Host`.
    /// Gives the status, the headers and the body.
    pub fn visit(
        &self,
        host: &str,
        path: &str,
        headers: &[(&str, &str)],
    ) -> (u16, Vec<(String, String)>, String) {
        let mut stream = std::net::TcpStream::connect(self.front_addr).expect("the front");
        stream
            .set_read_timeout(Some(Duration::from_secs(20)))
            .unwrap();
        let mut request = format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n");
        for (name, value) in headers {
            request.push_str(&format!("{name}: {value}\r\n"));
        }
        request.push_str("\r\n");
        stream.write_all(request.as_bytes()).unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).unwrap();
        let text = String::from_utf8_lossy(&raw).to_string();
        let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
        let mut lines = head.lines();
        let status = lines
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let headers = lines
            .filter_map(|l| l.split_once(':'))
            .map(|(n, v)| (n.trim().to_string(), v.trim().to_string()))
            .collect();
        (status, headers, body.to_string())
    }
}

impl Drop for Standin {
    fn drop(&mut self) {
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}

/// A podnet process whose standard output and error are read by threads.
pub struct Process {
    pub child: Child,
    lines: Receiver<String>,
    pub out: Arc<std::sync::Mutex<Vec<String>>>,
    pub err: Arc<std::sync::Mutex<String>>,
}

impl Process {
    pub fn spawn(mut command: Command) -> Process {
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = command.spawn().expect("spawn podnet");
        let (tx, lines) = std::sync::mpsc::channel();
        let out: Arc<std::sync::Mutex<Vec<String>>> = Arc::default();
        let kept = out.clone();
        let stdout = child.stdout.take().unwrap();
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                kept.lock().unwrap().push(line.clone());
                let _ = tx.send(line);
            }
        });
        let err: Arc<std::sync::Mutex<String>> = Arc::default();
        let kept = err.clone();
        let stderr = child.stderr.take().unwrap();
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(stderr).lines() {
                let Ok(line) = line else { break };
                let mut text = kept.lock().unwrap();
                text.push_str(&line);
                text.push('\n');
            }
        });
        Process {
            child,
            lines,
            out,
            err,
        }
    }

    /// The next line of standard output that `matches`, within `within`.
    pub fn line_where(&self, within: Duration, matches: impl Fn(&str) -> bool) -> Option<String> {
        let end = Instant::now() + within;
        loop {
            let left = end.checked_duration_since(Instant::now())?;
            let line = self.lines.recv_timeout(left).ok()?;
            if matches(&line) {
                return Some(line);
            }
        }
    }

    /// The JSON line with `"event":"ready"` (`--json`).
    pub fn ready(&self) -> serde_json::Value {
        let line = self
            .line_where(Duration::from_secs(60), |l| {
                l.contains(r#""event":"ready""#)
            })
            .unwrap_or_else(|| panic!("no ready line; stderr:\n{}", self.stderr()));
        serde_json::from_str(&line).expect("the ready line is JSON")
    }

    pub fn stderr(&self) -> String {
        self.err.lock().unwrap().clone()
    }

    pub fn signal(&self, signal: libc::c_int) {
        // SAFETY: kill() to our own child.
        unsafe { libc::kill(self.child.id() as i32, signal) };
    }

    /// Wait for the end of the process.
    pub fn wait(&mut self, within: Duration) -> Option<ExitStatus> {
        let end = Instant::now() + within;
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                // Let the thread of standard error finish.
                std::thread::sleep(Duration::from_millis(100));
                return Some(status);
            }
            if Instant::now() >= end {
                return None;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// An origin on the loopback: it answers each request with `body`, and it
/// keeps the head of each request. Gives the port.
pub fn origin(body: &'static str) -> (u16, Arc<std::sync::Mutex<Vec<String>>>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen: Arc<std::sync::Mutex<Vec<String>>> = Arc::default();
    let kept = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let kept = kept.clone();
            std::thread::spawn(move || {
                let mut stream = stream;
                let mut head = Vec::new();
                let mut byte = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") {
                    match stream.read(&mut byte) {
                        Ok(1) => head.push(byte[0]),
                        _ => return,
                    }
                }
                kept.lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&head).to_string());
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nX-Origin: standin\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            });
        }
    });
    (port, seen)
}
