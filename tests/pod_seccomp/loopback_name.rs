//! T-456 (PodUtils/podrunner#213): a loopback TCP connect under the seccomp
//! backend on a host that refuses writes to the memory of the program. The
//! program holds a TCP socket of the kernel from `socket` on, so a socket
//! option before the connect (.NET reads `IPV6_V6ONLY`), `getsockname` and
//! `getpeername` give the answers of the kernel (not EOPNOTSUPP).

use super::*;

/// The client in the pod: one line from TCP 127.0.0.1:PORT. Prints
/// `NODELAY:1` (or `OPTION-ERROR:ERRNO`), `LOCAL:IP`, `PEER:IP:PORT` (or
/// `NAME-ERROR:ERRNO`), then `LINE:TEXT`.
const CLIENT: &str = r#"
import socket, sys
port = int(sys.argv[1])
c = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
c.settimeout(5)
# Part 2: an option before the connect, as .NET reads IPV6_V6ONLY.
try:
    c.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
    print("NODELAY:%d" % c.getsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY))
except OSError as e:
    print("OPTION-ERROR:%d" % e.errno)
c.connect(("127.0.0.1", port))
try:
    print("LOCAL:" + c.getsockname()[0])
    peer = c.getpeername()
    print("PEER:%s:%d" % (peer[0], peer[1]))
except OSError as e:
    print("NAME-ERROR:%d" % e.errno)
print("LINE:" + c.recv(100).decode().strip())
"#;

/// A TCP service on the loopback of the host. Gives its port.
fn host_service() -> u16 {
    let tcp = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = tcp.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for conn in tcp.incoming() {
            let Ok(mut c) = conn else { break };
            let _ = c.write_all(b"host-loopback\n");
        }
    });
    port
}

fn client_in_pod(hook: &str, port: u16) -> String {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("socks.sock");
    let _listener = UnixListener::bind(&sock).unwrap();
    let client = dir.path().join("client.py");
    std::fs::write(&client, CLIENT).unwrap();
    let out = pod_command()
        .args(["--backend", "seccomp", "-x"])
        .arg(format!("unix:{}", sock.display()))
        .arg("--")
        .arg("python3")
        .arg(&client)
        .arg(port.to_string())
        .env("PODNET_NETNS_TEST_MEM", hook)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn a_loopback_connect_has_its_names_with_no_memory_write() {
    test_host::need_loopback!();
    if !seccomp_available() {
        eprintln!("skipping: no SECCOMP_RET_USER_NOTIF");
        return;
    }
    if std::process::Command::new("python3")
        .args(["-c", "1"])
        .output()
        .is_err()
    {
        eprintln!("skipping: no python3");
        return;
    }
    let port = host_service();
    let want = [
        "NODELAY:1".to_string(),
        "LOCAL:127.0.0.1".to_string(),
        format!("PEER:127.0.0.1:{port}"),
        "LINE:host-loopback".to_string(),
    ];
    // With no memory write, the kernel answers for its own TCP socket.
    let stdout = client_in_pod("none", port);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines, want, "{stdout}");
    // With memory writes, the pod answers (the local address is the
    // unspecified address of the family).
    let stdout = client_in_pod("", port);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[2..], want[2..], "{stdout}");
    assert_eq!(lines[0], want[0], "{stdout}");
}
