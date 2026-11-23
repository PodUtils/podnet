//! LOG-1 to LOG-3: a deliberately panicking child writes a bounded crash
//! log with the backtrace, the versions and the last events, and redacts a
//! seeded value at write time.
//!
//! The log of each child is read from the file, not from the log line of
//! the process, and the status is read from the child directly.

use std::process::{Command, Output};

use podnet::crashlog;

const SECRET: &str = "seed-crash-9f3a2b7c";

fn child(dir: &std::path::Path, max: u64, slots: usize) -> Output {
    Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(["host", "panic", "--seed", SECRET])
        .env("PODNET_CRASH_LOG_DIR", dir)
        .env("PODNET_CRASH_LOG_MAX", max.to_string())
        .env("PODNET_CRASH_LOG_SLOTS", slots.to_string())
        .env_remove("PODNET_STATE_DIR")
        .output()
        .expect("the child runs")
}

#[test]
fn a_panic_writes_the_backtrace_the_versions_and_the_events() {
    let dir = tempfile::tempdir().unwrap();
    let out = child(dir.path(), 64 * 1024, 1);
    assert_eq!(
        out.status.code(),
        Some(101),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let path = crashlog::path(dir.path(), 0);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.starts_with("podnet crash log\n"), "{text}");
    assert!(text.contains("format: podnet.crash/1"), "{text}");
    assert!(text.contains("components:"), "{text}");
    assert!(text.contains("podnet:"), "{text}");
    assert!(text.contains("podcrates:"), "{text}");
    assert!(text.contains("backtrace:"), "{text}");
    assert!(text.contains("last events ("), "{text}");
    assert!(text.contains("host panic"), "{text}");
    assert!(
        text.contains("redaction: on ("),
        "the marker of the redaction is absent: {text}"
    );
    assert!(
        !text.contains(SECRET),
        "the seeded value is in the crash log: {text}"
    );
    assert!(
        text.contains("<hidden>"),
        "the value was removed with no mask: {text}"
    );
}

/// LOG-3: three crashes with two slots of 4 KiB keep two files, each under
/// the bound.
#[test]
fn the_crash_log_is_bounded_and_rotated() {
    let dir = tempfile::tempdir().unwrap();
    for _ in 0..3 {
        let out = child(dir.path(), 4096, 2);
        assert_eq!(out.status.code(), Some(101));
    }
    let files = crashlog::files(dir.path(), 2);
    assert!(!files.is_empty(), "no crash file");
    assert!(files.len() <= 2, "{files:?}");
    for (path, size) in &files {
        assert!(*size <= 4096, "{}: {size} bytes", path.display());
    }
    assert!(!crashlog::path(dir.path(), 2).exists());
}
