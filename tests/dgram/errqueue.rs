//! T-149: the error queue of a UDP socket is empty, and a read of it does
//! not take a datagram.

use super::{expect_lines, Form, Proxy, C_HEAD};

const ERRQUEUE: &str = r#"
static const char *errqueue(int fd) {
    char data[64], control[256];
    struct iovec iov = { data, sizeof data };
    struct msghdr m;
    memset(&m, 0, sizeof m);
    m.msg_iov = &iov;
    m.msg_iovlen = 1;
    m.msg_control = control;
    m.msg_controllen = sizeof control;
    ssize_t n = recvmsg(fd, &m, MSG_ERRQUEUE);
    if (n >= 0) return "a message";
    return errno == EAGAIN ? "EAGAIN" : strerror(errno);
}

int main(void) {
    setvbuf(stdout, NULL, _IONBF, 0);
    alarm(20);
    int fd = socket(AF_INET, SOCK_DGRAM, 0);
    printf("empty: %s\n", errqueue(fd));
    struct sockaddr_in to = addr4("192.0.2.10", 7777);
    sendto(fd, "PING", 4, 0, (struct sockaddr *)&to, sizeof to);
    struct pollfd p = { .fd = fd, .events = POLLIN };
    if (poll(&p, 1, 5000) != 1) { printf("no echo\n"); return 1; }
    printf("waiting: %s\n", errqueue(fd));
    char b[64];
    ssize_t n = wait_datagram(fd, b, sizeof b);
    printf("data: %s\n", n > 0 ? b : "none");
    connect(fd, (struct sockaddr *)&to, sizeof to);
    printf("connected: %s\n", errqueue(fd));
    int icmp = socket(AF_INET, SOCK_DGRAM, IPPROTO_ICMP);
    printf("icmp: %s\n", errqueue(icmp));
    return 0;
}
"#;

#[test]
fn seccomp_udp_errqueue_read_is_eagain() {
    let proxy = Proxy::start(Form::Stream);
    let Some(out) = proxy.run(&format!("{C_HEAD}{ERRQUEUE}"), &[]) else {
        return;
    };
    expect_lines(
        &out,
        &[
            "empty: EAGAIN",
            "waiting: EAGAIN",
            "data: ECHO:PING",
            "connected: EAGAIN",
            "icmp: EAGAIN",
        ],
    );
}
