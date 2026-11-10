//! End-to-end tests of the seccomp backend of `podnet pod`.
//!
//! A SOCKS5 proxy listens on a unix socket (the only bind a sealed host
//! permits). A compiled C program calls `socket(AF_INET)`/`connect(AF_INET)`
//! normally; the filter stops both and the supervisor hands it a socketpair
//! and dials the proxy. No `LD_PRELOAD` is involved, so a **static** build is
//! exercised too — that is the whole point of the backend.
//!
//! Three tests: a single proxy, two chained hops, and UDP relayed through
//! `UDP ASSOCIATE`. If the host does not offer `SECCOMP_RET_USER_NOTIF` they
//! report that and pass, as the other shim tests do without a compiler.

#[path = "../test_host/mod.rs"]
mod test_host;

use std::io::{Read, Write};
use std::net::{Ipv4Addr, UdpSocket};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// `podnet pod`: the CLI of the pod is a command of the root binary.
fn pod_command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_podnet"));
    command.arg("pod");
    command
}

/// Find a C compiler: `$CC`, then `cc`, `gcc` or `clang` on `PATH`.
fn compiler() -> Option<PathBuf> {
    let mut names: Vec<String> = Vec::new();
    if let Ok(cc) = std::env::var("CC") {
        names.push(cc);
    }
    names.extend(["cc", "gcc", "clang"].map(String::from));
    let path = std::env::var_os("PATH")?;
    for name in names {
        let candidate = PathBuf::from(&name);
        if candidate.is_absolute() && candidate.is_file() {
            return Some(candidate);
        }
        for dir in std::env::split_paths(&path) {
            let full = dir.join(&name);
            if full.is_file() {
                return Some(full);
            }
        }
    }
    None
}

fn have_compiler() -> bool {
    compiler().is_some()
}

fn build_program(dir: &std::path::Path, name: &str, source: &str, static_link: bool) -> PathBuf {
    let source_path = dir.join(format!("{name}.c"));
    std::fs::write(&source_path, source).unwrap();
    let output = dir.join(name);
    let cc = compiler().unwrap();
    let mut cmd = Command::new(cc);
    cmd.arg("-O2");
    if static_link {
        cmd.arg("-static");
    }
    let status = cmd
        .arg("-o")
        .arg(&output)
        .arg(&source_path)
        .status()
        .unwrap();
    assert!(status.success(), "compiling {name} failed");
    output
}

/// Read a SOCKS5 request after the method handshake; returns (cmd, host, port).
fn read_request(c: &mut UnixStream) -> Option<(u8, String, u16)> {
    let mut hdr = [0u8; 2];
    c.read_exact(&mut hdr).ok()?;
    let mut methods = vec![0u8; hdr[1] as usize];
    c.read_exact(&mut methods).ok()?;
    c.write_all(&[5, 0]).ok()?;
    let mut req = [0u8; 4];
    c.read_exact(&mut req).ok()?;
    let host = match req[3] {
        1 => {
            let mut a = [0u8; 4];
            c.read_exact(&mut a).ok()?;
            Ipv4Addr::from(a).to_string()
        }
        3 => {
            let mut l = [0u8; 1];
            c.read_exact(&mut l).ok()?;
            let mut h = vec![0u8; l[0] as usize];
            c.read_exact(&mut h).ok()?;
            String::from_utf8_lossy(&h).to_string()
        }
        4 => {
            let mut a = [0u8; 16];
            c.read_exact(&mut a).ok()?;
            std::net::Ipv6Addr::from(a).to_string()
        }
        _ => return None,
    };
    let mut p = [0u8; 2];
    c.read_exact(&mut p).ok()?;
    Some((req[1], host, u16::from_be_bytes(p)))
}

/// A SOCKS5 CONNECT proxy on a unix socket. With `hop`, a target named
/// `unix:<path>` is forwarded to that socket; otherwise it greets and echoes.
fn socks5_proxy(listener: UnixListener, targets: mpsc::Sender<String>, hop: bool) {
    for conn in listener.incoming() {
        let mut c = match conn {
            Ok(c) => c,
            Err(_) => break,
        };
        let targets = targets.clone();
        let _ = std::thread::spawn(move || {
            let Some((cmd, host, port)) = read_request(&mut c) else {
                return;
            };
            if cmd != 1 {
                return;
            }
            let _ = targets.send(format!("{host}:{port}"));
            if hop && host.starts_with("unix:") {
                let Ok(mut up) = UnixStream::connect(&host[5..]) else {
                    return;
                };
                let _ = c.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
                let mut up2 = up.try_clone().unwrap();
                let mut c2 = c.try_clone().unwrap();
                std::thread::spawn(move || {
                    let _ = std::io::copy(&mut up2, &mut c2);
                    let _ = c2.shutdown(std::net::Shutdown::Write);
                });
                let _ = std::io::copy(&mut c, &mut up);
            } else {
                let _ = c.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
                let _ = c.write_all(b"HELLO-FROM-TARGET\n");
                let mut buf = [0u8; 4096];
                while let Ok(n) = c.read(&mut buf) {
                    if n == 0 || c.write_all(&buf[..n]).is_err() {
                        break;
                    }
                }
            }
        });
    }
}

