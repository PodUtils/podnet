//! T-457 (PodUtils/podrunner#142): a program that waits with epoll. It adds
//! the nonblocking socket to its epoll set before the connect, as .NET,
//! libuv and Go do. The pod must not replace that socket after the connect:
//! epoll then never reports the new socket, and the program waits forever.

use super::*;

/// Prints `EPOLL:LINE` when epoll reports the connect and the first line of
/// the target, else `EPOLL-TIMEOUT:STEP`.
const EPOLL_CLIENT: &str = r#"
#include <stdio.h>
#include <string.h>
#include <errno.h>
#include <fcntl.h>
#include <unistd.h>
#include <arpa/inet.h>
#include <sys/epoll.h>
#include <sys/socket.h>
int main(void) {
    alarm(20);
    int s = socket(AF_INET, SOCK_STREAM | SOCK_NONBLOCK, 0);
    int ep = epoll_create1(0);
    struct epoll_event ev = { .events = EPOLLIN | EPOLLOUT, .data.u64 = 7 };
    if (epoll_ctl(ep, EPOLL_CTL_ADD, s, &ev) < 0) { perror("epoll_ctl"); return 1; }
    struct sockaddr_in a; memset(&a, 0, sizeof a);
    a.sin_family = AF_INET; a.sin_port = htons(8001);
    inet_pton(AF_INET, "203.0.113.9", &a.sin_addr);
    if (connect(s, (struct sockaddr *)&a, sizeof a) < 0 && errno != EINPROGRESS) { perror("connect"); return 1; }
    struct epoll_event got;
    if (epoll_wait(ep, &got, 1, 5000) != 1 || !(got.events & EPOLLOUT)) { printf("EPOLL-TIMEOUT:connect\n"); return 0; }
    int err = 0; socklen_t len = sizeof err;
    getsockopt(s, SOL_SOCKET, SO_ERROR, &err, &len);
    if (err) { printf("EPOLL-ERROR:%d\n", err); return 0; }
    ev.events = EPOLLIN;
    epoll_ctl(ep, EPOLL_CTL_MOD, s, &ev);
    char b[128] = {0}; int n = 0;
    for (int i = 0; i < 2 && n <= 0; i++) {
        if (epoll_wait(ep, &got, 1, 5000) != 1) { printf("EPOLL-TIMEOUT:read\n"); return 0; }
        n = read(s, b, sizeof b - 1);
    }
    char *nl = strchr(b, '\n'); if (nl) *nl = 0;
    printf("EPOLL:%s\n", b);
    return 0;
}
"#;

#[test]
fn an_epoll_program_sees_its_socket_after_the_connect() {
    test_host::need_loopback!();
    if !have_compiler() || !seccomp_available() {
        eprintln!("skipping: no compiler or no SECCOMP_RET_USER_NOTIF");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("socks.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    let (tx, _rx) = mpsc::channel();
    std::thread::spawn(move || socks5_proxy(listener, tx, false));
    let program = build_program(dir.path(), "epoll", EPOLL_CLIENT, false);
    let out = pod_command()
        .args(["--backend", "seccomp", "-v", "-x"])
        .arg(format!("unix:{}", sock.display()))
        .arg("--")
        .arg(&program)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stdout}\n{stderr}");
    assert_eq!(
        stdout.trim(),
        "EPOLL:HELLO-FROM-TARGET",
        "{stdout}\n{stderr}"
    );
    assert!(
        !stderr.contains("the program holds the upstream socket"),
        "{stderr}"
    );
}
