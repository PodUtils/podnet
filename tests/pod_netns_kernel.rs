//! T-153: the `netns-kernel` backend. The kernel of the pod redirects each
//! TCP connection and each UDP datagram of the program to a listener of the
//! namespace; the parent serves them through the upstream proxy. Each test
//! skips with a named line where the host cannot unshare, or has no
//! nf_tables or conntrack in a namespace (`pod doctor`, `backend_netns_kernel`).

use std::io::{Read, Write};
use std::os::unix::net::UnixListener;
use std::sync::mpsc;

#[path = "pod_harness/mod.rs"]
mod pod_harness;
mod test_host;

use pod_harness::*;

/// True when the backend can run here; else print the named skip line.
fn kernel_available() -> bool {
    if !have_compiler() {
        test_host::skip_line("no C compiler for the program of the test");
        return false;
    }
    let out = pod_command()
        .args(["doctor", "--json"])
        .output()
        .expect("run podnet pod doctor");
    let report: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("doctor --json prints JSON");
    // The row of the probe says if the host can run the plane. A host that
    // can, and a binary that does not report the backend, fail the test.
    let row = report["capabilities"]
        .as_object()
        .and_then(|rows| {
            rows.iter()
                .find(|(name, _)| name.starts_with("netfilter in a namespace"))
        })
        .map(|(_, row)| row.clone());
    let Some(row) = row else {
        panic!("pod doctor has no row `netfilter in a namespace`");
    };
    if row["ok"] != serde_json::Value::Bool(true) {
        test_host::skip_line(&format!(
            "the netns-kernel backend cannot run on this host: {}",
            row["detail"].as_str().unwrap_or("no detail")
        ));
        return false;
    }
    assert_eq!(
        report["backend_netns_kernel"],
        serde_json::Value::Bool(true),
        "the probe passes, so pod doctor must report the backend"
    );
    true
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("podnet-nk-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A client that resolves a name and connects to it on port 8080.
const NAME_CLIENT: &str = r#"
#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include <netdb.h>
#include <sys/socket.h>
int main(void) {
    alarm(10);
    struct addrinfo hints, *res = NULL;
    memset(&hints, 0, sizeof hints);
    hints.ai_family = AF_INET; hints.ai_socktype = SOCK_STREAM;
    int r = getaddrinfo("origin.example", "8080", &hints, &res);
    if (r != 0) { printf("GAI:%d\n", r); return 1; }
    int s = socket(AF_INET, SOCK_STREAM, 0);
    if (connect(s, res->ai_addr, res->ai_addrlen) < 0) { perror("connect"); return 1; }
    write(s, "PING\n", 5);
    char b[64] = {0};
    int n = read(s, b, sizeof b - 1);
    printf("READ:%d:%s", n, b);
    return 0;
}
"#;

/// A client with a connect to a refused upstream: it must see an error.
const REFUSED_CLIENT: &str = r#"
#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include <arpa/inet.h>
#include <sys/socket.h>
int main(void) {
    alarm(10);
    int s = socket(AF_INET, SOCK_STREAM, 0);
    struct sockaddr_in a; memset(&a, 0, sizeof a);
    a.sin_family = AF_INET; a.sin_port = htons(80);
    inet_pton(AF_INET, "93.184.216.34", &a.sin_addr);
    if (connect(s, (struct sockaddr *)&a, sizeof a) < 0) { perror("connect"); return 1; }
    char b[64] = {0};
    int n = read(s, b, sizeof b - 1);
    printf("READ:%d\n", n);
    return 0;
}
"#;

/// Two UDP sockets send to one destination and port; each must get its own
/// answer, from the address that it sent to.
const TWO_UDP_CLIENT: &str = r#"
#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include <arpa/inet.h>
#include <sys/socket.h>
int main(void) {
    alarm(10);
    struct sockaddr_in d; memset(&d, 0, sizeof d);
    d.sin_family = AF_INET; d.sin_port = htons(9999);
    inet_pton(AF_INET, "93.184.216.34", &d.sin_addr);
    int a = socket(AF_INET, SOCK_DGRAM, 0), b = socket(AF_INET, SOCK_DGRAM, 0);
    sendto(a, "AAAA", 4, 0, (struct sockaddr *)&d, sizeof d);
    sendto(b, "BBBB", 4, 0, (struct sockaddr *)&d, sizeof d);
    sendto(a, "aaaa", 4, 0, (struct sockaddr *)&d, sizeof d);
    int socks[2] = {a, b};
    for (int i = 0; i < 2; i++) {
        char buf[64] = {0};
        struct sockaddr_in from; socklen_t len = sizeof from;
        int n = recvfrom(socks[i], buf, sizeof buf - 1, 0, (struct sockaddr *)&from, &len);
        char ip[32] = {0};
        inet_ntop(AF_INET, &from.sin_addr, ip, sizeof ip);
        printf("U%d:%d:%s:%s:%u\n", i, n, n > 0 ? buf : "", ip, ntohs(from.sin_port));
    }
    char buf[64] = {0};
    int n = recvfrom(a, buf, sizeof buf - 1, 0, NULL, NULL);
    printf("U2:%d:%s\n", n, n > 0 ? buf : "");
    return 0;
}
"#;

/// A UDP name client: the name of the pod goes to the proxy as the name.
const NAME_UDP_CLIENT: &str = r#"
#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include <netdb.h>
#include <sys/socket.h>
int main(void) {
    alarm(10);
    struct addrinfo hints, *res = NULL;
    memset(&hints, 0, sizeof hints);
    hints.ai_family = AF_INET; hints.ai_socktype = SOCK_DGRAM;
    int r = getaddrinfo("stun.example", "19302", &hints, &res);
    if (r != 0) { printf("GAI:%d\n", r); return 1; }
    int s = socket(AF_INET, SOCK_DGRAM, 0);
    if (sendto(s, "PING", 4, 0, res->ai_addr, res->ai_addrlen) < 0) { perror("sendto"); return 1; }
    char b[128] = {0};
    int n = recvfrom(s, b, sizeof b - 1, 0, NULL, NULL);
    printf("UDP:%d:%s", n, n > 0 ? b : "");
    return 0;
}
"#;

/// A proxy on a unix socket that knows the UDP association in the stream
/// (0x83): it answers each datagram with `ECHO:` and the payload, from the
/// address of the datagram, and gives each destination to `dests`.
fn stream_echo_proxy(listener: UnixListener, dests: mpsc::Sender<String>) {
    for conn in listener.incoming() {
        let Ok(mut c) = conn else { break };
        let dests = dests.clone();
        std::thread::spawn(move || {
            let Some((cmd, _, _)) = read_request(&mut c) else {
                return;
            };
            if cmd != 0x83 {
                let _ = c.write_all(&[5, 7, 0, 1, 0, 0, 0, 0, 0, 0]);
                return;
            }
            let _ = c.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
            loop {
                let mut len = [0u8; 2];
                if c.read_exact(&mut len).is_err() {
                    return;
                }
                let mut d = vec![0u8; u16::from_be_bytes(len) as usize];
                if c.read_exact(&mut d).is_err() {
                    return;
                }
                let header = match d[3] {
                    1 => 10,
                    3 => 7 + d[4] as usize,
                    _ => continue,
                };
                let dest = match d[3] {
                    3 => String::from_utf8_lossy(&d[5..5 + d[4] as usize]).into_owned(),
                    _ => std::net::Ipv4Addr::new(d[4], d[5], d[6], d[7]).to_string(),
                };
                let port = u16::from_be_bytes([d[header - 2], d[header - 1]]);
                let _ = dests.send(format!("{dest}:{port}"));
                let mut answer = d[..header].to_vec();
                answer.extend_from_slice(b"ECHO:");
                answer.extend_from_slice(&d[header..]);
                let mut frame = (answer.len() as u16).to_be_bytes().to_vec();
                frame.extend_from_slice(&answer);
                if c.write_all(&frame).is_err() {
                    return;
                }
            }
        });
    }
}

/// A TCP SOCKS5 proxy on the loopback of the host that greets and echoes.
fn tcp_proxy(targets: mpsc::Sender<String>) -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut c) = conn else { break };
            let targets = targets.clone();
            std::thread::spawn(move || {
                let mut hdr = [0u8; 2];
                if c.read_exact(&mut hdr).is_err() {
                    return;
                }
                let mut methods = vec![0u8; hdr[1] as usize];
                let _ = c.read_exact(&mut methods);
                let _ = c.write_all(&[5, 0]);
                let mut req = [0u8; 4];
                if c.read_exact(&mut req).is_err() {
                    return;
                }
                let host = match req[3] {
                    1 => {
                        let mut a = [0u8; 4];
                        let _ = c.read_exact(&mut a);
                        std::net::Ipv4Addr::from(a).to_string()
                    }
                    _ => {
                        let mut l = [0u8; 1];
                        let _ = c.read_exact(&mut l);
                        let mut h = vec![0u8; l[0] as usize];
                        let _ = c.read_exact(&mut h);
                        String::from_utf8_lossy(&h).into_owned()
                    }
                };
                let mut p = [0u8; 2];
                let _ = c.read_exact(&mut p);
                let _ = targets.send(format!("{host}:{}", u16::from_be_bytes(p)));
                let _ = c.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
                let _ = c.write_all(b"HELLO-FROM-TARGET\n");
                let mut buf = [0u8; 4096];
                while let Ok(n) = c.read(&mut buf) {
                    if n == 0 || c.write_all(&buf[..n]).is_err() {
                        break;
                    }
                }
            });
        }
    });
    port
}

