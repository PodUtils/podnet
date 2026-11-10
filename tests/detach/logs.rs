//! The logs and the records of an instance.

use super::*;

#[test]
fn no_secret_in_the_arguments_of_the_child_or_in_the_record() {
    let env = Env::new();
    let switch = env.switch();
    let start = env.run(&[
        "--proxy",
        "http://user:hunter2@127.0.0.1:9",
        "net",
        "switch",
        "--listen",
        &switch,
        "--detach",
        "--name",
        "sec",
    ]);
    assert!(
        start.status.success(),
        "{}",
        String::from_utf8_lossy(&start.stderr)
    );
    let pid = ready_doc(&start)["pid"].as_i64().unwrap();
    let supervisor = env.ps()["instances"][0]["supervisor"].as_i64().unwrap();
    for pid in [pid, supervisor] {
        let cmdline = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap();
        assert!(!String::from_utf8_lossy(&cmdline).contains("hunter2"));
    }
    let record = std::fs::read_to_string(env.run.join("sec.json")).unwrap();
    assert!(!record.contains("hunter2"));
    use std::os::unix::fs::PermissionsExt as _;
    for file in ["sec.json", "sec.log"] {
        let mode = std::fs::metadata(env.run.join(file))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "{file}");
    }
}

/// Issue 103: the log of an instance has its startup lines (before the
/// ready) after a separator with its pid; a second start of the same name
/// adds a second separator, and its lines come after it.
#[test]
fn the_log_has_the_startup_lines_of_this_start() {
    let env = Env::new();
    let switch = env.switch();
    for run in 1..=2 {
        let start = env.run(&[
            "net", "switch", "--listen", &switch, "--detach", "--name", "l",
        ]);
        assert!(
            start.status.success(),
            "{}",
            String::from_utf8_lossy(&start.stderr)
        );
        let pid = ready_doc(&start)["pid"].as_i64().unwrap();
        let logs = env.run(&["logs", "l"]);
        let text = String::from_utf8_lossy(&logs.stdout).into_owned();
        assert_eq!(text.matches("---- start").count(), run, "{text}");
        let lines: Vec<&str> = text.lines().collect();
        let last = lines
            .iter()
            .rposition(|l| l.contains("---- start"))
            .unwrap();
        let tail = &lines[last..];
        assert!(tail[0].contains(&format!("pid {pid} ")), "{text}");
        let listening = tail
            .iter()
            .filter(|l| l.contains("switch is listening"))
            .count();
        assert_eq!(listening, 1, "the line of this start, one time: {text}");
        assert!(!text.contains('\u{1b}'), "no colour codes: {text}");
        let stop = env.run(&["stop", "l", "--timeout", "5"]);
        assert!(stop.status.success());
        wait_until("the instance ends", || !alive(pid as i32));
    }
}

// Issue 263: stdout of --detach is one JSON document; the human lines of the
// child go to stderr.
#[test]
fn detach_prints_only_the_ready_document_on_stdout() {
    let env = Env::new();
    let switch = env.switch();
    let start = env.run(&[
        "net", "switch", "--listen", &switch, "--detach", "--name", "j1",
    ]);
    assert!(
        start.status.success(),
        "{}",
        String::from_utf8_lossy(&start.stderr)
    );
    let stdout = String::from_utf8_lossy(&start.stdout);
    assert_eq!(stdout.lines().count(), 1, "{stdout:?}");
    serde_json::from_str::<serde_json::Value>(stdout.trim())
        .unwrap_or_else(|e| panic!("{e}: {stdout:?}"));
    let stderr = String::from_utf8_lossy(&start.stderr);
    assert!(stderr.contains("podnet: switch"), "{stderr}");
}
