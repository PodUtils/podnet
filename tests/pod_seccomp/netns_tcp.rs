//! The TCP router of the netns backend on the stack of `pod-netstack` (T-249):
//! a burst of connects to one port, and more than 256 destination ports at
//! one time. The download of 48 MiB is in `tcp.rs`. Each test skips when
//! the namespace is not available (no compiler, or `unshare` refused).

use super::*;

/// T-249: 48 non-blocking connects to one port at the same time. The
/// stack adds a listen socket for each SYN, so a burst gets no RST and
/// `connect()` never fails.
pub(super) const BURST_CLIENT: &str = r#"
#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include <arpa/inet.h>
#include <sys/socket.h>
#define N 48
int main(void) {
    alarm(30);
    int fds[N];
    for (int i = 0; i < N; i++) {
        fds[i] = socket(AF_INET, SOCK_STREAM | SOCK_NONBLOCK, 0);
        struct sockaddr_in a; memset(&a, 0, sizeof a);
        a.sin_family = AF_INET; a.sin_port = htons(80);
        inet_pton(AF_INET, "93.184.216.34", &a.sin_addr);
        if (connect(fds[i], (struct sockaddr *)&a, sizeof a) < 0 && errno != EINPROGRESS) {
            perror("connect"); return 1;
        }
    }
    int ok = 0, hello = 0;
    for (int i = 0; i < N; i++) {
        struct pollfd p = { fds[i], POLLOUT, 0 };
        if (poll(&p, 1, 20000) != 1) continue;
        int err = 0; socklen_t len = sizeof err;
        getsockopt(fds[i], SOL_SOCKET, SO_ERROR, &err, &len);
        if (err == 0) ok++;
    }
    for (int i = 0; i < N; i++) {
        struct pollfd p = { fds[i], POLLIN, 0 };
        char b[64] = {0};
        if (poll(&p, 1, 20000) == 1 && read(fds[i], b, sizeof b - 1) > 0 && strstr(b, "HELLO")) hello++;
    }
    printf("CONNECTED:%d HELLO:%d\n", ok, hello);
    return 0;
}
"#;

#[test]
fn netns_backend_takes_a_burst_of_connects_to_one_port() {
    if !have_compiler() || !netns_available() {
        eprintln!("skipping: no compiler or no netns backend");
        return;
    }
    let dir = std::env::temp_dir().join(format!("podnet-netns-burst-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let sock = dir.join("socks.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    let (tx, _rx) = mpsc::channel();
    std::thread::spawn(move || socks5_proxy(listener, tx, false));
    let program = build_program(&dir, "burst-client", BURST_CLIENT, false);
    let stdout = run_on("netns", &[&sock], &program);
    assert!(stdout.contains("CONNECTED:48 HELLO:48"), "{stdout:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 300 connections to 300 different ports at the same time, each held open
/// until the program has them all. The router has no cap on the ports. Then
/// the program closes them and uses six of the ports again.
const MANY_PORTS_CLIENT: &str = r#"
#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include <arpa/inet.h>
#include <sys/socket.h>
#define N 300
int main(void) {
    alarm(60);
    int fds[N], hello = 0;
    for (int i = 0; i < N; i++) {
        fds[i] = socket(AF_INET, SOCK_STREAM, 0);
        struct sockaddr_in a; memset(&a, 0, sizeof a);
        a.sin_family = AF_INET; a.sin_port = htons(2000 + i);
        inet_pton(AF_INET, "93.184.216.34", &a.sin_addr);
        if (connect(fds[i], (struct sockaddr *)&a, sizeof a) < 0) { perror("connect"); continue; }
    }
    for (int i = 0; i < N; i++) {
        char b[64] = {0};
        if (read(fds[i], b, sizeof b - 1) > 0 && strstr(b, "HELLO")) hello++;
    }
    /* Close all, so that each listener closes, then use the ports again. */
    for (int i = 0; i < N; i++) close(fds[i]);
    usleep(1500000);
    int again = 0;
    for (int i = 0; i < N; i += 50) {
        int s = socket(AF_INET, SOCK_STREAM, 0);
        struct sockaddr_in a; memset(&a, 0, sizeof a);
        a.sin_family = AF_INET; a.sin_port = htons(2000 + i);
        inet_pton(AF_INET, "93.184.216.34", &a.sin_addr);
        char b[64] = {0};
        if (connect(s, (struct sockaddr *)&a, sizeof a) == 0
            && read(s, b, sizeof b - 1) > 0 && strstr(b, "HELLO")) again++;
        close(s);
    }
    printf("HELLO:%d AGAIN:%d\n", hello, again);
    return 0;
}
"#;

#[test]
fn netns_backend_has_no_cap_on_destination_ports() {
    if !have_compiler() || !netns_available() {
        eprintln!("skipping: no compiler or no netns backend");
        return;
    }
    let dir = std::env::temp_dir().join(format!("podnet-netns-ports-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let sock = dir.join("socks.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    let (tx, _rx) = mpsc::channel();
    std::thread::spawn(move || socks5_proxy(listener, tx, false));
    let program = build_program(&dir, "ports-client", MANY_PORTS_CLIENT, false);
    let stdout = run_on("netns", &[&sock], &program);
    assert!(stdout.contains("HELLO:300 AGAIN:6"), "{stdout:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
