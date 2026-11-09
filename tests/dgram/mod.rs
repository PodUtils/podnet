//! The kit of the UDP tests of the seccomp backend: a C compiler, a pod
//! run, and a SOCKS5 stand-in that echoes each datagram. Each module below
//! holds the tests of one record entry.

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

mod relay;

pub fn pod() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_podnet"));
    command.arg("pod");
    command
}

pub fn seccomp_available() -> bool {
    let out = pod().args(["doctor", "--json"]).output().unwrap();
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    report["backend_seccomp"] == serde_json::Value::Bool(true)
}

/// A C compiler: `$CC`, then `cc`, `gcc` or `clang` on `PATH`.
pub fn compiler() -> Option<PathBuf> {
    let mut names: Vec<String> = std::env::var("CC").into_iter().collect();
    names.extend(["cc", "gcc", "clang"].map(String::from));
    let path = std::env::var_os("PATH")?;
    names.iter().find_map(|name| {
        std::env::split_paths(&path)
            .map(|dir| dir.join(name))
            .find(|p| p.is_file())
    })
}

pub fn build(dir: &Path, cc: &Path, source: &str) -> PathBuf {
    let src = dir.join("client.c");
    std::fs::write(&src, source).unwrap();
    let out = dir.join("client");
    let status = Command::new(cc)
        .arg("-O2")
        .arg("-o")
        .arg(&out)
        .arg(&src)
        .status()
        .unwrap();
    assert!(status.success(), "compiling the client failed");
    out
}

pub fn expect_lines(out: &Output, want: &[&str]) {
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stdout}\n{stderr}");
    for want in want {
        assert!(
            stdout.lines().any(|l| l == *want),
            "{want:?} in:\n{stdout}\n{stderr}"
        );
    }
}

/// The UDP form of the stand-in.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Form {
    /// UDP in the stream (command `0x83`): each datagram is a frame on the
    /// control connection.
    Stream,
    /// `UDP ASSOCIATE`: the stand-in refuses `0x83`, and it echoes on a UDP
    /// socket of 127.0.0.1.
    Associate,
}

/// A SOCKS5 stand-in on a unix socket. Each datagram gets the answer
/// `ECHO:` and the payload, from the address that the datagram went to. A
/// payload that starts with `OTHER` first gets the datagram `NOISE` from
/// 203.0.113.9:9.
pub struct Proxy {
    pub dir: tempfile::TempDir,
    pub sock: PathBuf,
    /// The live UDP associations.
    pub live: Arc<AtomicUsize>,
}

impl Proxy {
    pub fn start(form: Form) -> Proxy {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("socks.sock");
        let listener = UnixListener::bind(&sock).unwrap();
        let live = Arc::new(AtomicUsize::new(0));
        let count = live.clone();
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                let Ok(c) = conn else { break };
                let count = count.clone();
                std::thread::spawn(move || serve(c, form, count));
            }
        });
        Proxy { dir, sock, live }
    }

    /// Compile `source` and run it in the pod, with `args`. `None` when the
    /// host has no C compiler or no seccomp user notification.
    pub fn run(&self, source: &str, args: &[&str]) -> Option<Output> {
        let Some(cc) = compiler() else {
            eprintln!("skipping: no C compiler");
            return None;
        };
        if !seccomp_available() {
            eprintln!("skipping: no SECCOMP_RET_USER_NOTIF");
            return None;
        }
        let program = build(self.dir.path(), &cc, source);
        let out = pod()
            .args(["--backend", "seccomp"])
            .arg("-x")
            .arg(format!("unix:{}", self.sock.display()))
            .arg("--")
            .arg(&program)
            .args(args)
            .output()
            .unwrap();
        Some(out)
    }
}

/// Read a greeting and a request. Return the command.
fn request(c: &mut UnixStream) -> Option<u8> {
    let mut hdr = [0u8; 2];
    c.read_exact(&mut hdr).ok()?;
    let mut methods = vec![0u8; hdr[1] as usize];
    c.read_exact(&mut methods).ok()?;
    c.write_all(&[5, 0]).ok()?;
    let mut req = [0u8; 4];
    c.read_exact(&mut req).ok()?;
    let rest = match req[3] {
        1 => 4 + 2,
        4 => 16 + 2,
        3 => {
            let mut n = [0u8; 1];
            c.read_exact(&mut n).ok()?;
            n[0] as usize + 2
        }
        _ => return None,
    };
    let mut r = vec![0u8; rest];
    c.read_exact(&mut r).ok()?;
    Some(req[1])
}

