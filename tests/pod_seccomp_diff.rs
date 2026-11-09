//! T-155: a differential check of the socket calls. One C driver makes a
//! fixed series of socket calls and prints one line for each: the result or
//! the errno, and the form of each address. The test runs the driver on the
//! host kernel and in a seccomp pod, and compares the lines. A difference
//! fails the test, except the differences of `ALLOWED`: each one names the
//! entry of the record that holds it. An item of `ALLOWED` that no longer
//! differs fails the test too, so that the list only gets shorter.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;

mod test_host;

/// The known differences: (the label of the line, the entry, the gap).
const ALLOWED: &[(&str, &str, &str)] = &[];

const DRIVER: &str = r#"
#define _GNU_SOURCE
#include <arpa/inet.h>
#include <errno.h>
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <poll.h>
#include <stdio.h>
#include <string.h>
#include <sys/socket.h>
#include <unistd.h>

static const char *errno_name(int e) {
    switch (e) {
    case EINVAL: return "EINVAL";
    case ENOTCONN: return "ENOTCONN";
    case EISCONN: return "EISCONN";
    case ECONNREFUSED: return "ECONNREFUSED";
    case EINPROGRESS: return "EINPROGRESS";
    case EAGAIN: return "EAGAIN";
    case EDESTADDRREQ: return "EDESTADDRREQ";
    case EOPNOTSUPP: return "EOPNOTSUPP";
    case EAFNOSUPPORT: return "EAFNOSUPPORT";
    case ENOPROTOOPT: return "ENOPROTOOPT";
    case EBADF: return "EBADF";
    case EFAULT: return "EFAULT";
    case EIO: return "EIO";
    case EPERM: return "EPERM";
    case EACCES: return "EACCES";
    }
    static char other[32];
    snprintf(other, sizeof other, "errno %d", e);
    return other;
}

static void result(const char *what, long r) {
    if (r >= 0) printf("%s: %ld\n", what, r);
    else printf("%s: %s\n", what, errno_name(errno));
}

/* The family, the address and the class of the port: the port numbers
   differ from run to run. */
static void name(const char *what, int fd, int peer) {
    struct sockaddr_storage s;
    memset(&s, 0, sizeof s);
    socklen_t l = sizeof s;
    int r = peer ? getpeername(fd, (struct sockaddr *)&s, &l)
                 : getsockname(fd, (struct sockaddr *)&s, &l);
    if (r < 0) { result(what, r); return; }
    struct sockaddr_in *a = (struct sockaddr_in *)&s;
    char ip[32] = "?";
    inet_ntop(AF_INET, &a->sin_addr, ip, sizeof ip);
    int port = ntohs(a->sin_port);
    printf("%s: family %d %s %s len %u\n", what, s.ss_family, ip,
           port == 0 ? "0" : port >= 32768 ? "ephemeral" : "fixed", (unsigned)l);
}

static void intopt(const char *what, int fd, int level, int opt) {
    int v = -1;
    socklen_t l = sizeof v;
    int r = getsockopt(fd, level, opt, &v, &l);
    if (r < 0) result(what, r);
    else printf("%s: %d len %u\n", what, v, (unsigned)l);
}

static struct sockaddr_in addr4(const char *ip, int port) {
    struct sockaddr_in a;
    memset(&a, 0, sizeof a);
    a.sin_family = AF_INET;
    a.sin_port = htons(port);
    inet_pton(AF_INET, ip, &a.sin_addr);
    return a;
}

#define SA(a) (struct sockaddr *)&(a), sizeof(a)

