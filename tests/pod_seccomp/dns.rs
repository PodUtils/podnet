//! DNS: names, IPv6 names and the resolution of an unspecified family.

use super::*;

#[test]
fn netns_backend_resolves_names_and_carries_parallel_flows() {
    if !have_compiler() || !netns_available() {
        eprintln!("skipping: no compiler or no netns backend");
        return;
    }
    let dir = std::env::temp_dir().join(format!("podnet-netns-nsdns-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let sock = dir.join("socks.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || socks5_proxy(listener, tx, false));

    let program = build_program(&dir, "parallel", PARALLEL_DNS_CLIENT, false);
    let stdout = run_on("netns", &[&sock], &program);
    assert_eq!(stdout.matches("HELLO-FROM-TARGET").count(), 3, "{stdout:?}");
    let mut targets: Vec<String> = (0..3).map(|_| rx.recv().unwrap()).collect();
    targets.sort();
    assert_eq!(targets, ["a.example:80", "b.example:80", "c.example:80"]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Resolve three names, open three connections to port 80 at the same time,
/// then read from each.
pub(super) const PARALLEL_DNS_CLIENT: &str = r#"
#include <netdb.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include <sys/socket.h>
int main(void) {
    alarm(10);
    const char *names[3] = {"a.example", "b.example", "c.example"};
    int fds[3];
    for (int i = 0; i < 3; i++) {
        struct addrinfo hints, *res;
        memset(&hints, 0, sizeof hints);
        hints.ai_family = AF_INET;
        hints.ai_socktype = SOCK_STREAM;
        if (getaddrinfo(names[i], "80", &hints, &res) != 0) { printf("DNS-FAIL\n"); return 1; }
        fds[i] = socket(AF_INET, SOCK_STREAM, 0);
        if (connect(fds[i], res->ai_addr, res->ai_addrlen) < 0) { perror("connect"); return 1; }
        freeaddrinfo(res);
    }
    for (int i = 0; i < 3; i++) {
        char b[64] = {0};
        int n = read(fds[i], b, sizeof b - 1);
        printf("%d:%s", n, n > 0 ? b : "");
    }
    return 0;
}
"#;

pub(super) const IPV6_CLIENT: &str = r#"
#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include <arpa/inet.h>
#include <sys/socket.h>
int main(void) {
    alarm(10);
    int s = socket(AF_INET6, SOCK_STREAM, 0);
    struct sockaddr_in6 a; memset(&a, 0, sizeof a);
    a.sin6_family = AF_INET6; a.sin6_port = htons(80);
    inet_pton(AF_INET6, "2001:db8::1", &a.sin6_addr);
    if (connect(s, (struct sockaddr *)&a, sizeof a) < 0) { perror("connect"); return 1; }
    write(s, "PING\n", 5);
    char b[64] = {0};
    int n = read(s, b, sizeof b - 1);
    printf("READ6:%d:%s", n, b);
    return 0;
}
"#;

pub(super) const DNS6_CLIENT: &str = r#"
#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include <netdb.h>
#include <arpa/inet.h>
#include <sys/socket.h>
int main(int argc, char **argv) {
    alarm(10);
    struct addrinfo hints, *res = NULL;
    memset(&hints, 0, sizeof hints);
    hints.ai_family = AF_INET6; hints.ai_socktype = SOCK_STREAM;
    int r = getaddrinfo("example.com", "80", &hints, &res);
    if (r != 0) { printf("GAI6:%d\n", r); return 1; }
    char ip[64] = {0};
    inet_ntop(AF_INET6, &((struct sockaddr_in6 *)res->ai_addr)->sin6_addr, ip, sizeof ip);
    int s = socket(AF_INET6, SOCK_STREAM, 0);
    if (connect(s, res->ai_addr, res->ai_addrlen) < 0) { perror("connect"); return 1; }
    char b[64] = {0};
    int n = read(s, b, sizeof b - 1);
    printf("DNS6:%s:%d:%s", ip, n, b);
    return 0;
}
"#;

/// Issue 99: AAAA gets a fake ULA address, and connect maps it to the name.
#[test]
fn seccomp_backend_resolves_ipv6_by_name() {
    test_host::need_loopback!();
    if !have_compiler() || !seccomp_available() {
        eprintln!("skipping: no compiler or no SECCOMP_RET_USER_NOTIF");
        return;
    }
    let dir = std::env::temp_dir().join(format!("podnet-netns-dns6-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let sock = dir.join("socks.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || socks5_proxy(listener, tx, false));

    let program = build_program(&dir, "client-dns6", DNS6_CLIENT, false);
    let stdout = run(&dir, &[&sock], &program);
    assert!(
        stdout.contains("DNS6:fd63:6672:7300::c612:0:18:HELLO-FROM-TARGET"),
        "{stdout:?}"
    );
    // The proxy gets the name, not the fake IPv6 address.
    assert_eq!(rx.recv().unwrap(), "example.com:80");

    // A name pinned to an IPv4 address has no AAAA: a clean EAI_NODATA, not
    // EAI_SYSTEM. After a NODATA answer, glibc tries the search domains of
    // the host (example.com.SEARCH), and the fake DNS answers each name: an
    // empty LOCALDOMAIN removes the search list of a CI runner.
    let output = pod_command()
        .args(["--backend", "seccomp", "--map", "example.com=1.2.3.4", "-x"])
        .arg(format!("unix:{}", sock.display()))
        .arg("--")
        .arg(&program)
        .env("LOCALDOMAIN", "")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("GAI6:-5") || stdout.contains("GAI6:-2"),
        "{stdout:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

pub(super) const UNSPEC_CLIENT: &str = r#"
#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include <netdb.h>
#include <sys/socket.h>
int main(void) {
    alarm(20);
    struct addrinfo hints, *res = NULL;
    memset(&hints, 0, sizeof hints);
    hints.ai_family = AF_UNSPEC; hints.ai_socktype = SOCK_STREAM;
    int r = getaddrinfo("example.com", "80", &hints, &res);
    printf("GAI:%d\n", r);
    if (r != 0) return 1;
    int s = socket(res->ai_family, SOCK_STREAM, 0);
    if (connect(s, res->ai_addr, res->ai_addrlen) < 0) { perror("connect"); return 1; }
    char b[64] = {0};
    int n = read(s, b, sizeof b - 1);
    printf("READ:%d:%s", n, b);
    return 0;
}
"#;

/// Issue 113: getaddrinfo with AF_UNSPEC (the default of most programs)
/// works in the pod, also when the supervisor cannot write the memory of
/// the program (glibc sends A and AAAA with one sendmmsg; the pod sets
/// RES_OPTIONS=single-request). Before, it failed with "Temporary failure
/// in name resolution" under PODNET_NETNS_TEST_MEM=none.
#[test]
fn seccomp_backend_resolves_unspec_without_memory_writes() {
    if !have_compiler() || !seccomp_available() {
        eprintln!("skipping: no compiler or no SECCOMP_RET_USER_NOTIF");
        return;
    }
    let dir = std::env::temp_dir().join(format!("podnet-netns-unspec-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let sock = dir.join("socks.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || socks5_proxy(listener, tx, false));
    let program = build_program(&dir, "unspec", UNSPEC_CLIENT, false);
    for (hook, res_options) in [
        ("", None),
        ("no-fd", None),
        ("none", None),
        ("none", Some("ndots:1")),
    ] {
        let mut command = pod_command();
        command
            .args(["--backend", "seccomp", "-x"])
            .arg(format!("unix:{}", sock.display()))
            .arg("--")
            .arg(&program)
            .env("PODNET_NETNS_TEST_MEM", hook);
        match res_options {
            Some(v) => command.env("RES_OPTIONS", v),
            None => command.env_remove("RES_OPTIONS"),
        };
        let output = command.output().unwrap();
        let out = String::from_utf8_lossy(&output.stdout);
        let err = String::from_utf8_lossy(&output.stderr);
        assert!(
            out.contains("GAI:0"),
            "{hook:?} {res_options:?}: {out:?} {err:?}"
        );
        assert!(
            out.contains("HELLO-FROM-TARGET"),
            "{hook:?}: {out:?} {err:?}"
        );
        assert_eq!(
            rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap(),
            "example.com:80"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn seccomp_backend_proxies_ipv6_targets() {
    if !have_compiler() || !seccomp_available() {
        eprintln!("skipping: no compiler or no SECCOMP_RET_USER_NOTIF");
        return;
    }
    let dir = std::env::temp_dir().join(format!("podnet-netns-v6-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let sock = dir.join("socks.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || socks5_proxy(listener, tx, false));

    let program = build_program(&dir, "client6", IPV6_CLIENT, false);
    let stdout = run(&dir, &[&sock], &program);
    assert!(stdout.contains("HELLO-FROM-TARGET"), "{stdout:?}");
    // The address reaches the proxy as an IPv6 target, not as a name.
    assert_eq!(rx.recv().unwrap(), "2001:db8::1:80");
    let _ = std::fs::remove_dir_all(&dir);
}
