//! T-109: `warp exec --detach --name N` keeps a pod under the supervisor.
//! A unix socket that answers is the stand-in of `podnet warp up`: the
//! program here opens no connection.

use super::*;

/// The backends that `podnet pod doctor` finds on this host.
fn a_pod_backend() -> bool {
    let out = Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(["pod", "doctor", "--json"])
        .output()
        .unwrap();
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
    report["backend_available"] == serde_json::Value::Bool(true)
}

/// The processes whose command line holds `word`.
fn processes_with(word: &str) -> Vec<i32> {
    let Ok(dir) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    dir.flatten()
        .filter_map(|e| e.file_name().to_str()?.parse::<i32>().ok())
        .filter(|pid| {
            std::fs::read(format!("/proc/{pid}/cmdline"))
                .map(|c| String::from_utf8_lossy(&c).contains(word))
                .unwrap_or(false)
        })
        .collect()
}

#[test]
fn warp_exec_detach_lists_the_pod() {
    if !a_pod_backend() {
        eprintln!("skipping: no backend of the pod on this host");
        return;
    }
    let env = Env::new();
    let socket = env.run.with_file_name("warp.sock");
    let _warp = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    // The first program starts a daemon and exits 0. The daemon writes to
    // its output (the log of the instance) and lives on.
    let daemon = env.run.with_file_name("pod-daemon.sh");
    std::fs::write(
        &daemon,
        "echo pod-daemon-says-hello\nwhile :; do sleep 1; done\n",
    )
    .unwrap();
    let starter = format!("sh {} & echo started", daemon.display());
    let start = env
        .cmd(&["warp", "exec", "--detach", "--name", "pod1", "--socket"])
        .arg(&socket)
        .args(["--", "sh", "-c", &starter])
        .env_remove("PODNET_WARP_SOCKET")
        .output()
        .unwrap();
    assert!(
        start.status.success(),
        "{}",
        String::from_utf8_lossy(&start.stderr)
    );
    let doc = ready_doc(&start);
    assert_eq!(doc["name"], "pod1", "{doc}");
    assert_eq!(doc["command"], "warp exec", "{doc}");
    let word = daemon.display().to_string();
    wait_until("the daemon runs", || !processes_with(&word).is_empty());
    // The first program exited; the pod stays in ps while the daemon lives.
    std::thread::sleep(Duration::from_secs(2));
    let listed = instance(&env, "pod1");
    assert_eq!(listed["state"], "ready", "{listed}");
    assert!(alive(listed["pid"].as_i64().unwrap() as i32), "{listed}");
    // logs N shows the output of the pod.
    wait_until("the log has the output of the daemon", || {
        let logs = env.run(&["logs", "pod1"]);
        String::from_utf8_lossy(&logs.stdout).contains("pod-daemon-says-hello")
    });
    // stop N ends the daemon too.
    let stop = env.run(&["stop", "pod1", "--timeout", "10"]);
    assert!(
        stop.status.success(),
        "{}",
        String::from_utf8_lossy(&stop.stderr)
    );
    wait_until("the daemon ends", || processes_with(&word).is_empty());
    assert!(instance(&env, "pod1").is_null());
}

/// The backends of the pod on this host (`pod doctor`).
fn backends() -> Vec<&'static str> {
    let out = Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(["pod", "doctor", "--json"])
        .output()
        .unwrap();
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
    ["netns", "seccomp"]
        .into_iter()
        .filter(|b| report[format!("backend_{b}")] == serde_json::Value::Bool(true))
        .collect()
}

/// A loopback listener in the pod: it resolves two names, writes their
/// addresses to a file, and answers each connection with one line.
const SERVER: &str = r#"
import socket, sys
names = [socket.gethostbyname(n) for n in ("a.example", "b.example")]
s = socket.socket()
s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(("127.0.0.1", 0))
s.listen()
with open(sys.argv[1] + ".tmp", "w") as f:
    f.write("%d %s %s\n" % (s.getsockname()[1], names[0], names[1]))
import os
os.rename(sys.argv[1] + ".tmp", sys.argv[1])
while True:
    c, _ = s.accept()
    c.sendall(b"pong-from-the-pod\n")
    c.close()
"#;

/// The client: one line from 127.0.0.1:PORT, then the address of b.example.
const CLIENT: &str = r#"
import socket, sys
c = socket.create_connection(("127.0.0.1", int(sys.argv[1])), timeout=5)
print(c.recv(100).decode().strip())
print(socket.gethostbyname("b.example"))
"#;

/// T-110: `warp exec --join N` reaches a loopback listener of the pod N,
/// and gets the addresses that the pod N gave to its names. With each
/// backend.
#[test]
fn warp_exec_join_reaches_a_listener_of_the_pod() {
    test_host::need_loopback!();
    if Command::new("python3").arg("-c").arg("1").output().is_err() {
        eprintln!("skipping: no python3");
        return;
    }
    let available = backends();
    if available.is_empty() {
        eprintln!("skipping: no backend of the pod on this host");
        return;
    }
    let env = Env::new();
    let socket = env.run.with_file_name("warp.sock");
    let _warp = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    let server = env.run.with_file_name("server.py");
    let client = env.run.with_file_name("client.py");
    std::fs::write(&server, SERVER).unwrap();
    std::fs::write(&client, CLIENT).unwrap();
    for backend in available {
        let name = format!("join-{backend}");
        let state = env.run.with_file_name(format!("{name}.state"));
        let start = env
            .cmd(&["-q", "warp", "exec", "--detach", "--name", &name])
            .args(["--backend", backend, "--socket"])
            .arg(&socket)
            .arg("--")
            .arg("python3")
            .arg(&server)
            .arg(&state)
            .env_remove("PODNET_WARP_SOCKET")
            .output()
            .unwrap();
        assert!(
            start.status.success(),
            "{backend}: {}",
            String::from_utf8_lossy(&start.stderr)
        );
        wait_until("the listener of the pod is ready", || state.exists());
        let text = std::fs::read_to_string(&state).unwrap();
        let fields: Vec<&str> = text.split_whitespace().collect();
        let (port, b_address) = (fields[0], fields[2]);
        let joined = env
            .cmd(&["-q", "warp", "exec", "--join", &name, "--", "python3"])
            .arg(&client)
            .arg(port)
            .env_remove("PODNET_WARP_SOCKET")
            .output()
            .unwrap();
        let out = String::from_utf8_lossy(&joined.stdout);
        assert!(
            joined.status.success(),
            "{backend}: {out}{}",
            String::from_utf8_lossy(&joined.stderr)
        );
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines, ["pong-from-the-pod", b_address], "{backend}");
        // The listener is not on the loopback of the host.
        assert!(
            std::net::TcpStream::connect(("127.0.0.1", port.parse::<u16>().unwrap())).is_err(),
            "{backend}"
        );
        let stop = env.run(&["stop", &name, "--timeout", "10"]);
        assert!(stop.status.success(), "{backend}");
    }
    // A name that is no pod of warp exec.
    let missing = env.run(&["warp", "exec", "--join", "none", "--", "true"]);
    assert_eq!(missing.status.code(), Some(69));
}
