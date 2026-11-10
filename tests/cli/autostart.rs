//! Autostart, the soft limit of open files and the stop by a signal.

use super::*;

/// `podnet autostart` with a fake systemctl (no user manager) and a fake
/// crontab (issue 23): the crontab line holds no secret, other lines stay,
/// the env file is private, and with no way at all the exit is 69.
#[test]
fn autostart_with_cron_and_with_nothing() {
    use std::os::unix::fs::PermissionsExt;
    const TOKEN: &str = "eyJhIjoiYWNjb3VudCIsInQiOiJ0dW5uZWwiLCJzIjoic2VjcmV0In0=";
    let dir = tempfile::tempdir().unwrap();
    let fake = dir.path().join("bin");
    std::fs::create_dir(&fake).unwrap();
    let script = |name: &str, body: &str| {
        let path = fake.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    };
    script("systemctl", "echo 'Failed to connect to bus' >&2; exit 1");
    script(
        "crontab",
        r#"f="$FAKE_CRONTAB"
if [ "$1" = "-l" ]; then if [ -f "$f" ]; then cat "$f"; exit 0; fi; echo "no crontab for user" >&2; exit 1; fi
if [ "$1" = "-" ]; then cat > "$f"; exit 0; fi
exit 2"#,
    );
    let crontab = dir.path().join("crontab");
    std::fs::write(&crontab, "MAILTO=\"\"\n@daily /usr/bin/true # mine\n").unwrap();
    let state = dir.path().join("state");
    let run = |args: &[&str], path: &str| {
        Command::new(env!("CARGO_BIN_EXE_podnet"))
            .args(args)
            .env("PATH", path)
            .env("HOME", dir.path())
            .env("PODNET_STATE_DIR", &state)
            .env("FAKE_CRONTAB", &crontab)
            .env("TUNNEL_TOKEN", TOKEN)
            .env_remove("PODNET_ERROR_PROGRAM")
            .env_remove("XDG_CONFIG_HOME")
            .output()
            .unwrap()
    };
    let path = format!("{}:/usr/bin:/bin", fake.display());
    let out = run(
        &[
            "autostart",
            "add",
            "web",
            "--",
            "status",
            "--proxy=http://u:pw@127.0.0.1:9",
        ],
        &path,
    );
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), stderr(&out));
    assert_eq!(out.status.code(), Some(0), "{text}");
    assert!(text.contains("added (cron)"), "{text}");
    let cron = std::fs::read_to_string(&crontab).unwrap();
    assert!(
        cron.starts_with("MAILTO=\"\"\n@daily /usr/bin/true # mine\n"),
        "{cron}"
    );
    assert!(
        cron.contains("@reboot ") && cron.contains(" autostart run web --dir "),
        "{cron}"
    );
    assert!(
        cron.trim_end().ends_with("# podnet autostart web"),
        "{cron}"
    );
    assert!(!cron.contains(TOKEN) && !cron.contains("pw@"), "{cron}");
    let entry = state.join("autostart");
    let env = std::fs::read_to_string(entry.join("web.env")).unwrap();
    assert!(env.contains(&format!("TUNNEL_TOKEN={TOKEN}")), "{env}");
    assert!(
        env.contains("PODNET_PROXY=http://u:pw@127.0.0.1:9"),
        "{env}"
    );
    let mode = std::fs::metadata(entry.join("web.env"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
    let args = std::fs::read_to_string(entry.join("web.args")).unwrap();
    assert_eq!(args, r#"["status"]"#);

    // Issue 117: a leading podnet is dropped; a command that does not parse
    // is exit 64, and no entry is written.
    let out = run(
        &["autostart", "add", "web2", "--", "podnet", "status"],
        &path,
    );
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let args = std::fs::read_to_string(entry.join("web2.args")).unwrap();
    assert_eq!(args, r#"["status"]"#);
    let out = run(&["autostart", "remove", "web2"], &path);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let out = run(
        &["autostart", "add", "web3", "--", "no-such-command"],
        &path,
    );
    assert_eq!(out.status.code(), Some(64), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("is not a podnet command"),
        "{}",
        stderr(&out)
    );
    assert!(!entry.join("web3.args").exists());
    let out = run(&["autostart", "list"], &path);
    assert_eq!(String::from_utf8_lossy(&out.stdout), "web\n");

    let out = run(&["autostart", "remove", "web"], &path);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let cron = std::fs::read_to_string(&crontab).unwrap();
    assert_eq!(cron, "MAILTO=\"\"\n@daily /usr/bin/true # mine\n");
    assert!(!entry.join("web.env").exists() && !entry.join("web.args").exists());

    // Issue 117: a crontab that takes the input and keeps nothing: exit 69.
    let lossy = dir.path().join("lossy");
    std::fs::create_dir(&lossy).unwrap();
    let path_lossy = format!("{}:/usr/bin:/bin", lossy.display());
    std::fs::copy(fake.join("systemctl"), lossy.join("systemctl")).unwrap();
    let crontab_lossy = lossy.join("crontab");
    std::fs::write(
        &crontab_lossy,
        "#!/bin/sh\n[ \"$1\" = \"-\" ] && cat > /dev/null\nexit 0\n",
    )
    .unwrap();
    std::fs::set_permissions(&crontab_lossy, std::fs::Permissions::from_mode(0o755)).unwrap();
    let out = run(&["autostart", "add", "web4", "--", "status"], &path_lossy);
    assert_eq!(out.status.code(), Some(69), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("does not keep the entry"),
        "{}",
        stderr(&out)
    );

    // No systemctl and no crontab: exit 69, and the line to run.
    let empty = dir.path().join("empty");
    std::fs::create_dir(&empty).unwrap();
    let out = run(
        &["autostart", "add", "web", "--", "status"],
        empty.to_str().unwrap(),
    );
    assert_eq!(out.status.code(), Some(69), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("autostart run web --dir"),
        "{}",
        stderr(&out)
    );
    assert!(!stderr(&out).contains(TOKEN));
}

// Issue 265: autostart add --from NAME copies the secrets of the instance
// (its record has none), and explicit arguments lose the options of a
// detached start.
#[test]
fn autostart_from_an_instance_keeps_its_secrets_and_drops_detach_options() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    let run_dir = dir.path().join("run");
    // No systemctl and no crontab: add writes the entry, then exits 69.
    let empty = dir.path().join("empty");
    std::fs::create_dir(&empty).unwrap();
    let listen = format!("unix:{}", dir.path().join("s.sock").display());
    let podnet = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_podnet"))
            .args(args)
            .env("PATH", &empty)
            .env("HOME", dir.path())
            .env("PODNET_STATE_DIR", &state)
            .env("PODNET_RUN_DIR", &run_dir)
            .env_remove("TUNNEL_TOKEN")
            .env_remove("PODNET_PROXY")
            .env_remove("PODNET_ERROR_PROGRAM")
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("RUST_LOG")
            .output()
            .unwrap()
    };
    let start = podnet(&[
        "--proxy",
        "http://u:pw265@127.0.0.1:9",
        "net",
        "switch",
        "--listen",
        &listen,
        "--detach",
        "--name",
        "src",
    ]);
    assert!(start.status.success(), "{}", stderr(&start));
    let out = podnet(&["autostart", "add", "x", "--from", "src"]);
    let stopped = podnet(&["stop", "src", "--timeout", "5"]);
    assert!(stopped.status.success(), "{}", stderr(&stopped));
    assert_eq!(out.status.code(), Some(69), "{}", stderr(&out));
    let entry = state.join("autostart");
    let env = std::fs::read_to_string(entry.join("x.env")).unwrap();
    assert!(
        env.contains("PODNET_PROXY=http://u:pw265@127.0.0.1:9"),
        "{env}"
    );
    let mode = std::fs::metadata(entry.join("x.env"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
    let args = std::fs::read_to_string(entry.join("x.args")).unwrap();
    assert!(!args.contains("pw265"), "{args}");
    assert!(args.contains("switch"), "{args}");
    assert!(
        !args.contains("--detach") && !args.contains("--name"),
        "{args}"
    );
    assert!(
        !run_dir.join("src.secrets").exists(),
        "stop removes the secrets"
    );

    let out = podnet(&[
        "autostart",
        "add",
        "y",
        "--",
        "net",
        "switch",
        "--listen",
        &listen,
        "--detach",
        "--name",
        "inner",
    ]);
    assert_eq!(out.status.code(), Some(69), "{}", stderr(&out));
    assert!(stderr(&out).contains("are removed"), "{}", stderr(&out));
    let args = std::fs::read_to_string(entry.join("y.args")).unwrap();
    assert!(
        !args.contains("--detach") && !args.contains("inner"),
        "{args}"
    );
}

/// Issue 126: podnet raises the soft limit of open files to the hard limit,
/// and `status` shows it. `warp exec` leaves it as it is (it runs a program
/// of the user).
#[test]
#[allow(clippy::unnecessary_cast)]
fn the_soft_limit_of_open_files_is_raised() {
    use std::os::unix::process::CommandExt;
    let status_with = |soft: u64, hard: u64| {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_podnet"));
        cmd.arg("status").env("PODNET_STATE_DIR", "none");
        // SAFETY: setrlimit is async-signal-safe.
        unsafe {
            cmd.pre_exec(move || {
                let limit = libc::rlimit {
                    rlim_cur: soft as libc::rlim_t,
                    rlim_max: hard as libc::rlim_t,
                };
                if libc::setrlimit(libc::RLIMIT_NOFILE, &limit) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let out = cmd.output().unwrap();
        assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
        let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        doc["nofile"].clone()
    };
    let mut hard = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: getrlimit writes one rlimit.
    unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut hard) };
    let hard = (hard.rlim_max as u64).min(4096);
    if hard <= 256 {
        eprintln!("skipping: the hard limit is {hard}");
        return;
    }
    let nofile = status_with(256, hard);
    assert_eq!(nofile["soft"], hard, "{nofile}");
    assert_eq!(nofile["raised_from"], 256, "{nofile}");
    let nofile = status_with(hard, hard);
    assert_eq!(nofile["soft"], hard, "{nofile}");
    assert!(nofile["raised_from"].is_null(), "{nofile}");
}

/// SIGTERM stops podnet as SIGINT does: a clean shutdown and exit 0.
#[test]
fn sigterm_is_a_clean_stop() {
    test_host::need_loopback!();
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    for signal in [libc::SIGTERM, libc::SIGINT] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_podnet"))
            .args([
                "connect",
                "-L",
                &format!("tcp://127.0.0.1:{port}"),
                "wss://127.0.0.1:9/__podnet/ws",
                "--no-check",
            ])
            .env_remove("PODNET_ERROR_PROGRAM")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        // Wait for the listener.
        let start = std::time::Instant::now();
        while std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
            assert!(
                start.elapsed() < std::time::Duration::from_secs(20),
                "no listener"
            );
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        unsafe { libc::kill(child.id() as i32, signal) };
        let status = child.wait().unwrap();
        assert_eq!(status.code(), Some(0), "signal {signal}: {status:?}");
    }
}