fn run_kernel(proxy: &str, program: &std::path::Path) -> std::process::Output {
    pod_command()
        .args(["--backend", "netns-kernel", "-v", "-x", proxy, "--"])
        .arg(program)
        .output()
        .expect("run podnet pod")
}

#[test]
fn netns_kernel_relays_tcp() {
    if !kernel_available() {
        return;
    }
    let dir = temp_dir("tcp");
    let sock = dir.join("socks.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || socks5_proxy(listener, tx, false));
    // An address, then a name (the fake DNS answers the query that the
    // kernel redirects to the DNS socket).
    let by_address = build_program(&dir, "by-address", CLIENT, false);
    let output = run_kernel(&format!("unix:{}", sock.display()), &by_address);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("HELLO-FROM-TARGET"),
        "{stdout:?} {:?}",
        output
    );
    assert_eq!(rx.recv().unwrap(), "93.184.216.34:80");
    let by_name = build_program(&dir, "by-name", NAME_CLIENT, false);
    let output = run_kernel(&format!("unix:{}", sock.display()), &by_name);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("HELLO-FROM-TARGET"),
        "{stdout:?} {:?}",
        output
    );
    assert_eq!(rx.recv().unwrap(), "origin.example:8080");
    // A static program gets the same treatment.
    let statically = build_program(&dir, "static", CLIENT, true);
    let output = run_kernel(&format!("unix:{}", sock.display()), &statically);
    assert!(String::from_utf8_lossy(&output.stdout).contains("HELLO-FROM-TARGET"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn netns_kernel_relays_tcp_through_a_proxy_on_the_loopback() {
    if !kernel_available() {
        return;
    }
    let dir = temp_dir("tcp-lo");
    let (tx, rx) = mpsc::channel();
    let port = tcp_proxy(tx);
    let program = build_program(&dir, "by-address", CLIENT, false);
    let output = run_kernel(&format!("socks5://127.0.0.1:{port}"), &program);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("HELLO-FROM-TARGET"),
        "{stdout:?} {:?}",
        output
    );
    assert_eq!(rx.recv().unwrap(), "93.184.216.34:80");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn netns_kernel_resets_a_connect_when_the_upstream_is_dead() {
    if !kernel_available() {
        return;
    }
    let dir = temp_dir("dead");
    // A port that nothing listens on.
    let free = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let program = build_program(&dir, "refused", REFUSED_CLIENT, false);
    let output = run_kernel(&format!("socks5://127.0.0.1:{free}"), &program);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    // The program sees a reset (a read error) or an end, never data.
    assert!(!stdout.contains("HELLO"), "{stdout:?}");
    assert!(
        stderr.contains("failed") || stderr.contains("connect"),
        "{stderr:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn netns_kernel_two_udp_flows_to_one_port_lose_nothing() {
    if !kernel_available() {
        return;
    }
    let dir = temp_dir("udp2");
    let sock = dir.join("socks.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    let (dests, seen) = mpsc::channel();
    std::thread::spawn(move || stream_echo_proxy(listener, dests));
    let program = build_program(&dir, "two-udp", TWO_UDP_CLIENT, false);
    let output = run_kernel(&format!("unix:{}", sock.display()), &program);
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Each socket gets its own answer, from the address that it sent to.
    assert!(
        stdout.contains("U0:9:ECHO:AAAA:93.184.216.34:9999"),
        "{stdout:?} {:?}",
        output
    );
    assert!(
        stdout.contains("U1:9:ECHO:BBBB:93.184.216.34:9999"),
        "{stdout:?}"
    );
    // The second datagram of the first socket is not lost.
    assert!(stdout.contains("U2:9:ECHO:aaaa"), "{stdout:?}");
    let dests: Vec<String> = seen.try_iter().collect();
    assert_eq!(dests.len(), 3, "{dests:?}");
    assert!(dests.iter().all(|d| d == "93.184.216.34:9999"), "{dests:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn netns_kernel_sends_a_udp_name_to_the_proxy_as_the_name() {
    if !kernel_available() {
        return;
    }
    let dir = temp_dir("udp-name");
    let sock = dir.join("socks.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    let (dests, seen) = mpsc::channel();
    std::thread::spawn(move || stream_echo_proxy(listener, dests));
    let program = build_program(&dir, "udp-name", NAME_UDP_CLIENT, false);
    let output = run_kernel(&format!("unix:{}", sock.display()), &program);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("UDP:9:ECHO:PING"),
        "{stdout:?} {:?}",
        output
    );
    assert_eq!(seen.try_recv().unwrap(), "stun.example:19302");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn netns_kernel_is_not_in_the_auto_order() {
    // `auto` names netns or seccomp, never netns-kernel.
    let out = pod_command()
        .args(["--name-backend", "-x", "socks5://127.0.0.1:9", "--", "true"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("netns-kernel"), "{stderr:?}");
}
