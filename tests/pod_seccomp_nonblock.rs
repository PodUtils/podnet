//! T-223: a nonblocking `connect` under the seccomp backend gives
//! `EINPROGRESS` at once, as the kernel does. A second `connect` during the
//! dial gives `EALREADY`. `getsockopt(SO_ERROR)` waits for the end of the
//! dial and gives its result. Then the connection works.

use std::io::{Read, Write};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

fn pod() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_podnet"));
    command.arg("pod");
    command
}

fn seccomp_available() -> bool {
    let out = pod().args(["doctor", "--json"]).output().unwrap();
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    report["backend_seccomp"] == serde_json::Value::Bool(true)
}

/// A C compiler: `$CC`, then `cc`, `gcc` or `clang` on `PATH`.
fn compiler() -> Option<PathBuf> {
    let mut names: Vec<String> = std::env::var("CC").into_iter().collect();
    names.extend(["cc", "gcc", "clang"].map(String::from));
    let path = std::env::var_os("PATH")?;
    names.iter().find_map(|name| {
        std::env::split_paths(&path)
            .map(|dir| dir.join(name))
            .find(|p| p.is_file())
    })
}

fn build(dir: &Path, cc: &Path, source: &str) -> PathBuf {
    let src = dir.join("client.c");
    std::fs::write(&src, source).unwrap();
    let out = dir.join("client");
    let status = Command::new(cc)
        .args(["-O2", "-o"])
        .arg(&out)
        .arg(&src)
        .status()
        .unwrap();
    assert!(status.success(), "compiling the client failed");
    out
}

/// The stand-in answers a CONNECT to this port after [`SLOW`].
const SLOW_PORT: u16 = 8001;
const SLOW: Duration = Duration::from_millis(1500);

/// A SOCKS5 CONNECT stand-in on a unix socket. After the handshake it
/// sends the line `HELLO PORT`, then echoes one line of the client.
fn socks5(listener: UnixListener) {
    for conn in listener.incoming() {
        let Ok(mut c) = conn else { break };
        std::thread::spawn(move || {
            let mut b = [0u8; 2];
            if c.read_exact(&mut b).is_err() {
                return;
            }
            let mut m = vec![0u8; b[1] as usize];
            let _ = c.read_exact(&mut m);
            let _ = c.write_all(&[5, 0]);
            let mut h = [0u8; 4];
            if c.read_exact(&mut h).is_err() {
                return;
            }
            let rest = match h[3] {
                1 => 4 + 2,
                4 => 16 + 2,
                _ => {
                    let mut n = [0u8; 1];
                    let _ = c.read_exact(&mut n);
                    n[0] as usize + 2
                }
            };
            let mut r = vec![0u8; rest];
            let _ = c.read_exact(&mut r);
            let port = u16::from_be_bytes([r[rest - 2], r[rest - 1]]);
            if port == SLOW_PORT {
                std::thread::sleep(SLOW);
            }
            let _ = c.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
            let _ = c.write_all(format!("HELLO {port}\n").as_bytes());
            let mut line = [0u8; 5];
            if c.read_exact(&mut line).is_ok() {
                let _ = c.write_all(&line);
            }
        });
    }
}

/// The first socket: the slow target, with `SO_ERROR` after `poll`. The
/// second socket: the fast target, with a write before the dial ends and
/// no `SO_ERROR`. Each line gives a result and an errno, or a time in
/// milliseconds.
const CLIENT: &str = r#"
#include <arpa/inet.h>
#include <errno.h>
#include <netinet/in.h>
#include <poll.h>
#include <stdio.h>
#include <string.h>
#include <sys/socket.h>
#include <time.h>
#include <unistd.h>

static long now_ms(void) {
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec * 1000 + t.tv_nsec / 1000000;
}

static struct sockaddr_in target(int port) {
    struct sockaddr_in to;
    memset(&to, 0, sizeof to);
    to.sin_family = AF_INET;
    to.sin_port = htons(port);
    inet_pton(AF_INET, "93.184.216.34", &to.sin_addr);
    return to;
}

static void talk(int fd, const char *name) {
    char buf[64];
    int got = 0;
    struct pollfd p = { .fd = fd, .events = POLLIN };
    while (got < 22 && poll(&p, 1, 5000) == 1) {
        ssize_t n = read(fd, buf + got, sizeof buf - 1 - got);
        if (n <= 0) break;
        got += n;
        if (got == 11 && write(fd, "PING\n", 5) != 5) break;
    }
    buf[got] = 0;
    for (int i = 0; i < got; i++) if (buf[i] == '\n') buf[i] = '|';
    printf("%s_data %s\n", name, buf);
}

