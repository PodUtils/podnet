//! T-143: a datagram that the program writes with `write(2)` on a
//! connected UDP socket leaves the pod.

use super::{expect_lines, Form, Proxy, C_HEAD};

/// `write` after `connect` (to the stand-in and to a resolver), and
/// `write` with no `connect`.
const WRITE: &str = r#"
int main(void) {
    alarm(20);
    struct sockaddr_in to = addr4("192.0.2.10", 7777);
    int fd = socket(AF_INET, SOCK_DGRAM, 0);
    if (connect(fd, (struct sockaddr *)&to, sizeof to) != 0) { perror("connect"); return 1; }
    if (write(fd, "PING", 4) != 4) { perror("write"); return 1; }
    char b[128];
    ssize_t n = wait_datagram(fd, b, sizeof b);
    printf("write: %s\n", n > 0 ? b : "no answer");
    struct iovec iov[2] = { { "PI", 2 }, { "NG", 2 } };
    if (writev(fd, iov, 2) != 4) { perror("writev"); return 1; }
    n = wait_datagram(fd, b, sizeof b);
    printf("writev: %s\n", n > 0 ? b : "no answer");
    close(fd);

    /* A DNS query for a.example (type A) with write. */
    unsigned char q[] = {0x12, 0x34, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0,
                         1, 'a', 7, 'e', 'x', 'a', 'm', 'p', 'l', 'e', 0, 0, 1, 0, 1};
    struct sockaddr_in resolver = addr4("192.0.2.53", 53);
    fd = socket(AF_INET, SOCK_DGRAM, 0);
    connect(fd, (struct sockaddr *)&resolver, sizeof resolver);
    if (write(fd, q, sizeof q) != sizeof q) { perror("write dns"); return 1; }
    unsigned char r[512];
    struct pollfd p = { .fd = fd, .events = POLLIN };
    n = poll(&p, 1, 5000) == 1 ? read(fd, r, sizeof r) : -1;
    printf("dns: %s\n", n > 16 && r[0] == 0x12 && r[1] == 0x34 && r[n - 4] == 198 ? "answer" : "none");
    close(fd);

    /* No connect: the datagram has no destination. */
    fd = socket(AF_INET, SOCK_DGRAM, 0);
    printf("unconnected write: %zd\n", write(fd, "PING", 4));
    struct pollfd u = { .fd = fd, .events = POLLIN };
    printf("unconnected: %s\n", poll(&u, 1, 500) == 0 ? "no answer" : "an answer");
    return 0;
}
"#;

#[test]
fn seccomp_udp_write_after_connect_is_relayed() {
    for form in [Form::Stream, Form::Associate] {
        let proxy = Proxy::start(form);
        let Some(out) = proxy.run(&format!("{C_HEAD}#include <sys/uio.h>\n{WRITE}"), &[]) else {
            return;
        };
        expect_lines(
            &out,
            &[
                "write: ECHO:PING",
                "writev: ECHO:PING",
                "dns: answer",
                "unconnected write: 4",
                "unconnected: no answer",
            ],
        );
    }
}

/// `sendto` with an address, and `send` and `sendmsg` after `connect`.
const SENDTO: &str = r#"
int main(void) {
    alarm(20);
    struct sockaddr_in to = addr4("192.0.2.10", 7777);
    int fd = socket(AF_INET, SOCK_DGRAM, 0);
    if (sendto(fd, "PING", 4, 0, (struct sockaddr *)&to, sizeof to) != 4) { perror("sendto"); return 1; }
    char b[128];
    struct sockaddr_in from;
    socklen_t fl = sizeof from;
    struct pollfd p = { .fd = fd, .events = POLLIN };
    ssize_t n = poll(&p, 1, 5000) == 1
        ? recvfrom(fd, b, sizeof b - 1, 0, (struct sockaddr *)&from, &fl) : -1;
    if (n >= 0) b[n] = 0;
    char ip[32] = "?";
    inet_ntop(AF_INET, &from.sin_addr, ip, sizeof ip);
    printf("sendto: %s from %s:%d\n", n > 0 ? b : "no answer", ip, ntohs(from.sin_port));
    connect(fd, (struct sockaddr *)&to, sizeof to);
    if (send(fd, "PING", 4, 0) != 4) { perror("send"); return 1; }
    n = wait_datagram(fd, b, sizeof b);
    printf("send: %s\n", n > 0 ? b : "no answer");
    struct iovec iov = { "PING", 4 };
    struct msghdr m;
    memset(&m, 0, sizeof m);
    m.msg_iov = &iov;
    m.msg_iovlen = 1;
    if (sendmsg(fd, &m, 0) != 4) { perror("sendmsg"); return 1; }
    n = wait_datagram(fd, b, sizeof b);
    printf("sendmsg: %s\n", n > 0 ? b : "no answer");
    return 0;
}
"#;

#[test]
fn seccomp_udp_sendto_still_works() {
    for form in [Form::Stream, Form::Associate] {
        let proxy = Proxy::start(form);
        let Some(out) = proxy.run(&format!("{C_HEAD}{SENDTO}"), &[]) else {
            return;
        };
        expect_lines(
            &out,
            &[
                "sendto: ECHO:PING from 192.0.2.10:7777",
                "send: ECHO:PING",
                "sendmsg: ECHO:PING",
            ],
        );
    }
}
