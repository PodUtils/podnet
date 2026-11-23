//! A listener that cannot bind (T-515, T-523, GitHub #689, #705, #706):
//! each command with a listener exits 69 and prints one line. The line names
//! the address one time and gives a next step. `warp proxy` and
//! `warp forward` bind before the registration of a device: the stand-in of
//! the WARP API gets no connection.
//!
//! The causes: a TCP address that another socket holds, a unix socket that
//! another process serves, and a unix socket in a directory that does not
//! exist. The cause EACCES of a host that refuses each TCP bind is a row of
//! the lab lane (`scripts/lab-lane.sh`, case `errors`).

use std::net::TcpListener;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

mod test_host;

/// A stand-in of the WARP API that counts the connections and closes each.
fn counting_api() -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let seen = count.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            seen.fetch_add(1, Ordering::SeqCst);
            drop(stream);
        }
    });
    (format!("http://{addr}"), count)
}

#[test]
fn a_listener_that_cannot_bind_exits_69_with_one_line_and_a_next_step() {
    test_host::need_loopback!();
    let dir = tempfile::tempdir().unwrap();
    let held = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = held.local_addr().unwrap().port().to_string();
    let addr = format!("127.0.0.1:{port}");
    let live = dir.path().join("live.sock");
    let _live = std::os::unix::net::UnixListener::bind(&live).unwrap();
    let live = format!("unix:{}", live.display());
    let missing = format!("unix:{}", dir.path().join("nodir/x.sock").display());
    let root = dir.path().join("root");
    std::fs::create_dir(&root).unwrap();
    let root = root.display().to_string();
    let tcp = format!("tcp://{addr}");
    let forward = format!("{port}=192.0.2.1:80");
    let ws = "wss://nowhere.invalid/__podnet/ws";

    // (the command, the address that the line names)
    let rows: Vec<(Vec<&str>, &str)> = vec![
        (vec!["connect", ws, "-L", &tcp, "--no-check"], &addr),
        (vec!["net", "proxy", "--listen", &tcp], &addr),
        (vec!["forward", "-L", &forward], &addr),
        (vec!["serve", &root, "--local", &addr], &addr),
        (
            vec![
                "git",
                "serve",
                "--root",
                &root,
                "--anon-read",
                "--ssh",
                &addr,
            ],
            &addr,
        ),
        (
            vec![
                "git",
                "serve",
                "--root",
                &root,
                "--anon-read",
                "--local",
                &addr,
            ],
            &addr,
        ),
        (
            vec![
                "tunnel",
                "--e2e",
                "--pair",
                "r/p",
                "--destination",
                "example.com:443",
                "--listen",
                &addr,
                "--control-listen",
                "127.0.0.1:0",
            ],
            &addr,
        ),
        (
            vec!["warp", "proxy", "--ephemeral", "--listen", &tcp],
            &addr,
        ),
        (
            vec!["warp", "forward", "--ephemeral", "-L", &forward],
            &addr,
        ),
        (vec!["connect", ws, "-L", &live, "--no-check"], &live),
        (vec!["net", "proxy", "--listen", &live], &live),
        (vec!["serve", &root, "--local", &live], &live),
        (
            vec!["warp", "proxy", "--ephemeral", "--listen", &live],
            &live,
        ),
        (vec!["connect", ws, "-L", &missing, "--no-check"], &missing),
        (vec!["net", "proxy", "--listen", &missing], &missing),
        (vec!["serve", &root, "--local", &missing], &missing),
        (
            vec!["warp", "proxy", "--ephemeral", "--listen", &missing],
            &missing,
        ),
    ];
    let mut faults = Vec::new();
    for (args, named) in &rows {
        let (api, calls) = counting_api();
        let out = Command::new(env!("CARGO_BIN_EXE_podnet"))
            .args(args)
            .args(["--run-for", "2"])
            .env("PODNET_WARP_API", &api)
            .env("PODNET_STATE_DIR", dir.path().join("state"))
            .env("PODNET_RUN_DIR", dir.path().join("run"))
            .env("HOME", dir.path())
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        let lines: Vec<&str> = stderr.lines().filter(|l| !l.trim().is_empty()).collect();
        let line = lines.first().copied().unwrap_or("");
        let step = line.contains("; give ")
            || line.contains("give another")
            || line.contains("create it")
            || line.contains("(podnet ps;");
        let fault = if out.status.code() != Some(69) {
            Some(format!("exit {:?}, not 69", out.status.code()))
        } else if lines.len() != 1 {
            Some(format!("{} lines, not one", lines.len()))
        } else if line.matches(named).count() != 1 {
            Some(format!(
                "the line names {named} {} times",
                line.matches(named).count()
            ))
        } else if !step {
            Some("the line gives no next step".to_string())
        } else if calls.load(Ordering::SeqCst) != 0 {
            Some("the WARP API got a request before the bind error".to_string())
        } else {
            None
        };
        if let Some(fault) = fault {
            faults.push(format!("podnet {}: {fault}\n{stderr}", args.join(" ")));
        }
    }
    drop(held);
    assert!(faults.is_empty(), "{}", faults.join("\n"));
}
