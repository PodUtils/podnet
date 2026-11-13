//! Multi-hop forwarding with three podnet processes (T-287): A reaches a
//! service of C through B. B is `link listen --announce`; C is `link connect
//! --announce` to B and serves the service; A is `link dial --announce --node
//! C`. 1 MiB goes there and back with the same SHA-256. A spy between A and B
//! copies the bytes of that link: with `--seal` at A and C, the bytes hold no
//! plaintext of the stream; without it, they do (the control). When the link
//! from B to C closes, the stream through B fails and B lives on. The chain
//! runs over the unix carrier, and with the feature `tcp` over TCP on the
//! loopback.

#[path = "test_host/mod.rs"]
mod test_host;

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// A line of plaintext that the stream carries each KiB.
const MARK: &[u8] = b"podnet-mesh-plaintext-mark";

/// The bytes of the transfer: 1 MiB with the mark at each KiB.
fn payload() -> Vec<u8> {
    let mut out = Vec::with_capacity(1 << 20);
    let mut n: u64 = 0x9e37_79b9_7f4a_7c15;
    while out.len() < 1 << 20 {
        out.extend_from_slice(MARK);
        while out.len() % 1024 != 0 {
            n ^= n << 13;
            n ^= n >> 7;
            n ^= n << 17;
            out.push(n as u8);
        }
    }
    out
}

