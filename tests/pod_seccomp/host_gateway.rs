//! T-112: with `--host-gateway`, the gateway of a netns pod (10.66.255.254)
//! reaches the loopback of the host. Without it, the connect is refused.

use super::*;

/// The client in the pod: one line from TCP GATEWAY:PORT, then the answer
/// of UDP GATEWAY:UPORT. Prints `TCP:LINE` or `TCP-ERROR:KIND`, and
/// `UDP:ANSWER` or `UDP-ERROR:KIND`.
const CLIENT: &str = r#"
import socket, sys
gateway = "10.66.255.254"
try:
    c = socket.create_connection((gateway, int(sys.argv[1])), timeout=5)
    print("TCP:" + c.recv(100).decode().strip())
except OSError as e:
    print("TCP-ERROR:" + type(e).__name__)
u = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
u.settimeout(2)
try:
    u.sendto(b"ping", (gateway, int(sys.argv[2])))
    print("UDP:" + u.recv(100).decode())
except OSError as e:
    print("UDP-ERROR:" + type(e).__name__)
"#;

/// A TCP and a UDP service on the loopback of the host. Gives the two
/// ports.
fn host_services() -> (u16, u16) {
    let tcp = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let udp = UdpSocket::bind("127.0.0.1:0").unwrap();
    let ports = (
        tcp.local_addr().unwrap().port(),
        udp.local_addr().unwrap().port(),
    );
    std::thread::spawn(move || {
        for conn in tcp.incoming() {
            let Ok(mut c) = conn else { break };
            let _ = c.write_all(b"host-loopback\n");
        }
    });
    std::thread::spawn(move || {
        let mut buf = [0u8; 512];
        while let Ok((n, from)) = udp.recv_from(&mut buf) {
            let mut out = b"pong:".to_vec();
            out.extend_from_slice(&buf[..n]);
            let _ = udp.send_to(&out, from);
        }
    });
    ports
}

/// Run the client in a netns pod, with `options`. Gives stdout.
fn client_in_pod(options: &[&str], ports: (u16, u16)) -> String {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("socks.sock");
    let _listener = UnixListener::bind(&sock).unwrap();
    let client = dir.path().join("client.py");
    std::fs::write(&client, CLIENT).unwrap();
    let out = pod_command()
        .args(["--backend", "netns"])
        .args(options)
        .arg("-x")
        .arg(format!("socks5://unix:{}", sock.display()))
        .arg("--")
        .arg("python3")
        .arg(&client)
        .arg(ports.0.to_string())
        .arg(ports.1.to_string())
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

/// Whether the test can run: a loopback, the netns backend and python3.
fn can_run() -> bool {
    if !netns_available() {
        eprintln!("skipping: no netns backend");
        return false;
    }
    if std::process::Command::new("python3")
        .args(["-c", "1"])
        .output()
        .is_err()
    {
        eprintln!("skipping: no python3");
        return false;
    }
    true
}

#[test]
fn netns_host_gateway_reaches_the_host_loopback() {
    test_host::need_loopback!();
    if !can_run() {
        return;
    }
    let ports = host_services();
    let stdout = client_in_pod(&["--host-gateway"], ports);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines, ["TCP:host-loopback", "UDP:pong:ping"], "{stdout}");
}

#[test]
fn netns_without_host_gateway_is_refused() {
    test_host::need_loopback!();
    if !can_run() {
        return;
    }
    let ports = host_services();
    let stdout = client_in_pod(&[], ports);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines,
        ["TCP-ERROR:ConnectionRefusedError", "UDP-ERROR:TimeoutError"],
        "{stdout}"
    );
}