int main(void) {
    alarm(20);
    struct sockaddr_in slow = target(8001), fast = target(8002);
    int s = socket(AF_INET, SOCK_STREAM | SOCK_NONBLOCK, 0);
    long t0 = now_ms();
    int r = connect(s, (struct sockaddr *)&slow, sizeof slow);
    printf("connect %d %d %ld\n", r, r < 0 ? errno : 0, now_ms() - t0);
    r = connect(s, (struct sockaddr *)&slow, sizeof slow);
    printf("again %d %d\n", r, r < 0 ? errno : 0);
    struct sockaddr_in peer;
    socklen_t pl = sizeof peer;
    r = getpeername(s, (struct sockaddr *)&peer, &pl);
    printf("peer_early %d %d\n", r, r < 0 ? errno : 0);
    struct pollfd p = { .fd = s, .events = POLLOUT };
    poll(&p, 1, 5000);
    int err = -1;
    socklen_t len = sizeof err;
    t0 = now_ms();
    r = getsockopt(s, SOL_SOCKET, SO_ERROR, &err, &len);
    printf("so_error %d %d %u %ld\n", r, err, (unsigned)len, now_ms() - t0);
    pl = sizeof peer;
    r = getpeername(s, (struct sockaddr *)&peer, &pl);
    printf("peer %d %d\n", r, r == 0 ? ntohs(peer.sin_port) : errno);
    r = connect(s, (struct sockaddr *)&slow, sizeof slow);
    printf("connected %d %d\n", r, r < 0 ? errno : 0);
    talk(s, "slow");

    int f = socket(AF_INET, SOCK_STREAM | SOCK_NONBLOCK, 0);
    r = connect(f, (struct sockaddr *)&fast, sizeof fast);
    printf("fast %d %d\n", r, r < 0 ? errno : 0);
    talk(f, "fast");
    return 0;
}
"#;

/// The words of the line that starts with `name`.
fn line<'a>(stdout: &'a str, name: &str) -> Vec<&'a str> {
    let prefix = format!("{name} ");
    let line = stdout.lines().find(|l| l.starts_with(&prefix));
    let line = line.unwrap_or_else(|| panic!("no line {name}:\n{stdout}"));
    line.split(' ').skip(1).collect()
}

#[test]
fn seccomp_a_nonblocking_connect_gives_einprogress() {
    let Some(cc) = compiler() else {
        eprintln!("skipping: no C compiler");
        return;
    };
    if !seccomp_available() {
        eprintln!("skipping: no SECCOMP_RET_USER_NOTIF");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("socks.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    std::thread::spawn(move || socks5(listener));
    let program = build(dir.path(), &cc, CLIENT);
    let out = pod()
        .args(["--backend", "seccomp", "-v", "-x"])
        .arg(format!("unix:{}", sock.display()))
        .arg("--")
        .arg(&program)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(out.status.success(), "{stdout}\n{stderr}");
    let all = format!("{stdout}\n{stderr}");

    let connect = line(&stdout, "connect");
    assert_eq!(
        connect[..2],
        ["-1", &libc::EINPROGRESS.to_string()],
        "{all}"
    );
    let ms: u64 = connect[2].parse().unwrap();
    assert!(ms < 500, "the nonblocking connect took {ms} ms:\n{all}");
    let again = line(&stdout, "again");
    assert_eq!(again, ["-1", &libc::EALREADY.to_string()], "{all}");
    let early = line(&stdout, "peer_early");
    assert_eq!(early, ["-1", &libc::ENOTCONN.to_string()], "{all}");
    // SO_ERROR waits for the slow dial, then gives 0.
    let so_error = line(&stdout, "so_error");
    assert_eq!(so_error[..3], ["0", "0", "4"], "{all}");
    let ms: u64 = so_error[3].parse().unwrap();
    assert!(
        ms >= 800,
        "SO_ERROR did not wait for the dial ({ms} ms):\n{all}"
    );
    assert_eq!(line(&stdout, "peer"), ["0", "8001"], "{all}");
    assert_eq!(
        line(&stdout, "connected"),
        ["-1", &libc::EISCONN.to_string()],
        "{all}"
    );
    assert_eq!(line(&stdout, "slow_data"), ["HELLO", "8001|PING|"], "{all}");
    // T-065: the one SO_ERROR call takes the plain upstream.
    assert!(
        stderr.contains("the program holds the upstream socket"),
        "{all}"
    );
    // No call waits for the fast dial: the connection stays, with a relay.
    let fast = line(&stdout, "fast");
    assert_eq!(fast, ["-1", &libc::EINPROGRESS.to_string()], "{all}");
    assert_eq!(line(&stdout, "fast_data"), ["HELLO", "8002|PING|"], "{all}");
}
