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
