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
use std::process::Stdio;
use std::sync::mpsc;
use std::time::Duration;

#[path = "../pod_harness/mod.rs"]
mod pod_harness;
pub(crate) use pod_harness::*;

fn run(_dir: &std::path::Path, proxies: &[&std::path::Path], program: &std::path::Path) -> String {
    run_on("seccomp", proxies, program)
}

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
mod netns_udp;
mod outlive;
mod rules;
mod tcp;
mod udp;
