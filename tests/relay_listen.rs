//! `podnet relay connect --listen` has the instance surface of `connect -L`
//! (T-199): `--detach`, `--name`, `--replace`, `ps`, `stop`, and a clean
//! end that removes its socket file.

mod test_host;

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn podnet(run: &std::path::Path, args: &[&str]) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_podnet"));
    cmd.args(args)
        .env("PODNET_RUN_DIR", run)
        .env_remove("RUST_LOG")
        .env_remove("PODNET_RELAY")
        .env_remove("PODNET_ERROR_PROGRAM");
    cmd
}

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(10);
    while !f() {
        assert!(Instant::now() < until, "{what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn relay_connect_listen_detach() {
    test_host::need_loopback!();
    let tmp = tempfile::tempdir().unwrap();
    let run = tmp.path().join("run");
    let out = podnet(
        &run,
        &[
            "relay",
            "connect",
            "x",
            "--token",
            "sekret-9",
            "--listen",
            "tcp://127.0.0.1:0",
            "--detach",
            "--name",
            "fwd",
        ],
    )
    .output()
    .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{err}");
    let ps = podnet(&run, &["ps", "--json"]).output().unwrap();
    let doc: serde_json::Value = serde_json::from_slice(&ps.stdout).unwrap();
    let names: Vec<_> = doc["instances"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(names, ["fwd"], "{doc}");
    assert!(
        !doc.to_string().contains("sekret-9"),
        "the token shows: {doc}"
    );
    let stop = podnet(&run, &["stop", "fwd"]).output().unwrap();
    assert!(
        stop.status.success(),
        "{}",
        String::from_utf8_lossy(&stop.stderr)
    );
    let ps = podnet(&run, &["ps", "--json"]).output().unwrap();
    let doc: serde_json::Value = serde_json::from_slice(&ps.stdout).unwrap();
    assert!(doc["instances"].as_array().unwrap().is_empty(), "{doc}");
}

#[test]
fn relay_connect_unix_socket_removed() {
    test_host::need_loopback!();
    let tmp = tempfile::tempdir().unwrap();
    let run = tmp.path().join("run");
    let sock = tmp.path().join("fwd.sock");
    let mut child = podnet(
        &run,
        &[
            "relay",
            "connect",
            "x",
            "--token",
            "t",
            "--listen",
            &format!("unix:{}", sock.display()),
        ],
    )
    .stdout(Stdio::null())
    .stderr(Stdio::piped())
    .spawn()
    .unwrap();
    wait_until("the socket file", || sock.exists());
    // SAFETY: a signal to the child of this test.
    unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
    let status = child.wait().unwrap();
    assert!(status.success(), "{status}");
    assert!(!sock.exists(), "the socket file stays");
}
