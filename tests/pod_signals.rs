//! T-517 (GitHub #691): a stop signal (SIGTERM, SIGINT, SIGHUP) that a
//! process sends to `podnet pod` goes to the program. podnet waits for the
//! program and exits with its status (128+N), and no process of the pod
//! stays: not the program, and not a process that the program started.
//!
//! Each row runs on each backend that `podnet pod doctor` finds. A host with
//! no backend (no user namespace and no seccomp notification) skips.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The backends that `podnet pod doctor` finds on this host.
fn backends() -> Vec<&'static str> {
    let out = Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(["pod", "doctor", "--json"])
        .output()
        .unwrap();
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    [
        ("seccomp", "backend_seccomp"),
        ("netns", "backend_netns"),
        ("netns-kernel", "backend_netns_kernel"),
    ]
    .into_iter()
    .filter(|(_, key)| report[*key] == serde_json::Value::Bool(true))
    .map(|(name, _)| name)
    .collect()
}

/// True while `pid` lives (a zombie does not live).
fn lives(pid: i32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/status")).is_ok_and(|status| {
        status
            .lines()
            .find_map(|l| l.strip_prefix("State:"))
            .is_some_and(|state| !state.trim_start().starts_with('Z'))
    })
}

/// Read the process id that the program writes to `path`.
fn read_pid(path: &std::path::Path) -> Option<i32> {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if let Some(pid) = std::fs::read_to_string(path)
            .ok()
            .and_then(|text| text.trim().parse().ok())
        {
            return Some(pid);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    None
}

#[test]
fn a_stop_signal_ends_the_program_and_each_process_of_the_pod() {
    let backends = backends();
    if backends.is_empty() {
        eprintln!("skipping, podnet pod doctor finds no backend on this host");
        return;
    }
    let signals = [
        (libc::SIGTERM, "SIGTERM"),
        (libc::SIGINT, "SIGINT"),
        (libc::SIGHUP, "SIGHUP"),
    ];
    let mut faults = Vec::new();
    for backend in &backends {
        for (signal, name) in signals {
            let dir = tempfile::tempdir().unwrap();
            let program = dir.path().join("program.pid");
            let daemon = dir.path().join("daemon.pid");
            // The program starts a process of its own, then is `sleep 37`.
            let script = format!(
                "sleep 41 & echo $! > {}; echo $$ > {}; exec sleep 37",
                daemon.display(),
                program.display()
            );
            let mut child = Command::new(env!("CARGO_BIN_EXE_podnet"))
                .args(["-q", "pod", "--backend", backend])
                .args(["-x", "socks5://127.0.0.1:9", "--", "sh", "-c", &script])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            let (Some(program), Some(daemon)) = (read_pid(&program), read_pid(&daemon)) else {
                let _ = child.kill();
                let out = child.wait_with_output().unwrap();
                faults.push(format!(
                    "{backend} {name}: the program did not start: {}",
                    String::from_utf8_lossy(&out.stderr)
                ));
                continue;
            };
            let sent = Instant::now();
            unsafe { libc::kill(child.id() as i32, signal) };
            let mut code = None;
            while sent.elapsed() < Duration::from_secs(5) {
                if let Some(status) = child.try_wait().unwrap() {
                    code = Some(status);
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            if code.is_none() {
                let _ = child.kill();
            }
            std::thread::sleep(Duration::from_millis(200));
            let stays: Vec<_> = [("the program", program), ("its process", daemon)]
                .into_iter()
                .filter(|(_, pid)| lives(*pid))
                .collect();
            // A process that stays holds the pipe of stderr: end it first.
            for (_, pid) in &stays {
                unsafe { libc::kill(*pid, libc::SIGKILL) };
            }
            let out = child.wait_with_output().unwrap();
            let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
            let lines: Vec<&str> = stderr.lines().collect();
            let mut row = Vec::new();
            match code {
                None => row.push("podnet did not exit in 5 s".to_string()),
                Some(status) if status.code() != Some(128 + signal) => row.push(format!(
                    "podnet ended with {status}, not exit {}",
                    128 + signal
                )),
                Some(_) => {}
            }
            if !stays.is_empty() {
                row.push(format!("{stays:?} stays"));
            }
            if lines.len() != 1 || !lines[0].contains(name) {
                row.push(format!("not one line that names {name}"));
            }
            if !row.is_empty() {
                faults.push(format!("{backend} {name}: {}\n{stderr}", row.join("; ")));
            }
        }
    }
    assert!(faults.is_empty(), "{}", faults.join("\n"));
}

/// #770: `pod --serve` is a server command: a stop signal ends it with one
/// line and exit 0, and its unix socket goes (`docs/cli.md`, Stop signals).
/// It runs no program, so it needs no backend.
#[test]
fn a_stop_signal_ends_pod_serve_cleanly_and_removes_its_socket() {
    let dir = tempfile::tempdir().unwrap();
    for (signal, name) in [
        (libc::SIGINT, "SIGINT"),
        (libc::SIGTERM, "SIGTERM"),
        (libc::SIGHUP, "SIGHUP"),
    ] {
        let sock = dir.path().join(format!("{name}.sock"));
        let mut child = Command::new(env!("CARGO_BIN_EXE_podnet"))
            .args([
                "pod",
                "--serve",
                &format!("unix:{}", sock.display()),
                "-x",
                "direct",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        while !sock.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(sock.exists(), "{name}: no socket");
        unsafe { libc::kill(child.id() as i32, signal) };
        let started = Instant::now();
        let out = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if started.elapsed() > Duration::from_secs(10) {
                let _ = child.kill();
                panic!("{name}: pod --serve did not end in 10 s");
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        let mut err = String::new();
        std::io::Read::read_to_string(&mut child.stderr.take().unwrap(), &mut err).unwrap();
        assert_eq!(out.code(), Some(0), "{name}: {err}");
        assert!(err.contains(&format!("{name}: stopping")), "{name}: {err}");
        assert!(!sock.exists(), "{name}: the socket stays");
    }
}

/// T-616: `net proxy --listen unix:PATH` and `net switch --listen unix:PATH`
/// remove their socket at a clean stop, so the next start (also of another
/// user in a shared directory) finds no stale file.
#[test]
fn a_clean_stop_of_a_unix_listener_removes_its_socket() {
    let dir = tempfile::tempdir().unwrap();
    for (name, command) in [("proxy", ["net", "proxy"]), ("switch", ["net", "switch"])] {
        let sock = dir.path().join(format!("{name}.sock"));
        let mut child = Command::new(env!("CARGO_BIN_EXE_podnet"))
            .args(command)
            .args(["--listen", &format!("unix:{}", sock.display())])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        while !sock.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(sock.exists(), "{name}: no socket");
        unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
        let status = child.wait().unwrap();
        assert_eq!(status.code(), Some(0), "{name}");
        assert!(!sock.exists(), "{name}: the socket stays");
    }
}