int main(void) {
    setvbuf(stdout, NULL, _IONBF, 0);
    alarm(20);
    int one = 1, ttl = 7;
    struct sockaddr_in lo0 = addr4("127.0.0.1", 0), closed = addr4("127.0.0.1", 1);
    struct sockaddr_in any0 = addr4("0.0.0.0", 0), peer = addr4("192.0.2.10", 7777);

    int t = socket(AF_INET, SOCK_STREAM, 0);
    result("tcp socket", t >= 0 ? 0 : -1);
    intopt("tcp SO_TYPE", t, SOL_SOCKET, SO_TYPE);
    intopt("tcp SO_ERROR", t, SOL_SOCKET, SO_ERROR);
    name("tcp getsockname fresh", t, 0);
    name("tcp getpeername fresh", t, 1);
    result("tcp setsockopt TCP_NODELAY", setsockopt(t, IPPROTO_TCP, TCP_NODELAY, &one, sizeof one));
    intopt("tcp TCP_NODELAY", t, IPPROTO_TCP, TCP_NODELAY);
    result("tcp shutdown unconnected", shutdown(t, SHUT_RDWR));
    result("tcp bind", bind(t, SA(lo0)));
    name("tcp getsockname bound", t, 0);
    result("tcp bind again", bind(t, SA(lo0)));
    close(t);
    t = socket(AF_INET, SOCK_STREAM, 0);
    result("tcp listen unbound", listen(t, 4));
    name("tcp getsockname listening", t, 0);
    close(t);
    t = socket(AF_INET, SOCK_STREAM, 0);
    result("tcp connect closed", connect(t, SA(closed)));
    intopt("tcp SO_ERROR after refused", t, SOL_SOCKET, SO_ERROR);
    close(t);
    t = socket(AF_INET, SOCK_STREAM | SOCK_NONBLOCK, 0);
    result("tcp nonblocking connect", connect(t, SA(closed)));
    struct pollfd p = { .fd = t, .events = POLLOUT };
    poll(&p, 1, 5000);
    intopt("tcp nonblocking SO_ERROR", t, SOL_SOCKET, SO_ERROR);
    close(t);

    int u = socket(AF_INET, SOCK_DGRAM, 0);
    intopt("udp SO_TYPE", u, SOL_SOCKET, SO_TYPE);
    name("udp getsockname fresh", u, 0);
    name("udp getpeername fresh", u, 1);
    result("udp setsockopt IP_TTL", setsockopt(u, IPPROTO_IP, IP_TTL, &ttl, sizeof ttl));
    intopt("udp IP_TTL", u, IPPROTO_IP, IP_TTL);
    result("udp bind", bind(u, SA(any0)));
    name("udp getsockname bound", u, 0);
    result("udp bind again", bind(u, SA(any0)));
    result("udp send unconnected", send(u, "x", 1, 0));
    result("udp connect", connect(u, SA(peer)));
    name("udp getpeername connected", u, 1);
    char b[16];
    struct iovec iov = { b, sizeof b };
    struct msghdr m;
    memset(&m, 0, sizeof m);
    m.msg_iov = &iov;
    m.msg_iovlen = 1;
    result("udp recvmsg MSG_ERRQUEUE", recvmsg(u, &m, MSG_ERRQUEUE));
    result("udp recv MSG_DONTWAIT", recv(u, b, sizeof b, MSG_DONTWAIT));
    struct sockaddr unspec;
    memset(&unspec, 0, sizeof unspec);
    unspec.sa_family = AF_UNSPEC;
    result("udp connect AF_UNSPEC", connect(u, &unspec, sizeof unspec));
    name("udp getpeername after AF_UNSPEC", u, 1);
    result("udp send after AF_UNSPEC", send(u, "x", 1, 0));
    close(u);
    printf("end: 0\n");
    return 0;
}
"#;

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

fn seccomp_available() -> bool {
    let out = Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(["pod", "doctor", "--json"])
        .output()
        .unwrap();
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    report["backend_seccomp"] == serde_json::Value::Bool(true)
}

/// The lines of one run, by label.
fn lines(mut command: Command) -> BTreeMap<String, String> {
    let out = command.output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success() && stdout.contains("end: 0"),
        "{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    stdout
        .lines()
        .filter_map(|l| l.split_once(": "))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn seccomp_errnos_match_the_host() {
    // The host side needs a loopback and a free bind(AF_INET): with no
    // network the kernel gives other answers, and the comparison is void.
    test_host::need_loopback!();
    let Some(cc) = compiler() else {
        eprintln!("skipping: no C compiler");
        return;
    };
    if !seccomp_available() {
        eprintln!("skipping: no SECCOMP_RET_USER_NOTIF");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("driver.c");
    let driver = dir.path().join("driver");
    std::fs::write(&src, DRIVER).unwrap();
    let built = Command::new(&cc)
        .arg("-O2")
        .arg("-o")
        .arg(&driver)
        .arg(&src)
        .status()
        .unwrap();
    assert!(built.success(), "compiling the driver failed");
    let host = lines(Command::new(&driver));
    let mut pod = Command::new(env!("CARGO_BIN_EXE_podnet"));
    // No call of the driver goes to the proxy: a TCP connect to loopback
    // goes to the host, and a UDP connect sends nothing.
    pod.args([
        "pod",
        "--backend",
        "seccomp",
        "-x",
        "socks5://127.0.0.1:9",
        "--",
    ])
    .arg(&driver);
    let pod = lines(pod);
    assert_eq!(
        host.keys().collect::<Vec<_>>(),
        pod.keys().collect::<Vec<_>>(),
        "the same calls"
    );
    let mut faults = Vec::new();
    for (label, want) in &host {
        let got = &pod[label];
        let allowed = ALLOWED.iter().find(|(l, _, _)| l == label);
        match (want == got, allowed) {
            (true, None) => {}
            (false, None) => faults.push(format!("{label}: host {want:?}, pod {got:?}")),
            (true, Some((_, entry, _))) => faults.push(format!(
                "{label}: the same now; remove it from ALLOWED ({entry})"
            )),
            (false, Some(_)) => {}
        }
    }
    for (label, _, _) in ALLOWED {
        if !host.contains_key(*label) {
            faults.push(format!("{label}: in ALLOWED, but no such line"));
        }
    }
    assert!(faults.is_empty(), "{}", faults.join("\n"));
}
