//! T-456 (PodUtils/podrunner#213): a loopback TCP connect under the seccomp
//! backend on a host that refuses writes to the memory of the program. The
//! program holds the TCP socket of the kernel, so `getsockname` and
//! `getpeername` give the INET addresses (not EOPNOTSUPP).

use super::*;

/// The client in the pod: one line from TCP 127.0.0.1:PORT. Prints
/// `LOCAL:IP`, `PEER:IP:PORT` (or `NAME-ERROR:ERRNO`), then `LINE:TEXT`.
const CLIENT: &str = r#"
import socket, sys
port = int(sys.argv[1])
c = socket.create_connection(("127.0.0.1", port), timeout=5)
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
    assert_eq!(lines[1..], want[1..], "{stdout}");
}
