//! TCP: static and dynamic programs, chains, direct sends and the socket calls of a stream.

use super::*;

#[test]
fn netns_backend_proxies_a_static_and_a_dynamic_binary() {
    if !have_compiler() || !netns_available() {
        eprintln!("skipping: no compiler or no netns backend");
        return;
    }
    let dir = std::env::temp_dir().join(format!("podnet-netns-nsclient-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let sock = dir.join("socks.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || socks5_proxy(listener, tx, false));

    let dynamic = build_program(&dir, "client-dynamic", CLIENT, false);
    let statically = build_program(&dir, "client-static", CLIENT, true);
    for program in [&dynamic, &statically] {
        let stdout = run_on("netns", &[&sock], program);
        assert!(stdout.contains("HELLO-FROM-TARGET"), "{stdout:?}");
    }
    assert_eq!(rx.recv().unwrap(), "93.184.216.34:80");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A client whose upstream is dead: `connect()` must fail with
/// ECONNREFUSED at once (issue 190). Before, it succeeded, and a client
/// that waits for the server (TLS) hung.
pub(super) const DEAD_CLIENT: &str = r#"
#include <errno.h>
#include <stdio.h>
#include <string.h>
#include <time.h>
#include <unistd.h>
#include <arpa/inet.h>
#include <sys/socket.h>
int main(void) {
    alarm(20);
    struct timespec t0, t1;
    clock_gettime(CLOCK_MONOTONIC, &t0);
    int s = socket(AF_INET, SOCK_STREAM, 0);
    struct sockaddr_in a; memset(&a, 0, sizeof a);
    a.sin_family = AF_INET; a.sin_port = htons(443);
    inet_pton(AF_INET, "93.184.216.34", &a.sin_addr);
    int r = connect(s, (struct sockaddr *)&a, sizeof a);
    int e = errno;
    clock_gettime(CLOCK_MONOTONIC, &t1);
    long ms = (t1.tv_sec - t0.tv_sec) * 1000 + (t1.tv_nsec - t0.tv_nsec) / 1000000;
    printf("CONNECT:%d ERRNO:%d MS:%ld\n", r, r < 0 ? e : 0, ms);
    return 0;
}
"#;

#[test]
fn netns_backend_refuses_connect_when_the_upstream_is_dead() {
    if !have_compiler() || !netns_available() {
        eprintln!("skipping: no compiler or no netns backend");
        return;
    }
    let dir = std::env::temp_dir().join(format!("podnet-netns-nsdead-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let program = build_program(&dir, "dead", DEAD_CLIENT, false);
    let stdout = run_on("netns", &[&dir.join("nobody.sock")], &program);
    let want = format!("CONNECT:-1 ERRNO:{} ", libc::ECONNREFUSED);
    assert!(stdout.contains(&want), "{stdout:?}");
    let ms: u64 = stdout
        .rsplit("MS:")
        .next()
        .and_then(|v| v.trim().parse().ok())
        .unwrap();
    assert!(ms < 3000, "{stdout:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn seccomp_backend_proxies_a_static_and_a_dynamic_binary() {
    if !have_compiler() || !seccomp_available() {
        eprintln!("skipping: no compiler or no SECCOMP_RET_USER_NOTIF");
        return;
    }
    let dir = std::env::temp_dir().join(format!("podnet-netns-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let sock = dir.join("socks.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || socks5_proxy(listener, tx, false));

    let dynamic = build_program(&dir, "client-dynamic", CLIENT, false);
    let statically = build_program(&dir, "client-static", CLIENT, true);
    for program in [&dynamic, &statically] {
        let stdout = run(&dir, &[&sock], program);
        assert!(stdout.contains("HELLO-FROM-TARGET"), "{stdout:?}");
    }
    assert_eq!(rx.recv().unwrap(), "93.184.216.34:80");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn seccomp_backend_chains_two_proxies() {
    if !have_compiler() || !seccomp_available() {
        eprintln!("skipping: no compiler or no SECCOMP_RET_USER_NOTIF");
        return;
    }
    let dir = std::env::temp_dir().join(format!("podnet-netns-chain-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p1 = dir.join("p1.sock");
    let p2 = dir.join("p2.sock");
    let (tx1, rx1) = mpsc::channel();
    let (tx2, rx2) = mpsc::channel();
    let l2 = UnixListener::bind(&p2).unwrap();
    let l1 = UnixListener::bind(&p1).unwrap();
    std::thread::spawn(move || socks5_proxy(l2, tx2, false));
    std::thread::spawn(move || socks5_proxy(l1, tx1, true));

    let program = build_program(&dir, "client-chain", CLIENT, false);
    let stdout = run(&dir, &[&p1, &p2], &program);
    assert!(stdout.contains("HELLO-FROM-TARGET"), "{stdout:?}");
    // The first hop was asked for the second, not for the target.
    assert_eq!(rx1.recv().unwrap(), format!("unix:{}:0", p2.display()));
    assert_eq!(rx2.recv().unwrap(), "93.184.216.34:80");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Issue 61: the program does not get the proxy variables of the host (the
/// pod is its egress), and --keep-proxy-env keeps them.
#[test]
fn the_program_does_not_get_the_host_proxy() {
    if !seccomp_available() {
        eprintln!("skipping: no SECCOMP_RET_USER_NOTIF");
        return;
    }
    let run = |keep: bool| {
        let mut cmd = pod_command();
        cmd.args(["--backend", "seccomp"]);
        if keep {
            cmd.arg("--keep-proxy-env");
        }
        let out = cmd
            .args(["-x", "socks5://127.0.0.1:1", "--", "sh", "-c", "env"])
            .env("HTTPS_PROXY", "http://127.0.0.1:3128")
            .env("no_proxy", "localhost")
            .output()
            .unwrap();
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };
    let (env, stderr) = run(false);
    assert!(!env.lines().any(|l| l.starts_with("HTTPS_PROXY=")), "{env}");
    assert!(!env.lines().any(|l| l.starts_with("no_proxy=")), "{env}");
    assert!(stderr.contains("does not get"), "{stderr}");
    let (env, _) = run(true);
    assert!(
        env.lines()
            .any(|l| l == "HTTPS_PROXY=http://127.0.0.1:3128"),
        "{env}"
    );
}

/// Issue 80: a connect to 127.0.0.1 in a seccomp pod goes to the host
/// listener, not to the proxy (here the proxy socket does not exist, so a
/// connect through it fails).
#[test]
fn seccomp_backend_sends_loopback_direct() {
    test_host::need_loopback!();
    if !have_compiler() || !seccomp_available() {
        eprintln!("skipping: no compiler or no SECCOMP_RET_USER_NOTIF");
        return;
    }
    let dir = std::env::temp_dir().join(format!("podnet-netns-loop-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        if let Ok((mut conn, _)) = listener.accept() {
            let mut buf = [0u8; 16];
            let _ = conn.read(&mut buf);
            let _ = conn.write_all(b"LOCAL-SERVICE\n");
        }
    });
    let source = CLIENT
        .replace("htons(80)", &format!("htons({port})"))
        .replace("93.184.216.34", "127.0.0.1");
    let program = build_program(&dir, "client-loop", &source, false);
    let output = pod_command()
        .args(["--backend", "seccomp", "-x"])
        .arg(format!("unix:{}", dir.join("no-proxy.sock").display()))
        .arg("--")
        .arg(&program)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains("LOCAL-SERVICE"),
        "stdout={stdout:?} stderr={:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

pub(super) const SOCKNAME_CLIENT: &str = r#"
#include <stdio.h>
#include <string.h>
#include <errno.h>
#include <unistd.h>
#include <arpa/inet.h>
#include <sys/socket.h>
int main(void) {
    alarm(10);
    int s = socket(AF_INET, SOCK_DGRAM, 0);
    struct sockaddr_in d; memset(&d, 0, sizeof d);
    d.sin_family = AF_INET; d.sin_port = htons(123);
    inet_pton(AF_INET, "162.159.200.1", &d.sin_addr);
    if (connect(s, (struct sockaddr *)&d, sizeof d) < 0) { perror("connect"); return 1; }
    /* A guard after the length: an 8-byte write over socklen_t breaks it. */
    struct { struct sockaddr_in a; socklen_t len; unsigned guard; } p;
    memset(&p, 0, sizeof p); p.len = sizeof p.a; p.guard = 0x5a5a5a5a;
    int r = getpeername(s, (struct sockaddr *)&p.a, &p.len);
    if (r < 0) { printf("PEER:-1:%s\n", errno == EOPNOTSUPP ? "EOPNOTSUPP" : strerror(errno)); return 0; }
    char ip[32]; inet_ntop(AF_INET, &p.a.sin_addr, ip, sizeof ip);
    printf("PEER:%d:%u:%s:%d:guard=%x\n", p.a.sin_family, (unsigned)p.len, ip, ntohs(p.a.sin_port), p.guard);
    return 0;
}
"#;

/// Issue 98: getpeername of a pod socket gives the AF_INET address, through
/// /proc/<pid>/mem or through process_vm_writev; with no way to write the
/// memory of the program, it fails with EOPNOTSUPP (not the AF_UNIX pair).
#[test]
fn getpeername_is_the_inet_address_or_eopnotsupp() {
    test_host::need_loopback!();
    if !have_compiler() || !seccomp_available() {
        eprintln!("skipping: no compiler or no SECCOMP_RET_USER_NOTIF");
        return;
    }
    let dir = std::env::temp_dir().join(format!("podnet-netns-sockname-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let sock = dir.join("socks.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    let (tx, _rx) = mpsc::channel();
    std::thread::spawn(move || socks5_proxy(listener, tx, false));
    let program = build_program(&dir, "sockname", SOCKNAME_CLIENT, false);
    let run_with = |hook: &str| {
        let output = pod_command()
            .args(["--backend", "seccomp", "-x"])
            .arg(format!("unix:{}", sock.display()))
            .arg("--")
            .arg(&program)
            .env("PODNET_NETNS_TEST_MEM", hook)
            .output()
            .unwrap();
        (
            String::from_utf8_lossy(&output.stdout).to_string(),
            String::from_utf8_lossy(&output.stderr).to_string(),
        )
    };
    let want = "PEER:2:16:162.159.200.1:123:guard=5a5a5a5a";
    let (out, err) = run_with("");
    assert!(out.contains(want), "{out:?} {err:?}");
    let (out, err) = run_with("no-fd");
    assert!(out.contains(want), "process_vm_writev: {out:?} {err:?}");
    let (out, err) = run_with("none");
    assert!(out.contains("PEER:-1:EOPNOTSUPP"), "{out:?} {err:?}");
    assert!(err.contains("refuses writes to the memory"), "{err:?}");
    // Issue 186: the warning names the crash of a program that reads its
    // address, and the netns backend.
    assert!(
        err.contains("can crash") && err.contains("--backend netns"),
        "{err:?}"
    );
    // Issue 115: PODNET_NETNS_QUIET silences the warning.
    let output = pod_command()
        .args(["--backend", "seccomp", "-x"])
        .arg(format!("unix:{}", sock.display()))
        .arg("--")
        .arg(&program)
        .env("PODNET_NETNS_TEST_MEM", "none")
        .env("PODNET_NETNS_QUIET", "1")
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&output.stderr);
    assert!(!err.contains("refuses writes to the memory"), "{err:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

pub(super) const SHORT_SOCKNAME_CLIENT: &str = r#"
#include <stdio.h>
#include <string.h>
#include <errno.h>
#include <unistd.h>
#include <sys/mman.h>
#include <arpa/inet.h>
#include <sys/socket.h>
static void hex(const unsigned char *b, int n) { for (int i = 0; i < n; i++) printf("%02x", b[i]); }
int main(void) {
    alarm(10);
    int s = socket(AF_INET, SOCK_DGRAM, 0);
    struct sockaddr_in d; memset(&d, 0, sizeof d);
    d.sin_family = AF_INET; d.sin_port = htons(123);
    inet_pton(AF_INET, "162.159.200.1", &d.sin_addr);
    if (connect(s, (struct sockaddr *)&d, sizeof d) < 0) { perror("connect"); return 1; }
    /* An 8-byte buffer with a guard after it. */
    struct { unsigned char buf[8]; unsigned char guard[8]; } p;
    memset(p.buf, 0, 8); memset(p.guard, 0xbb, 8);
    socklen_t len = 8;
    int r = getpeername(s, (struct sockaddr *)p.buf, &len);
    printf("SHORT:%d:%u:", r, (unsigned)len); hex(p.buf, 8); printf(":"); hex(p.guard, 8); printf("\n");
    /* An 8-byte buffer at the end of a page; the next page is not mapped. */
    long page = sysconf(_SC_PAGESIZE);
    unsigned char *m = mmap(NULL, 2 * page, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (m == MAP_FAILED || mprotect(m + page, page, PROT_NONE) != 0) { perror("mmap"); return 1; }
    unsigned char *edge = m + page - 8;
    len = 8;
    r = getpeername(s, (struct sockaddr *)edge, &len);
    printf("EDGE:%d:%s:%u:", r, r < 0 ? strerror(errno) : "ok", (unsigned)len); hex(edge, 8); printf("\n");
    /* The length query: no buffer, length 0. */
    len = 0;
    r = getsockname(s, NULL, &len);
    printf("QUERY:%d:%u\n", r, (unsigned)len);
    return 0;
}
"#;

/// Issue 301: getsockname and getpeername write at most the length of the
/// caller, and give the full length back, as the kernel does. Before, the
/// supervisor wrote all 16 bytes past an 8-byte buffer.
#[test]
fn getpeername_does_not_write_past_the_buffer_of_the_caller() {
    if !have_compiler() || !seccomp_available() {
        eprintln!("skipping: no compiler or no SECCOMP_RET_USER_NOTIF");
        return;
    }
    let dir = std::env::temp_dir().join(format!("podnet-netns-shortname-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let sock = dir.join("socks.sock");
    let _listener = UnixListener::bind(&sock).unwrap();
    let program = build_program(&dir, "shortname", SHORT_SOCKNAME_CLIENT, false);
    // AF_INET, port 123, 162.159.200.1: the first 8 bytes of sockaddr_in.
    // sin_family (2) is in the byte order of the host.
    let family = if cfg!(target_endian = "little") {
        "0200"
    } else {
        "0002"
    };
    let head = format!("{family}007ba29fc801");
    for hook in ["", "no-fd"] {
        let output = pod_command()
            .args(["--backend", "seccomp", "-x"])
            .arg(format!("unix:{}", sock.display()))
            .arg("--")
            .arg(&program)
            .env("PODNET_NETNS_TEST_MEM", hook)
            .output()
            .unwrap();
        let out = String::from_utf8_lossy(&output.stdout);
        let err = String::from_utf8_lossy(&output.stderr);
        assert!(
            out.contains(&format!("SHORT:0:16:{head}:bbbbbbbbbbbbbbbb")),
            "{hook:?}: {out:?} {err:?}"
        );
        assert!(
            out.contains(&format!("EDGE:0:ok:16:{head}")),
            "{hook:?}: {out:?} {err:?}"
        );
        assert!(out.contains("QUERY:0:16"), "{hook:?}: {out:?} {err:?}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

pub(super) const SO_ERROR_CLIENT: &str = r#"
#include <stdio.h>
#include <string.h>
#include <errno.h>
#include <unistd.h>
#include <arpa/inet.h>
#include <sys/socket.h>
int main(void) {
    alarm(10);
    int s = socket(AF_INET, SOCK_STREAM | SOCK_NONBLOCK, 0);
    struct sockaddr_in a; memset(&a, 0, sizeof a);
    a.sin_family = AF_INET; a.sin_port = htons(9999);
    inet_pton(AF_INET, "10.66.0.1", &a.sin_addr);
    int r = connect(s, (struct sockaddr *)&a, sizeof a);
    printf("CONNECT:%d:%d\n", r, r < 0 ? errno : 0);
    for (int i = 0; i < 2; i++) {
        int err = -1; socklen_t len = sizeof err;
        r = getsockopt(s, SOL_SOCKET, SO_ERROR, &err, &len);
        printf("SOERROR%d:%d:%d:%u\n", i, r, err, (unsigned)len);
    }
    return 0;
}
"#;

/// Issue 302, T-223: a nonblocking connect gives EINPROGRESS; then SO_ERROR
/// gives the error of the dial once (ECONNREFUSED), then 0, as the kernel.
#[test]
fn so_error_gives_the_error_of_a_failed_connect() {
    if !have_compiler() || !seccomp_available() {
        eprintln!("skipping: no compiler or no SECCOMP_RET_USER_NOTIF");
        return;
    }
    let dir = std::env::temp_dir().join(format!("podnet-netns-soerror-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let program = build_program(&dir, "soerror", SO_ERROR_CLIENT, false);
    let stdout = run(&dir, &[&dir.join("nobody.sock")], &program);
    let refused = libc::ECONNREFUSED;
    assert!(
        stdout.contains(&format!("CONNECT:-1:{}\n", libc::EINPROGRESS)),
        "{stdout:?}"
    );
    assert!(
        stdout.contains(&format!("SOERROR0:0:{refused}:4\n")),
        "{stdout:?}"
    );
    assert!(stdout.contains("SOERROR1:0:0:4\n"), "{stdout:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
