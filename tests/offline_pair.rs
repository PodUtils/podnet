//! The eight-word pairing code of PodUtils/podnet#499: an offer bound to the
//! key of the giver, an accept bound to the key of the taker, and a code
//! that is used once.
//!
//! The code goes through stdin, the offer through a file or a spool. A
//! second accept with the same code and the same used-code directory exits
//! 77. The spool path also proves that the accept takes the offer and
//! consumes its entry.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use podnet::offline::pairing::{Code, Paired};

fn podnet(dir: &Path, args: &[&str], stdin: Option<&[u8]>) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_podnet"));
    cmd.args(args)
        .env("PODNET_STATE_DIR", dir.join("state"))
        .env("PODNET_RUN_DIR", dir.join("run"))
        .env_remove("RUST_LOG")
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd
}

fn key(dir: &Path) -> podnet::identity::Identity {
    std::fs::create_dir_all(dir).expect("a state directory");
    podnet::identity::create(dir).expect("a key")
}

fn run(dir: &Path, args: &[&str], stdin: Option<&[u8]>) -> std::process::Output {
    let mut child = podnet(dir, args, stdin).spawn().expect("podnet");
    if let Some(bytes) = stdin {
        child
            .stdin
            .take()
            .expect("the stdin")
            .write_all(bytes)
            .expect("the code");
    }
    child.wait_with_output().expect("the output")
}

fn offer(dir: &Path, key_dir: &Path, out: &Path) -> String {
    let out = run(
        dir,
        &[
            "offline",
            "pair",
            "offer",
            "--key",
            key_dir.to_str().expect("a path"),
            "--out",
            out.to_str().expect("a path"),
        ],
        None,
    );
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let code = String::from_utf8_lossy(&out.stdout).trim().to_string();
    assert_eq!(
        code.split('-').count(),
        podnet::offline::pairing::WORDS_COUNT,
        "the code is {code:?}"
    );
    code
}

#[test]
fn a_code_pairs_the_two_keys_once_through_a_file() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let giver = key(&dir.path().join("giver"));
    let taker = key(&dir.path().join("taker"));
    let offer_file = dir.path().join("offer");
    let code_text = offer(dir.path(), &dir.path().join("giver"), &offer_file);
    let code = Code::parse(&code_text).expect("a code");

    let used = dir.path().join("used");
    let paired_file = dir.path().join("paired");
    let out = run(
        dir.path(),
        &[
            "offline",
            "pair",
            "accept",
            "--key",
            dir.path().join("taker").to_str().expect("a path"),
            "--offer",
            offer_file.to_str().expect("a path"),
            "--used",
            used.to_str().expect("a path"),
            "--out",
            paired_file.to_str().expect("a path"),
        ],
        Some(code_text.as_bytes()),
    );
    assert_eq!(out.status.code(), Some(0), "{out:?}");

    let bytes = std::fs::read(&paired_file).expect("the paired record");
    let paired = Paired::decode(&bytes).expect("a paired record");
    assert_eq!(paired.identity, taker.public());
    assert_eq!(paired.peer, giver.public());
    assert!(paired.verify(&code), "the proof of the taker");

    // The second use of the same code is refused.
    let second = dir.path().join("paired-again");
    let out = run(
        dir.path(),
        &[
            "offline",
            "pair",
            "accept",
            "--key",
            dir.path().join("taker").to_str().expect("a path"),
            "--offer",
            offer_file.to_str().expect("a path"),
            "--used",
            used.to_str().expect("a path"),
            "--out",
            second.to_str().expect("a path"),
        ],
        Some(code_text.as_bytes()),
    );
    assert_eq!(out.status.code(), Some(77), "{out:?}");
    assert!(!second.exists(), "the refused accept wrote a record");
}

#[test]
fn an_offer_in_a_spool_is_taken_once() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let giver = key(&dir.path().join("giver"));
    let taker = key(&dir.path().join("taker"));
    let spool = dir.path().join("spool");
    let out = run(
        dir.path(),
        &[
            "offline",
            "pair",
            "offer",
            "--key",
            dir.path().join("giver").to_str().expect("a path"),
            "--spool",
            spool.to_str().expect("a path"),
            "--sequence",
            "0",
        ],
        None,
    );
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let code_text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let code = Code::parse(&code_text).expect("a code");

    let paired_file = dir.path().join("paired");
    let out = run(
        dir.path(),
        &[
            "offline",
            "pair",
            "accept",
            "--key",
            dir.path().join("taker").to_str().expect("a path"),
            "--spool",
            spool.to_str().expect("a path"),
            "--used",
            dir.path().join("used").to_str().expect("a path"),
            "--out",
            paired_file.to_str().expect("a path"),
        ],
        Some(code_text.as_bytes()),
    );
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let paired =
        Paired::decode(&std::fs::read(&paired_file).expect("the record")).expect("a record");
    assert_eq!(paired.identity, taker.public());
    assert_eq!(paired.peer, giver.public());
    assert!(paired.verify(&code));

    // The spool holds no entry for a second accept.
    let out = run(
        dir.path(),
        &[
            "offline",
            "pair",
            "accept",
            "--key",
            dir.path().join("taker").to_str().expect("a path"),
            "--spool",
            spool.to_str().expect("a path"),
            "--used",
            dir.path().join("used-2").to_str().expect("a path"),
            "--out",
            dir.path().join("paired-2").to_str().expect("a path"),
        ],
        Some(code_text.as_bytes()),
    );
    assert_eq!(out.status.code(), Some(66), "{out:?}");
}
