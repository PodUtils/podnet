//! T-147: a connected UDP socket takes the datagrams of its peer only. The
//! stand-in answers a payload that starts with `OTHER` with a datagram
//! `NOISE` from 203.0.113.9:9, then with the echo.

use super::{expect_lines, Form, Proxy, C_HEAD};

const CONNECTED: &str = r#"
int main(void) {
    alarm(20);
    struct sockaddr_in to = addr4("192.0.2.10", 7777);
    int fd = socket(AF_INET, SOCK_DGRAM, 0);
    connect(fd, (struct sockaddr *)&to, sizeof to);
    if (write(fd, "OTHER", 5) != 5) { perror("write"); return 1; }
    char b[64];
    ssize_t n = wait_datagram(fd, b, sizeof b);
    printf("first: %s\n", n > 0 ? b : "none");
    struct pollfd p = { .fd = fd, .events = POLLIN };
    printf("more: %s\n", poll(&p, 1, 300) == 0 ? "none" : "a datagram");
    return 0;
}
"#;

#[test]
fn seccomp_connected_udp_drops_other_sources() {
    for form in [Form::Stream, Form::Associate] {
        let proxy = Proxy::start(form);
        let Some(out) = proxy.run(&format!("{C_HEAD}{CONNECTED}"), &[]) else {
            return;
        };
        expect_lines(&out, &["first: ECHO:OTHER", "more: none"]);
    }
}

const UNCONNECTED: &str = r#"
static void take(int fd) {
    char b[64];
    struct sockaddr_in from;
    socklen_t fl = sizeof from;
    struct pollfd p = { .fd = fd, .events = POLLIN };
    ssize_t n = poll(&p, 1, 5000) == 1
        ? recvfrom(fd, b, sizeof b - 1, 0, (struct sockaddr *)&from, &fl) : -1;
    if (n >= 0) b[n] = 0;
    char ip[32] = "?";
    inet_ntop(AF_INET, &from.sin_addr, ip, sizeof ip);
    printf("%s from %s:%d\n", n > 0 ? b : "none", ip, ntohs(from.sin_port));
}

int main(void) {
    alarm(20);
    struct sockaddr_in to = addr4("192.0.2.10", 7777);
    int fd = socket(AF_INET, SOCK_DGRAM, 0);
    sendto(fd, "OTHER", 5, 0, (struct sockaddr *)&to, sizeof to);
    take(fd);
    take(fd);
    return 0;
}
"#;

#[test]
fn seccomp_unconnected_udp_keeps_all_sources() {
    let proxy = Proxy::start(Form::Stream);
    let Some(out) = proxy.run(&format!("{C_HEAD}{UNCONNECTED}"), &[]) else {
        return;
    };
    expect_lines(
        &out,
        &[
            "NOISE from 203.0.113.9:9",
            "ECHO:OTHER from 192.0.2.10:7777",
        ],
    );
}
