//! The capture record of PodUtils/podnet#499: one format, a merge ordered
//! by the timestamp, and redaction at write time.
//!
//! Two captures carry records out of order, and a third file is written by
//! hand with the secret still in it, to prove that the merge redacts again.
//! The merged file holds no secret, holds the marker, and is the same bytes
//! when the merge runs again.

use std::path::{Path, PathBuf};
use std::process::Command;

use podnet::offline::capture::{self, Capture, Direction, MARKER};

const SECRET: &str = "s3cr3t-token";

fn podnet(dir: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_podnet"));
    cmd.args(args)
        .env("PODNET_STATE_DIR", dir.join("state"))
        .env("PODNET_RUN_DIR", dir.join("run"))
        .env_remove("RUST_LOG");
    cmd
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[test]
fn a_merge_is_ordered_redacted_and_stable() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let a = dir.path().join("a.cap");
    let b = dir.path().join("b.cap");
    let c = dir.path().join("c.cap");
    let out = dir.path().join("merged.cap");
    let again = dir.path().join("merged-again.cap");
    let secret = SECRET.as_bytes().to_vec();

    {
        let mut capture = Capture::create(&a, vec![secret.clone()]).expect("a capture");
        assert!(capture
            .record(
                20,
                Direction::In,
                format!("before {SECRET} after").as_bytes()
            )
            .expect("a record"));
        assert!(!capture
            .record(10, Direction::Out, b"hello")
            .expect("a record"));
        assert!(!capture
            .record(10, Direction::Out, b"again")
            .expect("a record"));
    }
    {
        let mut capture = Capture::create(&b, vec![secret.clone()]).expect("a capture");
        assert!(!capture
            .record(10, Direction::In, b"other at ten")
            .expect("a record"));
        assert!(!capture
            .record(30, Direction::Out, b"last")
            .expect("a record"));
    }
    // A file from another tool, with no redaction of its own.
    std::fs::write(
        &c,
        format!(
            "{}\n25 out 0 {}\n",
            capture::HEADER,
            hex(format!("raw {SECRET}").as_bytes())
        ),
    )
    .expect("a capture");

    for path in [&a, &b] {
        let bytes = std::fs::read(path).expect("a capture");
        assert!(
            !bytes.windows(SECRET.len()).any(|w| w == SECRET.as_bytes()),
            "{} holds the secret",
            path.display()
        );
    }

    let merge = |out: &PathBuf| {
        podnet(
            dir.path(),
            &[
                "offline",
                "capture",
                "merge",
                a.to_str().expect("a path"),
                b.to_str().expect("a path"),
                c.to_str().expect("a path"),
                "--out",
                out.to_str().expect("a path"),
                "--secret",
                SECRET,
            ],
        )
        .output()
        .expect("capture merge")
    };
    let first = merge(&out);
    assert_eq!(first.status.code(), Some(0), "{first:?}");
    let said = String::from_utf8_lossy(&first.stdout);
    assert!(said.contains("merged 6 record(s)"), "{said:?}");
    assert!(said.contains("2 marked redacted"), "{said:?}");

    let bytes = std::fs::read(&out).expect("the merge");
    assert!(
        !bytes.windows(SECRET.len()).any(|w| w == SECRET.as_bytes()),
        "the merge holds the secret"
    );
    let merged = capture::records(&out).expect("the merge");
    assert_eq!(merged.len(), 6);
    assert!(merged.windows(2).all(|w| w[0].timestamp <= w[1].timestamp));
    assert_eq!(merged[0].timestamp, 10);
    assert_eq!(merged[0].bytes, b"hello");
    assert_eq!(merged[1].bytes, b"again");
    assert_eq!(merged[2].bytes, b"other at ten");
    assert_eq!(merged[3].timestamp, 20);
    assert_eq!(merged[4].timestamp, 25);
    assert!(
        merged[4].bytes.windows(MARKER.len()).any(|w| w == MARKER),
        "the hand-written secret was not replaced: {:?}",
        String::from_utf8_lossy(&merged[4].bytes)
    );
    assert_eq!(merged[5].timestamp, 30);

    let second = merge(&again);
    assert_eq!(second.status.code(), Some(0), "{second:?}");
    assert_eq!(
        std::fs::read(&out).expect("the first merge"),
        std::fs::read(&again).expect("the second merge"),
        "the two merges differ"
    );
}
