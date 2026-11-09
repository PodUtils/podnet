//! T-148: a UDP and an ICMP socket keep their IP-level options.

use super::{expect_lines, Form, Proxy, C_HEAD};

const TTL: &str = r#"
static void round_trip(const char *what, int fd, int level, int name, int value) {
    if (setsockopt(fd, level, name, &value, sizeof value) != 0) {
        printf("%s: set %s\n", what, strerror(errno));
        return;
    }
    int got = -1;
    socklen_t l = sizeof got;
    if (getsockopt(fd, level, name, &got, &l) != 0) {
        printf("%s: get %s\n", what, strerror(errno));
        return;
    }
    printf("%s: %d len %u\n", what, got, (unsigned)l);
}

int main(void) {
    alarm(20);
    int udp = socket(AF_INET, SOCK_DGRAM, 0);
    round_trip("udp ttl", udp, IPPROTO_IP, IP_TTL, 7);
    round_trip("udp tos", udp, IPPROTO_IP, IP_TOS, 16);
    int six = socket(AF_INET6, SOCK_DGRAM, 0);
    round_trip("udp6 hops", six, IPPROTO_IPV6, IPV6_UNICAST_HOPS, 9);
    int icmp = socket(AF_INET, SOCK_DGRAM, IPPROTO_ICMP);
    round_trip("icmp ttl", icmp, IPPROTO_IP, IP_TTL, 9);
    return 0;
}
"#;

#[test]
fn seccomp_udp_ip_ttl_round_trips() {
    let proxy = Proxy::start(Form::Stream);
    let Some(out) = proxy.run(&format!("{C_HEAD}{TTL}"), &[]) else {
        return;
    };
    expect_lines(
        &out,
        &[
            "udp ttl: 7 len 4",
            "udp tos: 16 len 4",
            "udp6 hops: 9 len 4",
        ],
    );
}

#[test]
fn seccomp_icmp_socket_keeps_its_options() {
    let proxy = Proxy::start(Form::Stream);
    let Some(out) = proxy.run(&format!("{C_HEAD}{TTL}"), &[]) else {
        return;
    };
    expect_lines(&out, &["icmp ttl: 9 len 4"]);
}