fn seccomp_available() -> bool {
    let out = pod_command()
        .args(["doctor", "--json"])
        .output()
        .expect("run podnet pod doctor");
    let report: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("doctor --json prints JSON");
    report["backend_seccomp"] == serde_json::Value::Bool(true)
}

fn netns_available() -> bool {
    let out = pod_command()
        .args(["doctor", "--json"])
        .output()
        .expect("run podnet pod doctor");
    let report: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("doctor --json prints JSON");
    report["backend_netns"] == serde_json::Value::Bool(true)
}

fn run(_dir: &std::path::Path, proxies: &[&std::path::Path], program: &std::path::Path) -> String {
    run_on("seccomp", proxies, program)
}

fn run_on(backend: &str, proxies: &[&std::path::Path], program: &std::path::Path) -> String {
    let mut cmd = pod_command();
    cmd.args(["--backend", backend, "-v"]);
    for p in proxies {
        cmd.arg("-x").arg(format!("unix:{}", p.display()));
    }
    let output = cmd.arg("--").arg(program).output().expect("run podnet pod");
    assert!(
        output.status.success(),
        "podnet pod failed: stderr={:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).to_string()
}

const CLIENT: &str = r#"
#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include <arpa/inet.h>
#include <sys/socket.h>
int main(void) {
    alarm(10);
    int s = socket(AF_INET, SOCK_STREAM, 0);
    struct sockaddr_in a; memset(&a, 0, sizeof a);
    a.sin_family = AF_INET; a.sin_port = htons(80);
    inet_pton(AF_INET, "93.184.216.34", &a.sin_addr);
    if (connect(s, (struct sockaddr *)&a, sizeof a) < 0) { perror("connect"); return 1; }
    write(s, "PING\n", 5);
    char b[64] = {0};
    int n = read(s, b, sizeof b - 1);
    printf("READ:%d:%s", n, b);
    return 0;
}
"#;

const SERVER: &str = r#"
#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include <arpa/inet.h>
#include <sys/socket.h>
int main(void) {
    alarm(10);
    int s = socket(AF_INET, SOCK_STREAM, 0);
    struct sockaddr_in a; memset(&a, 0, sizeof a);
    a.sin_family = AF_INET; a.sin_port = htons(8080);
    a.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    if (bind(s, (struct sockaddr *)&a, sizeof a) < 0) { perror("bind"); return 1; }
    if (listen(s, 4) < 0) { perror("listen"); return 1; }
    int c = accept(s, NULL, NULL);
    if (c < 0) { perror("accept"); return 1; }
    char b[64] = {0};
    int n = read(c, b, sizeof b - 1);
    if (n <= 0) return 1;
    write(c, "PONG\n", 5);
    return 0;
}
"#;

const DNS_CLIENT: &str = r#"
#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include <netdb.h>
#include <arpa/inet.h>
#include <sys/socket.h>
int main(void) {
    alarm(10);
    struct addrinfo hints, *res = NULL;
    memset(&hints, 0, sizeof hints);
    hints.ai_family = AF_INET; hints.ai_socktype = SOCK_STREAM;
    int r = getaddrinfo("example.com", "80", &hints, &res);
    if (r != 0) { printf("GAI:%d\n", r); return 1; }
    char ip[64] = {0};
    inet_ntop(AF_INET, &((struct sockaddr_in *)res->ai_addr)->sin_addr, ip, sizeof ip);
    int s = socket(AF_INET, SOCK_STREAM, 0);
    if (connect(s, res->ai_addr, res->ai_addrlen) < 0) { perror("connect"); return 1; }
    char b[64] = {0};
    int n = read(s, b, sizeof b - 1);
    printf("DNS:%s:%d:%s", ip, n, b);
    return 0;
}
"#;

mod dns;
mod listen;
mod rules;
mod tcp;
mod udp;
