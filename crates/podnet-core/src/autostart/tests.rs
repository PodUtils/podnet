use super::*;

const TOKEN: &str = "eyJhIjoiYSIsInQiOiJ0IiwicyI6InMifQ==";

#[test]
fn the_secrets_leave_the_arguments() {
    let args: Vec<String> = [
        "tunnel",
        "--token",
        TOKEN,
        "--proxy=http://u:pw@proxy:3128",
        "--url",
        "http://127.0.0.1:8080",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let env = |k: &str| (k == "NO_PROXY").then(|| "localhost".to_string());
    let (kept, vars) = split_secrets(&args, &env);
    assert_eq!(kept, ["tunnel", "--url", "http://127.0.0.1:8080"]);
    assert!(vars.contains(&("TUNNEL_TOKEN".into(), TOKEN.into())));
    assert!(vars.contains(&("PODNET_PROXY".into(), "http://u:pw@proxy:3128".into())));
    assert!(vars.contains(&("NO_PROXY".into(), "localhost".into())));
    // A proxy with no password stays on the command line.
    let (kept, _) = split_secrets(
        &["tunnel".into(), "--proxy".into(), "http://p:3128".into()],
        &|_| None,
    );
    assert_eq!(kept, ["tunnel", "--proxy", "http://p:3128"]);
    // The env file reads back; a newline is refused.
    let text = env_text(&vars).unwrap();
    assert_eq!(parse_env(&text), vars);
    assert!(env_text(&[("A".into(), "x\ny".into())]).is_err());
}

#[test]
fn the_unit_and_the_cron_line_hold_no_secret() {
    let line = run_line(
        Path::new("/home/u/.local/bin/podnet"),
        "web",
        Path::new("/home/u/.local/state/podnet/autostart"),
    );
    assert_eq!(
        line,
        "/home/u/.local/bin/podnet autostart run web --dir /home/u/.local/state/podnet/autostart"
    );
    let unit = unit_text("web", &line);
    assert!(unit.starts_with("# podnet autostart web\n"));
    assert!(unit.contains("ExecStart=/home/u/.local/bin/podnet autostart run web"));
    assert!(unit.contains("\nRestartPreventExitStatus=64 78\n"));
    assert!(unit.contains("\nStartLimitIntervalSec=0\n"));
    assert!(!unit.contains(TOKEN));
}

// Issue 265: a unit takes % and $ as its own specifiers.
#[test]
fn the_unit_line_quotes_for_systemd() {
    let line = unit_line(
        Path::new("/opt/c f/podnet"),
        "web",
        Path::new("/tmp/%h/$X/a\"b"),
    );
    assert_eq!(
        line,
        r#""/opt/c f/podnet" autostart run web --dir "/tmp/%%h/$$X/a\"b""#
    );
    let plain = unit_line(Path::new("/b/podnet"), "web", Path::new("/d"));
    assert_eq!(plain, "/b/podnet autostart run web --dir /d");
    let unit = unit_text("web", &line);
    assert!(unit.contains(&format!("\nExecStart={line}\n")), "{unit}");
}

// Issue 265: a program that writes more than a pipe holds ends in time.
#[test]
fn run_limited_reads_a_large_output() {
    let ran = run_limited(
        "sh",
        &[
            "-c",
            "head -c 300000 /dev/zero; head -c 200000 /dev/zero >&2",
        ],
        None,
        Duration::from_secs(20),
    )
    .unwrap();
    assert_eq!(ran.code, Some(0));
    assert_eq!(ran.stdout.len(), 300000);
    assert_eq!(ran.stderr.len(), 200000);
}

#[test]
fn the_crontab_keeps_other_lines() {
    let other = "MAILTO=\"\"\n0 3 * * * backup # podnet autostart webx\n@daily x\n";
    let with = crontab_with(
        other,
        "web",
        "/b/podnet autostart run web --dir /d",
        Path::new("/d/web.log"),
    );
    assert!(with.starts_with(other));
    assert!(with.ends_with(
        "@reboot /b/podnet autostart run web --dir /d >> /d/web.log 2>&1 # podnet autostart web\n"
    ));
    // Again: one line, not two.
    let twice = crontab_with(
        &with,
        "web",
        "/b/podnet autostart run web --dir /d",
        Path::new("/d/web.log"),
    );
    assert_eq!(twice, with);
    assert_eq!(crontab_without(&with, "web"), other);
    // No newline at the end of the old crontab.
    let bare = crontab_with("@daily x", "web", "c", Path::new("/l"));
    assert!(bare.starts_with("@daily x\n@reboot c"));
}

#[test]
fn a_hung_program_ends_at_the_limit() {
    let start = Instant::now();
    let err = run_limited("sleep", &["30"], None, Duration::from_millis(500)).unwrap_err();
    assert!(start.elapsed() < Duration::from_secs(5));
    assert!(format!("{err:#}").contains("did not end"), "{err:#}");
    let ran = run_limited("cat", &[], Some("in"), Duration::from_secs(5)).unwrap();
    assert_eq!(ran.stdout, "in");
}

#[test]
fn names() {
    assert!(check_name("web-1_a").is_ok());
    for bad in ["", "a/b", "a b", "..", &"x".repeat(65)] {
        assert!(check_name(bad).is_err(), "{bad}");
    }
}

/// GitHub #734: a binary under `/var/tmp` (kept across a reboot) or a home
/// is lasting; a binary under `/tmp`, `/dev/shm` or the runtime directory is
/// in a directory that the host clears.
#[test]
fn a_binary_is_refused_only_in_a_directory_that_the_host_clears() {
    let dirs = [
        PathBuf::from("/tmp"),
        PathBuf::from("/dev/shm"),
        PathBuf::from("/run/user/1000"),
    ];
    let cases = [
        ("/var/tmp/home/.local/bin/podnet", None),
        ("/home/u/.local/bin/podnet", None),
        ("/usr/local/bin/podnet", None),
        ("/tmp/x/podnet", Some("/tmp")),
        ("/dev/shm/podnet", Some("/dev/shm")),
        ("/run/user/1000/podnet", Some("/run/user/1000")),
    ];
    for (exe, want) in cases {
        let got = cleared_dir(Path::new(exe), &dirs);
        assert_eq!(got.as_deref(), want.map(Path::new), "{exe}");
    }
    assert!(!cleared_dirs().contains(&PathBuf::from("/var/tmp")));
}
