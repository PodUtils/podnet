//! The names of the instances: the run directory, `ps`, `stop` and the locks.

use super::*;

#[test]
fn stop_logs_and_wait() {
    let env = Env::new();
    let switch = env.switch();
    let start = env.run(&[
        "net", "switch", "--listen", &switch, "--detach", "--name", "w",
    ]);
    assert!(start.status.success());
    let pid = ready_doc(&start)["pid"].as_i64().unwrap() as i32;
    let wait = env.run(&["wait", "w", "--timeout", "5"]);
    assert!(wait.status.success());
    assert_eq!(ready_doc(&wait), ready_doc(&start));
    let logs = env.run(&["logs", "w"]);
    assert!(logs.status.success());
    let stop = env.run(&["stop", "w", "--timeout", "5"]);
    assert!(
        stop.status.success(),
        "{}",
        String::from_utf8_lossy(&stop.stderr)
    );
    wait_until("the instance ends", || !alive(pid));
    assert!(env.ps()["instances"].as_array().unwrap().is_empty());
    assert_eq!(env.run(&["stop", "w"]).status.code(), Some(69));
    assert_eq!(
        env.run(&["wait", "w", "--timeout", "1"]).status.code(),
        Some(69)
    );
}

#[test]
fn detach_options_need_detach_and_other_commands_refuse_it() {
    let env = Env::new();
    // Issue 209: --name alone is valid; --replace and --no-restart alone
    // are not.
    assert_eq!(
        env.run(&["net", "switch", "--replace"]).status.code(),
        Some(64)
    );
    assert_eq!(
        env.run(&["net", "switch", "--no-restart"]).status.code(),
        Some(64)
    );
    assert_eq!(
        env.run(&["net", "switch", "--name", "x", "--replace"])
            .status
            .code(),
        Some(64)
    );
    assert_eq!(env.run(&["status", "--detach"]).status.code(), Some(64));
    assert_eq!(
        env.run(&["--detach-child", "status"]).status.code(),
        Some(64)
    );
}

