//! The spool of PodUtils/podnet#499: a real process takes an entry and is
//! killed before it consumes it. The entry is still in the spool, and a
//! later `podnet offline spool take` gives its body and removes it.
//!
//! The child is this test binary, run with `--exact` on one test that does
//! nothing unless `PODNET_SPOOL_CHILD` is set.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn podnet(dir: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_podnet"));
    cmd.args(args)
        .env("PODNET_STATE_DIR", dir.join("state"))
        .env("PODNET_RUN_DIR", dir.join("run"))
        .env_remove("RUST_LOG");
    cmd
}

fn state(dir: &Path) -> podnet::identity::Identity {
    std::fs::create_dir_all(dir).expect("a state directory");
    podnet::identity::create(dir).expect("a key")
}

#[test]
fn a_taken_entry_survives_a_kill_before_its_consume() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let spool = dir.path().join("spool");
    let body = dir.path().join("body");
    std::fs::write(&body, b"the bytes of the entry\n").expect("a body");
    let key = dir.path().join("key");
    let me = state(&key);

    let add = podnet(
        dir.path(),
        &[
            "offline",
            "spool",
            "add",
            spool.to_str().expect("a path"),
            "--key",
            key.to_str().expect("a path"),
            "--sequence",
            "0",
            "--file",
            body.to_str().expect("a path"),
        ],
    )
    .output()
    .expect("spool add");
    assert_eq!(add.status.code(), Some(0), "{add:?}");
    let listed = String::from_utf8_lossy(&add.stdout);
    assert!(listed.starts_with("0 "), "{listed:?}");
    assert!(
        listed.contains(&me.public().hex()),
        "the sender is in {listed:?}"
    );

    // The child receives the entry, writes a marker, and waits to be
    // killed. It never calls consume.
    let marker = dir.path().join("received");
    let mut child = Command::new(std::env::current_exe().expect("the test binary"))
        .args(["--exact", "child_receives_then_waits", "--nocapture"])
        .env("PODNET_SPOOL_CHILD", spool.to_str().expect("a path"))
        .env("PODNET_SPOOL_MARKER", marker.to_str().expect("a path"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("the child");

    let deadline = Instant::now() + Duration::from_secs(30);
    while !marker.exists() {
        assert!(Instant::now() < deadline, "the child never received");
        std::thread::sleep(Duration::from_millis(20));
    }
    child.kill().expect("a kill");
    let waited = child.wait().expect("the wait of the child");
    assert!(
        !waited.success(),
        "the child ended without the kill: {waited:?}"
    );

    let list = podnet(
        dir.path(),
        &["offline", "spool", "list", spool.to_str().expect("a path")],
    )
    .output()
    .expect("spool list");
    assert_eq!(list.status.code(), Some(0), "{list:?}");
    let lines = String::from_utf8_lossy(&list.stdout);
    assert_eq!(lines.lines().count(), 1, "the entry is gone: {lines:?}");

    let taken = dir.path().join("taken");
    let take = podnet(
        dir.path(),
        &[
            "offline",
            "spool",
            "take",
            spool.to_str().expect("a path"),
            "--out",
            taken.to_str().expect("a path"),
        ],
    )
    .output()
    .expect("spool take");
    assert_eq!(take.status.code(), Some(0), "{take:?}");
    assert_eq!(
        std::fs::read(&taken).expect("the body"),
        b"the bytes of the entry\n"
    );

    let again = dir.path().join("again");
    let second = podnet(
        dir.path(),
        &[
            "offline",
            "spool",
            "take",
            spool.to_str().expect("a path"),
            "--out",
            again.to_str().expect("a path"),
        ],
    )
    .output()
    .expect("the second take");
    assert_eq!(
        second.status.code(),
        Some(66),
        "an empty spool exits 66: {second:?}"
    );
    assert!(!again.exists(), "the second take wrote a file");
}

/// Run by the parent with `PODNET_SPOOL_CHILD`. It receives one entry,
/// writes the marker, and waits for a signal that only the parent gives.
#[test]
fn child_receives_then_waits() {
    let Ok(spool) = std::env::var("PODNET_SPOOL_CHILD") else {
        return;
    };
    let marker: PathBuf = std::env::var("PODNET_SPOOL_MARKER")
        .expect("a marker")
        .into();
    let spool = podnet::offline::spool::Spool::open(&spool).expect("the spool");
    let entry = spool.receive().expect("a read").expect("an entry");
    assert!(!entry.body.is_empty());
    std::fs::write(&marker, b"received").expect("the marker");
    loop {
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// T-613: `take --out` writes a new file. A file that exists exits 64 and
/// keeps the entry; a directory that does not exist exits 71; a new path
/// takes the entry.
#[test]
fn take_out_refuses_a_file_that_exists_and_keeps_the_entry() {
    let dir = tempfile::tempdir().expect("a temp dir");
    state(&dir.path().join("state"));
    let spool = dir.path().join("spool");
    let body = dir.path().join("body.txt");
    std::fs::write(&body, "the body of T-613\n").expect("a body");
    let s = spool.to_str().expect("a path");
    let add = podnet(
        dir.path(),
        &[
            "offline",
            "spool",
            "add",
            s,
            "--file",
            body.to_str().unwrap(),
        ],
    )
    .output()
    .expect("spool add");
    assert_eq!(add.status.code(), Some(0), "{add:?}");
    let missing = dir.path().join("no-such-dir/out.txt");
    // (the --out path, the exit code)
    let cases = [(&body, 64), (&missing, 71)];
    for (out, code) in cases {
        let take = podnet(
            dir.path(),
            &[
                "offline",
                "spool",
                "take",
                s,
                "--out",
                out.to_str().unwrap(),
            ],
        )
        .output()
        .expect("spool take");
        assert_eq!(take.status.code(), Some(code), "{take:?}");
    }
    assert_eq!(
        std::fs::read_to_string(&body).unwrap(),
        "the body of T-613\n"
    );
    let fresh = dir.path().join("taken.txt");
    let take = podnet(
        dir.path(),
        &[
            "offline",
            "spool",
            "take",
            s,
            "--out",
            fresh.to_str().unwrap(),
        ],
    )
    .output()
    .expect("spool take");
    assert_eq!(take.status.code(), Some(0), "the entry stayed: {take:?}");
    assert_eq!(
        std::fs::read_to_string(&fresh).unwrap(),
        "the body of T-613\n"
    );
}
