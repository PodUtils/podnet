//! T-144: `bind` on a UDP socket is recorded; `getsockname` and
//! `getpeername` give the addresses of the kernel.

use super::{expect_lines, Form, Proxy, C_HEAD};

/// The C part of the tests: print the address of `getsockname` (`peer` 0)
/// or `getpeername`, or the errno.
const NAME: &str = r#"
static void show(const char *what, int fd, int peer) {
    struct sockaddr_storage s;
    memset(&s, 0, sizeof s);
    socklen_t l = sizeof s;
    int r = peer ? getpeername(fd, (struct sockaddr *)&s, &l)
                 : getsockname(fd, (struct sockaddr *)&s, &l);
    if (r != 0) {
        printf("%s: %s\n", what, errno == ENOTCONN ? "ENOTCONN" : strerror(errno));
        return;
    }
    char ip[64] = "?";
    int port = 0;
    if (s.ss_family == AF_INET) {
        struct sockaddr_in *a = (struct sockaddr_in *)&s;
        inet_ntop(AF_INET, &a->sin_addr, ip, sizeof ip);
        port = ntohs(a->sin_port);
    } else if (s.ss_family == AF_INET6) {
        struct sockaddr_in6 *a = (struct sockaddr_in6 *)&s;
        inet_ntop(AF_INET6, &a->sin6_addr, ip, sizeof ip);
        port = ntohs(a->sin6_port);
    }
    if (port >= 32768 && port <= 60999)
        printf("%s: family %d %s ephemeral\n", what, s.ss_family, ip);
    else
        printf("%s: family %d %s %d\n", what, s.ss_family, ip, port);
}

static int port_of(int fd) {
    struct sockaddr_in a;
    socklen_t l = sizeof a;
    getsockname(fd, (struct sockaddr *)&a, &l);
    return ntohs(a.sin_port);
}
"#;

const BIND: &str = r#"
int main(void) {
    alarm(20);
    int fd = socket(AF_INET, SOCK_DGRAM, 0);
    show("fresh", fd, 0);
    struct sockaddr_in any = addr4("0.0.0.0", 5353);
    printf("bind: %d\n", bind(fd, (struct sockaddr *)&any, sizeof any));
    show("bound", fd, 0);
    printf("second bind: %s\n",
           bind(fd, (struct sockaddr *)&any, sizeof any) < 0 && errno == EINVAL ? "EINVAL" : "other");
    int zero = socket(AF_INET, SOCK_DGRAM, 0);
    struct sockaddr_in zero_addr = addr4("127.0.0.1", 0);
    bind(zero, (struct sockaddr *)&zero_addr, sizeof zero_addr);
    show("port 0", zero, 0);
    printf("stable: %s\n", port_of(zero) == port_of(zero) ? "yes" : "no");
    int six = socket(AF_INET6, SOCK_DGRAM, 0);
    show("fresh6", six, 0);
    struct sockaddr_in6 a6;
    memset(&a6, 0, sizeof a6);
    a6.sin6_family = AF_INET6;
    a6.sin6_port = htons(5354);
    bind(six, (struct sockaddr *)&a6, sizeof a6);
    show("bound6", six, 0);
    return 0;
}
"#;

#[test]
fn seccomp_udp_bind_is_read_back() {
    let proxy = Proxy::start(Form::Stream);
    let Some(out) = proxy.run(&format!("{C_HEAD}{NAME}{BIND}"), &[]) else {
        return;
    };
    expect_lines(
        &out,
        &[
            "fresh: family 2 0.0.0.0 0",
            "bind: 0",
            "bound: family 2 0.0.0.0 5353",
            "second bind: EINVAL",
            "port 0: family 2 127.0.0.1 ephemeral",
            "stable: yes",
            "fresh6: family 10 :: 0",
            "bound6: family 10 :: 5354",
        ],
    );
}

const PEER: &str = r#"
int main(void) {
    alarm(20);
    int fd = socket(AF_INET, SOCK_DGRAM, 0);
    show("unconnected", fd, 1);
    struct sockaddr_in to = addr4("192.0.2.10", 7777);
    connect(fd, (struct sockaddr *)&to, sizeof to);
    show("connected", fd, 1);
    return 0;
}
"#;

#[test]
fn seccomp_udp_getpeername_is_enotconn_when_unconnected() {
    let proxy = Proxy::start(Form::Stream);
    let Some(out) = proxy.run(&format!("{C_HEAD}{NAME}{PEER}"), &[]) else {
        return;
    };
    expect_lines(
        &out,
        &[
            "unconnected: ENOTCONN",
            "connected: family 2 192.0.2.10 7777",
        ],
    );
}

const LOCAL: &str = r#"
int main(void) {
    alarm(20);
    int fd = socket(AF_INET, SOCK_DGRAM, 0);
    struct sockaddr_in to = addr4("192.0.2.10", 7777);
    connect(fd, (struct sockaddr *)&to, sizeof to);
    show("local", fd, 0);
    int sent = socket(AF_INET, SOCK_DGRAM, 0);
    sendto(sent, "PING", 4, 0, (struct sockaddr *)&to, sizeof to);
    show("after send", sent, 0);
    return 0;
}
"#;

#[test]
fn seccomp_udp_getsockname_of_a_connected_socket_is_local() {
    let proxy = Proxy::start(Form::Stream);
    let Some(out) = proxy.run(&format!("{C_HEAD}{NAME}{LOCAL}"), &[]) else {
        return;
    };
    expect_lines(
        &out,
        &[
            "local: family 2 0.0.0.0 ephemeral",
            "after send: family 2 0.0.0.0 ephemeral",
        ],
    );
}
