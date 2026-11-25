//! Usage errors of `podnet pod` (issue 62): each bad upstream or rule exits
//! 64, and the program does not run (it would make a marker file).

use std::process::Command;

mod test_host;

/// `podnet pod`: the CLI of the pod is a command of the root binary.
fn pod_command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_podnet"));
    command.arg("pod");
    command
}

#[test]
fn a_bad_upstream_or_rule_exits_64_and_runs_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("ran");
    let cases: &[&[&str]] = &[
        &["-x", "bad"],
        &["-x", "socks4://127.0.0.1:1080"],
        &["-x", "http://"],
        &["-x", "socks5://"],
        &["-x", "https://unix:/x"],
        &["-x", "http://unix:"],
        &["-x", "socks5://127.0.0.1:1", "--rule", "bad"],
        &[
            "-x",
            "socks5://127.0.0.1:1",
            "--rule",
            "domain:example.com=bad",
        ],
    ];
    for case in cases {
        let out = pod_command()
            .args(*case)
            .arg("--")
            .arg("touch")
            .arg(&marker)
            .output()
            .unwrap();
        assert_eq!(
            out.status.code(),
            Some(64),
            "{case:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(!marker.exists(), "{case:?}: the program ran");
    }
}

/// Issue 306: the seccomp backend refuses an inbound directory that the
/// group or others can write, before the program runs.
#[test]
fn an_inbound_directory_that_others_can_write_is_refused() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let open = dir.path().join("open");
    std::fs::create_dir(&open).unwrap();
    std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o777)).unwrap();
    let marker = dir.path().join("ran");
    let out = pod_command()
        .args(["--backend", "seccomp", "--inbound-dir"])
        .arg(&open)
        .args(["-x", "socks5://127.0.0.1:9", "--", "touch"])
        .arg(&marker)
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "{err}");
    assert!(err.contains("writable by the group or by others"), "{err}");
    assert!(!marker.exists(), "the program ran");
}

/// Issue 300: `--serve tcp://127.0.0.1:0` prints the port that the kernel
/// chose, and a client connects to it.
#[test]
fn a_port_0_front_door_prints_the_bound_port() {
    test_host::need_loopback!();
    use std::io::BufRead;
    let mut child = pod_command()
        .args(["--serve", "tcp://127.0.0.1:0", "-x", "socks5://127.0.0.1:9"])
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut lines = std::io::BufReader::new(child.stderr.take().unwrap()).lines();
    let line = lines
        .find_map(|l| l.ok().filter(|l| l.starts_with("podnet pod: serving ")))
        .expect("a serving line");
    let addr: std::net::SocketAddr = line
        .trim_start_matches("podnet pod: serving tcp://")
        .split_whitespace()
        .next()
        .unwrap()
        .parse()
        .unwrap_or_else(|e| panic!("{line}: {e}"));
    let connect = std::net::TcpStream::connect(addr);
    let _ = child.kill();
    let _ = child.wait();
    assert_ne!(addr.port(), 0, "{line}");
    connect.unwrap_or_else(|e| panic!("connect to {addr}: {e}"));
}

// T-135: `podnet pod` names itself: `--version` prints the version line of
// podnet, and each message starts with `podnet pod:`.
#[test]
fn podnet_pod_prints_its_own_program_name() {
    let version = pod_command().arg("--version").output().unwrap();
    assert!(version.status.success());
    let line = String::from_utf8_lossy(&version.stdout).into_owned();
    let top = Command::new(env!("CARGO_BIN_EXE_podnet"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(line.starts_with("podnet "), "{line:?}");
    assert!(!line.contains("podnet-netns"), "{line:?}");
    assert_eq!(
        line,
        String::from_utf8_lossy(&top.stdout),
        "the line of the CLI"
    );

    let error = pod_command()
        .args(["-x", "bad", "--", "true"])
        .output()
        .unwrap();
    assert_eq!(error.status.code(), Some(64));
    let text = String::from_utf8_lossy(&error.stderr).into_owned();
    assert!(text.starts_with("podnet pod: "), "{text:?}");
    assert!(!text.contains("podnet-netns"), "{text:?}");
}

/// GitHub #717: a `--serve` listener that cannot bind exits 69 (as each
/// command with a listener), with one line that names the address one time
/// and gives the next step; not the bare error of the system and exit 3.
#[test]
fn a_front_door_that_cannot_bind_exits_69_with_the_address() {
    test_host::need_loopback!();
    let held = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let busy = format!("tcp://{}", held.local_addr().unwrap());
    let missing = "unix:/nonexistent-podnet-dir/x.sock".to_string();
    for (listen, step) in [
        (&busy, "give another port"),
        (&missing, "the directory of the socket does not exist"),
    ] {
        let out = pod_command()
            .args(["--serve", listen, "-x", "socks5://127.0.0.1:9"])
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(69), "{listen}: {text}");
        assert_eq!(text.lines().count(), 1, "{listen}: one line: {text}");
        assert_eq!(
            text.matches(listen.as_str()).count(),
            1,
            "{listen}: the address one time: {text}"
        );
        assert!(text.contains(step), "{listen}: {text}");
    }
}
