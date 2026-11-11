//! The veth wire of the netns backend (T-174). `PODNET_NETNS_TEST_VETH`
//! makes the namespace use a veth pair and a packet socket also where it
//! can open `/dev/net/tun`. The program resolves a name (UDP on the wire),
//! connects (TCP on the wire, after ARP for the gateway) and sends 256 KiB
//! that a SOCKS5 proxy on a unix socket echoes.

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::process::Command;
use std::sync::mpsc;
use std::time::Duration;

/// Set in the copy of this test that runs the pod: the arguments of the
/// pod, separated by a unit separator.
const POD_ARGS: &str = "PODNET_VETH_TEST_POD_ARGS";

/// The program: a name, a connect, 256 KiB out and back.
const PROGRAM: &str = r#"
import socket, threading
s = socket.create_connection(("veth.example", 80), 20)
s.settimeout(20)
data = bytes(range(256)) * 1024
threading.Thread(target=s.sendall, args=(data,)).start()
want = len(b"HELLO-FROM-TARGET\n") + len(data)
got = b""
while len(got) < want:
    chunk = s.recv(65536)
    if not chunk:
        break
    got += chunk
hello = got.startswith(b"HELLO-FROM-TARGET\n")
print("HELLO:%s ECHO:%d SAME:%s" % (hello, len(got) - 18, got[18:] == data))
"#;

fn python() -> Option<String> {
    ["python3", "python"]
        .iter()
        .find(|p| {
            Command::new(p)
                .args(["-c", "pass"])
                .status()
                .is_ok_and(|s| s.success())
        })
        .map(|p| p.to_string())
}

/// A SOCKS5 CONNECT proxy: it sends the target on `targets`, greets and
/// echoes.
fn socks5_proxy(listener: UnixListener, targets: mpsc::Sender<String>) {
    for conn in listener.incoming() {
        let Ok(mut c) = conn else { break };
        let targets = targets.clone();
        std::thread::spawn(move || {
            let mut head = [0u8; 2];
            c.read_exact(&mut head).ok()?;
            let mut methods = vec![0u8; usize::from(head[1])];
            c.read_exact(&mut methods).ok()?;
            c.write_all(&[5, 0]).ok()?;
            let mut req = [0u8; 4];
            c.read_exact(&mut req).ok()?;
            let host = match req[3] {
                1 => {
                    let mut a = [0u8; 4];
                    c.read_exact(&mut a).ok()?;
                    std::net::Ipv4Addr::from(a).to_string()
                }
                3 => {
                    let mut n = [0u8; 1];
                    c.read_exact(&mut n).ok()?;
                    let mut name = vec![0u8; usize::from(n[0])];
                    c.read_exact(&mut name).ok()?;
                    String::from_utf8_lossy(&name).into_owned()
                }
                _ => return None,
            };
            let mut port = [0u8; 2];
            c.read_exact(&mut port).ok()?;
            let _ = targets.send(format!("{host}:{}", u16::from_be_bytes(port)));
            c.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).ok()?;
            c.write_all(b"HELLO-FROM-TARGET\n").ok()?;
            let mut buf = [0u8; 16384];
            while let Ok(n) = c.read(&mut buf) {
                if n == 0 || c.write_all(&buf[..n]).is_err() {
                    break;
                }
            }
            Some(())
        });
    }
}

#[test]
fn veth_wire_carries_a_tcp_flow() {
    if let Ok(args) = std::env::var(POD_ARGS) {
        // The copy that runs the pod.
        let argv: Vec<String> = args.split('\u{1f}').map(String::from).collect();
        std::process::exit(podnet_netns::main_with_args(&argv));
    }
    let Some(python) = python() else {
        eprintln!("skipping: no python");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("socks.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || socks5_proxy(listener, tx));
    // The proxy answers before the pod starts.
    drop(UnixStream::connect(&sock).unwrap());

    let argv = [
        "--backend".to_string(),
        "netns".into(),
        "-v".into(),
        "-x".into(),
        format!("unix:{}", sock.display()),
        "--".into(),
        python,
        "-c".into(),
        PROGRAM.into(),
    ];
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["veth_wire_carries_a_tcp_flow", "--exact", "--nocapture"])
        .args(["--test-threads", "1"])
        .env(POD_ARGS, argv.join("\u{1f}"))
        .env("PODNET_NETNS_TEST_VETH", "1")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if output.status.code() == Some(3) && stderr.contains("unshare") {
        // No user namespace (AppArmor on GitHub runners).
        eprintln!("skipping: the netns backend cannot run here: {stderr}");
        return;
    }
    if stderr.contains("SIOCSIFFLAGS lo") {
        // The namespace has no capability: lo cannot be set up, before the
        // wire (the row "lo cannot be set up" of pod doctor; GitHub runners).
        eprintln!("skipping: the namespace cannot set up lo here: {stderr}");
        return;
    }
    assert!(
        stderr.contains("the wire of the namespace is a veth pair"),
        "the pod did not take the veth wire: {stderr}"
    );
    assert!(
        output.status.success(),
        "the pod failed: {stdout}\n{stderr}"
    );
    assert!(
        stdout.contains("HELLO:True ECHO:262144 SAME:True"),
        "stdout={stdout}\nstderr={stderr}"
    );
    let target = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(target, "veth.example:80", "the proxy got the name");
}