/// The answers to one SOCKS5 UDP datagram (header and payload).
fn answers(datagram: &[u8]) -> Vec<Vec<u8>> {
    let off = match datagram.get(3) {
        Some(1) => 10,
        Some(4) => 22,
        Some(3) => match datagram.get(4) {
            Some(n) => 4 + 1 + *n as usize + 2,
            None => return Vec::new(),
        },
        _ => return Vec::new(),
    };
    if datagram.len() < off {
        return Vec::new();
    }
    let mut out = Vec::new();
    if datagram[off..].starts_with(b"OTHER") {
        out.push(vec![
            0, 0, 0, 1, 203, 0, 113, 9, 0, 9, b'N', b'O', b'I', b'S', b'E',
        ]);
    }
    let mut echo = datagram[..off].to_vec();
    echo.extend_from_slice(b"ECHO:");
    echo.extend_from_slice(&datagram[off..]);
    out.push(echo);
    out
}

fn serve(mut c: UnixStream, form: Form, live: Arc<AtomicUsize>) {
    let Some(command) = request(&mut c) else {
        return;
    };
    match (command, form) {
        (0x83, Form::Stream) => {
            let _ = c.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
            live.fetch_add(1, Ordering::SeqCst);
            loop {
                let mut len = [0u8; 2];
                if c.read_exact(&mut len).is_err() {
                    break;
                }
                let mut datagram = vec![0u8; u16::from_be_bytes(len) as usize];
                if c.read_exact(&mut datagram).is_err() {
                    break;
                }
                for answer in answers(&datagram) {
                    let mut frame = (answer.len() as u16).to_be_bytes().to_vec();
                    frame.extend_from_slice(&answer);
                    let _ = c.write_all(&frame);
                }
            }
            live.fetch_sub(1, Ordering::SeqCst);
        }
        (3, Form::Associate) => {
            let udp = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
            udp.set_read_timeout(Some(Duration::from_millis(100)))
                .unwrap();
            let port = udp.local_addr().unwrap().port().to_be_bytes();
            let _ = c.write_all(&[5, 0, 0, 1, 127, 0, 0, 1, port[0], port[1]]);
            live.fetch_add(1, Ordering::SeqCst);
            let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let stop = done.clone();
            let echo = std::thread::spawn(move || {
                let mut buf = vec![0u8; 65536];
                while !stop.load(Ordering::SeqCst) {
                    if let Ok((n, from)) = udp.recv_from(&mut buf) {
                        for answer in answers(&buf[..n]) {
                            let _ = udp.send_to(&answer, from);
                        }
                    }
                }
            });
            let mut sink = [0u8; 64];
            while matches!(c.read(&mut sink), Ok(n) if n > 0) {}
            done.store(true, Ordering::SeqCst);
            let _ = echo.join();
            live.fetch_sub(1, Ordering::SeqCst);
        }
        _ => {
            let _ = c.write_all(&[5, 7, 0, 1, 0, 0, 0, 0, 0, 0]);
        }
    }
}

/// The common part of the C programs of these tests.
pub const C_HEAD: &str = r#"
#define _GNU_SOURCE
#include <arpa/inet.h>
#include <dirent.h>
#include <errno.h>
#include <netinet/in.h>
#include <poll.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/wait.h>
#include <unistd.h>

/* The count of the descriptors of the supervisor (the parent). */
static int supervisor_fds(void) {
    char path[64];
    snprintf(path, sizeof path, "/proc/%d/fd", (int)getppid());
    DIR *d = opendir(path);
    if (!d) return -1;
    int n = 0;
    struct dirent *e;
    while ((e = readdir(d)))
        if (e->d_name[0] != '.') n++;
    closedir(d);
    return n;
}

static struct sockaddr_in addr4(const char *ip, int port) {
    struct sockaddr_in a;
    memset(&a, 0, sizeof a);
    a.sin_family = AF_INET;
    a.sin_port = htons(port);
    inet_pton(AF_INET, ip, &a.sin_addr);
    return a;
}

/* Wait up to 5 s for a datagram on fd. Put it in buf, with a 0 after it. */
static ssize_t wait_datagram(int fd, char *buf, size_t len) {
    struct pollfd p = { .fd = fd, .events = POLLIN };
    if (poll(&p, 1, 5000) != 1) return -1;
    ssize_t n = recv(fd, buf, len - 1, 0);
    if (n >= 0) buf[n] = 0;
    return n;
}
"#;