fn sha(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

struct Proc(Child);

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn host_command(dir: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_podnet"));
    cmd.args(args)
        .env("PODNET_STATE_DIR", dir.join("state"))
        .env("PODNET_RUN_DIR", dir.join("run"))
        .env_remove("PODNET_LINK_CONTROL")
        .env_remove("RUST_LOG");
    cmd
}

fn fingerprint(dir: &Path) -> String {
    let out = host_command(dir, &["link", "status", "--create", "--json"])
        .output()
        .unwrap();
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    doc["identity"]["fingerprint"].as_str().unwrap().to_string()
}

/// Start `args` in `dir` and read its first line of standard output (the
/// ready document of `--json`).
fn start(dir: &Path, args: &[&str]) -> (Proc, Value) {
    let mut child = host_command(dir, args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    out.read_line(&mut line).unwrap();
    // Drain the rest, so that a full pipe does not stop the process.
    std::thread::spawn(move || std::io::copy(&mut out, &mut std::io::sink()));
    let doc = serde_json::from_str(&line).unwrap_or_else(|e| panic!("{e}: {line:?}"));
    (Proc(child), doc)
}

fn routes(socket: &Path) -> Value {
    let token = std::fs::read_to_string(format!("{}.token", socket.display())).unwrap();
    let mut s = UnixStream::connect(socket).unwrap();
    let line = json!({"verb": "routes", "token": token.trim()}).to_string();
    s.write_all(line.as_bytes()).unwrap();
    s.write_all(b"\n").unwrap();
    let mut got = String::new();
    BufReader::new(&mut s).read_line(&mut got).unwrap();
    serde_json::from_str(&got).unwrap()
}

/// Whether the table of B has a direct route to the node `fp`.
fn has_direct(ctl: &Path, fp: &str) -> bool {
    let doc = routes(ctl);
    doc["routes"].as_array().is_some_and(|rows| {
        rows.iter()
            .any(|r| r["origin"] == fp && r["direct"] == true)
    })
}

fn until(what: &str, limit: Duration, mut ok: impl FnMut() -> bool) {
    let end = Instant::now() + limit;
    while !ok() {
        assert!(Instant::now() < end, "timeout: {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// An echo server on a unix socket: the service of C.
fn echo(path: &Path) {
    let listener = UnixListener::bind(path).unwrap();
    std::thread::spawn(move || {
        for conn in listener.incoming().flatten() {
            std::thread::spawn(move || {
                let (mut r, mut w) = (conn.try_clone().unwrap(), conn);
                let _ = std::io::copy(&mut r, &mut w);
                let _ = w.shutdown(std::net::Shutdown::Write);
            });
        }
    });
}

/// A spy on a unix socket in front of `to`: it copies each connection to
/// `to` and keeps a copy of each byte, both ways.
fn spy(at: &Path, to: PathBuf) -> Arc<Mutex<Vec<u8>>> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let listener = UnixListener::bind(at).unwrap();
    let kept = seen.clone();
    std::thread::spawn(move || {
        for near in listener.incoming().flatten() {
            let far = UnixStream::connect(&to).unwrap();
            for (mut from, mut into) in [
                (near.try_clone().unwrap(), far.try_clone().unwrap()),
                (far, near),
            ] {
                let kept = kept.clone();
                std::thread::spawn(move || {
                    let mut buf = [0u8; 16 << 10];
                    loop {
                        let n = match from.read(&mut buf) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => n,
                        };
                        kept.lock().unwrap().extend_from_slice(&buf[..n]);
                        if into.write_all(&buf[..n]).is_err() {
                            break;
                        }
                    }
                    let _ = into.shutdown(std::net::Shutdown::Write);
                });
            }
        }
    });
    seen
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// The three installs, B listening, C connected to B with the service
/// `echo`. `carrier` is `unix` or `tcp`; `seal` adds `--seal` at A and C.
struct Chain {
    dirs: [tempfile::TempDir; 3],
    fps: [String; 3],
    /// The dial address of B for A (through the spy for unix).
    to_b: String,
    carrier: &'static str,
    seal: bool,
    ctl: PathBuf,
    seen: Option<Arc<Mutex<Vec<u8>>>>,
    b: Proc,
    c: Proc,
}

impl Chain {
    fn up(carrier: &'static str, seal: bool) -> Chain {
        let dirs = [(); 3].map(|_| tempfile::tempdir().unwrap());
        let fps = [0, 1, 2].map(|i| fingerprint(dirs[i].path()));
        let [a, b, c] = &fps;
        let (bdir, cdir) = (dirs[1].path(), dirs[2].path());
        let allow_b = bdir.join("allow");
        std::fs::write(&allow_b, format!("{a}\n{c}\n")).unwrap();
        let allow_c = cdir.join("allow");
        std::fs::write(&allow_c, format!("{b}\n{a}\n")).unwrap();
        let ctl = bdir.join("c.sock");
        let at = match carrier {
            "unix" => bdir.join("l.sock").display().to_string(),
            _ => "127.0.0.1:0".to_string(),
        };
        let (bproc, doc) = start(
            bdir,
            &[
                "link",
                "listen",
                &at,
                "--carrier",
                carrier,
                "--allow",
                allow_b.to_str().unwrap(),
                "--announce",
                "--control",
                ctl.to_str().unwrap(),
                "--json",
            ],
        );
        let door = doc["address"].as_str().unwrap().to_string();
        let to = |door: &str| match carrier {
            "unix" => door.to_string(),
            _ => format!("{b}@{door}"),
        };
        let echo_at = cdir.join("echo.sock");
        echo(&echo_at);
        let service = format!("echo=unix:{}", echo_at.display());
        let to_c = to(&door);
        let mut args = vec![
            "link",
            "connect",
            &to_c,
            "--carrier",
            carrier,
            "--allow",
            allow_c.to_str().unwrap(),
            "--service",
            &service,
            "--announce",
            "--json",
        ];
        if seal {
            args.push("--seal");
        }
        let (cproc, _) = start(cdir, &args);
        until("B learns C", Duration::from_secs(20), || {
            has_direct(&ctl, c)
        });
        let (to_b, seen) = match carrier {
            "unix" => {
                let at = bdir.join("spy.sock");
                let seen = spy(&at, PathBuf::from(&door));
                (at.display().to_string(), Some(seen))
            }
            _ => (to(&door), None),
        };
        Chain {
            dirs,
            fps,
            to_b,
            carrier,
            seal,
            ctl,
            seen,
            b: bproc,
            c: cproc,
        }
    }

    /// A as `link dial --announce --node C --service echo`.
    fn dial(&self) -> Child {
        let mut args = vec![
            "link",
            "dial",
            &self.to_b,
            "--carrier",
            self.carrier,
            "--peer",
            &self.fps[1],
            "--announce",
            "--node",
            &self.fps[2],
            "--service",
            "echo",
        ];
        if self.seal {
            args.push("--seal");
        }
        host_command(self.dirs[0].path(), &args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    }

    /// 1 MiB from A to the echo of C and back: the SHA-256 of the bytes
    /// back, and the exit of A.
    fn round_trip(&self, data: &[u8]) -> ([u8; 32], usize, Option<i32>, String) {
        let mut a = self.dial();
        let mut stdin = a.stdin.take().unwrap();
        let up = data.to_vec();
        let writer = std::thread::spawn(move || {
            let _ = stdin.write_all(&up);
        });
        let mut back = Vec::new();
        a.stdout.take().unwrap().read_to_end(&mut back).unwrap();
        writer.join().unwrap();
        let out = a.wait_with_output().unwrap();
        (
            sha(&back),
            back.len(),
            out.status.code(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }
}

/// A reaches the echo of C through B: 1 MiB there and back, the same
/// SHA-256. Sealed, the link from A to B holds no plaintext of the stream.
fn a_reaches_c_through_b(carrier: &'static str, seal: bool) {
    let chain = Chain::up(carrier, seal);
    let data = payload();
    let (hash, n, code, err) = chain.round_trip(&data);
    assert_eq!((n, code), (data.len(), Some(0)), "A: {err}");
    assert_eq!(hash, sha(&data));
    if let Some(seen) = &chain.seen {
        let seen = seen.lock().unwrap();
        assert!(seen.len() > data.len(), "the spy saw {} bytes", seen.len());
        assert_eq!(
            contains(&seen, MARK),
            !seal,
            "plaintext on the link A to B, seal {seal}"
        );
    }
    // A has ended; B keeps its direct route to C.
    assert!(has_direct(&chain.ctl, &chain.fps[2]));
    drop(chain.c);
    drop(chain.b);
}

#[test]
fn a_reaches_a_service_of_c_through_b_with_three_processes_sealed() {
    a_reaches_c_through_b("unix", true);
}

#[test]
fn without_the_seal_the_link_a_to_b_shows_the_plaintext() {
    a_reaches_c_through_b("unix", false);
}

#[cfg(feature = "tcp")]
#[test]
fn a_reaches_a_service_of_c_through_b_over_tcp_on_the_loopback() {
    test_host::need_loopback!();
    a_reaches_c_through_b("tcp", true);
}

/// A stream from A to C goes through B. The link from B to C closes (C
/// ends): the stream fails at A, B lives on, and its table drops C.
#[test]
fn a_closed_middle_link_fails_the_stream_and_b_lives_on() {
    let mut chain = Chain::up("unix", false);
    let mut a = chain.dial();
    let mut stdin = a.stdin.take().unwrap();
    let mut out = a.stdout.take().unwrap();
    stdin.write_all(b"one\n").unwrap();
    let mut got = [0u8; 4];
    out.read_exact(&mut got).unwrap();
    assert_eq!(&got, b"one\n");
    let _ = chain.c.0.kill();
    let _ = chain.c.0.wait();
    let mut rest = Vec::new();
    let _ = out.read_to_end(&mut rest);
    assert!(rest.is_empty(), "{rest:?}");
    drop(stdin);
    let end = Instant::now() + Duration::from_secs(20);
    while a.try_wait().unwrap().is_none() {
        assert!(Instant::now() < end, "A did not end");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(chain.b.0.try_wait().unwrap().is_none(), "B ended");
    let c = chain.fps[2].clone();
    until("B drops C", Duration::from_secs(10), || {
        !has_direct(&chain.ctl, &c)
    });
    assert_eq!(routes(&chain.ctl)["ok"], true);
}

/// `link connect --announce-weight` needs `--announce`, and with
/// `--announce` the name `fwd` is no service: exit 64.
#[test]
fn the_announce_flags_of_link_connect_are_checked() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("l.sock");
    let sock = sock.to_str().unwrap();
    let base = ["link", "connect", sock, "--peer", "AAAAAAAA"];
    let run = |more: &[&str]| {
        let args: Vec<&str> = base.iter().chain(more).copied().collect();
        host_command(dir.path(), &args).output().unwrap()
    };
    let out = run(&["--announce-weight", "3"]);
    assert_eq!(out.status.code(), Some(64), "{out:?}");
    let out = run(&["--announce", "--service", "fwd=unix:/x"]);
    assert_eq!(out.status.code(), Some(64), "{out:?}");
}