// Issue 209: --name with no --detach registers the foreground process:
// ps, logs, wait and stop see it; a second process with the name fails.
#[test]
fn a_named_foreground_process_is_in_ps_and_stops() {
    let env = Env::new();
    let switch = env.switch();
    let mut child = env
        .cmd(&["net", "switch", "--listen", &switch, "--name", "fg"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    wait_until("the record is ready", || {
        env.ps()["instances"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["name"] == "fg" && i["state"] == "ready")
    });
    let entry = instance(&env, "fg");
    assert_eq!(entry["pid"].as_i64(), Some(child.id() as i64), "{entry}");
    assert!(entry["supervisor"].is_null(), "{entry}");
    let wait = env.run(&["wait", "fg", "--timeout", "5"]);
    assert!(wait.status.success());
    let second = env.run(&[
        "net",
        "switch",
        "--listen",
        "unix:/nonexistent/x",
        "--name",
        "fg",
    ]);
    assert_ne!(second.status.code(), Some(0));
    assert!(
        String::from_utf8_lossy(&second.stderr).contains("runs already"),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    let logs = env.run(&["logs", "fg"]);
    assert!(logs.status.success());
    let stop = env.run(&["stop", "fg", "--timeout", "5"]);
    assert!(
        stop.status.success(),
        "{}",
        String::from_utf8_lossy(&stop.stderr)
    );
    let status = child.wait().unwrap();
    assert_eq!(status.code(), Some(0), "{status:?}");
    assert!(env.ps()["instances"].as_array().unwrap().is_empty());
}

// Issue 182: wait for a name that never existed says so.
#[test]
fn wait_for_a_name_that_never_existed_says_so() {
    let env = Env::new();
    let out = env.run(&["wait", "nosuch", "--timeout", "1"]);
    assert_eq!(out.status.code(), Some(69));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("no instance nosuch"), "{err}");
}

// Issue 260: logs, stop and wait check the name before they make a path. A
// name with a slash or a dot cannot read, lock or remove a file outside the
// run directory.
#[test]
fn logs_stop_and_wait_refuse_a_name_that_leaves_the_run_directory() {
    let env = Env::new();
    std::fs::create_dir_all(&env.run).unwrap();
    let outer = env.run.parent().unwrap().join("outer");
    std::fs::create_dir(&outer).unwrap();
    std::fs::write(outer.join("secret.log"), "SECRET\n").unwrap();
    std::fs::write(outer.join("v.json"), "{}").unwrap();
    std::fs::write(outer.join("v.lock"), "").unwrap();
    let before = listing(&outer);
    let abs = format!("{}/v", outer.display());
    for name in [
        "../outer/secret",
        "../outer/v",
        "../outer/w",
        abs.as_str(),
        "a/b",
        ".hidden",
    ] {
        for args in [
            vec!["logs", name],
            vec!["stop", name, "--timeout", "1"],
            vec!["wait", name, "--timeout", "1"],
        ] {
            let out = env.run(&args);
            let stderr = String::from_utf8_lossy(&out.stderr);
            assert_eq!(out.status.code(), Some(64), "{args:?}: {stderr}");
            assert!(stderr.contains("instance name"), "{args:?}: {stderr}");
            assert!(
                !String::from_utf8_lossy(&out.stdout).contains("SECRET"),
                "{args:?}"
            );
        }
    }
    assert_eq!(listing(&outer), before, "the outer directory is unchanged");
    assert!(listing(&env.run).is_empty(), "{:?}", listing(&env.run));
}

// Issue 262: the record of a foreground --name had the raw command line: the
// proxy password was in the record and in ps --json.
#[test]
fn a_named_foreground_process_keeps_no_secret_in_its_record() {
    let env = Env::new();
    let switch = env.switch();
    let mut child = env
        .cmd(&[
            "--proxy",
            "http://u:SECRETPW@127.0.0.1:9",
            "net",
            "switch",
            "--listen",
            &switch,
            "--name",
            "fgs",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    wait_until("the record is ready", || {
        env.ps()["instances"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["name"] == "fgs" && i["state"] == "ready")
    });
    let ps = env.run(&["ps", "--json"]);
    let ps = String::from_utf8_lossy(&ps.stdout);
    assert!(!ps.contains("SECRETPW"), "{ps}");
    let entry = instance(&env, "fgs");
    assert!(entry["args"].to_string().contains("switch"), "{entry}");
    let record = std::fs::read_to_string(env.run.join("fgs.json")).unwrap();
    assert!(!record.contains("SECRETPW"), "{record}");
    let record: serde_json::Value = serde_json::from_str(&record).unwrap();
    assert_eq!(record["hash"].as_str().map(str::len), Some(16), "{record}");
    let stop = env.run(&["stop", "fgs", "--timeout", "5"]);
    assert!(
        stop.status.success(),
        "{}",
        String::from_utf8_lossy(&stop.stderr)
    );
    let _ = child.wait();
}

// Issue 264: stop on a record that does not parse removed the locks of the
// live instance; the instance ran on, hidden, and its name was free.
#[test]
fn stop_keeps_the_locks_of_a_live_instance_with_a_bad_record() {
    let env = Env::new();
    let switch = env.switch();
    let args = [
        "net", "switch", "--listen", &switch, "--detach", "--name", "cor",
    ];
    let start = env.run(&args);
    assert!(
        start.status.success(),
        "{}",
        String::from_utf8_lossy(&start.stderr)
    );
    let pid = ready_doc(&start)["pid"].as_i64().unwrap() as i32;
    let record = env.run.join("cor.json");
    let saved = std::fs::read(&record).unwrap();
    std::fs::write(&record, "").unwrap();
    let stop = env.run(&["stop", "cor", "--timeout", "2"]);
    let err = String::from_utf8_lossy(&stop.stderr);
    assert_eq!(stop.status.code(), Some(69), "{err}");
    assert!(err.contains("cannot be read"), "{err}");
    assert!(env.run.join("cor.lock").exists(), "the lock stays");
    assert!(alive(pid));
    let entry = instance(&env, "cor");
    assert_eq!(entry["state"], "unknown", "{entry}");
    let again = env.run(&args);
    assert_ne!(again.status.code(), Some(0), "a second instance started");
    // The record back: the instance stops as usual.
    std::fs::write(&record, saved).unwrap();
    let stop = env.run(&["stop", "cor", "--timeout", "5"]);
    assert!(
        stop.status.success(),
        "{}",
        String::from_utf8_lossy(&stop.stderr)
    );
    wait_until("the instance ends", || !alive(pid));
}
