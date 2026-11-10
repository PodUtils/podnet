//! `podnet warp` commands.

use super::*;

/// warp status, license, rotate and delete use a saved device and register
/// none.
#[test]
fn warp_status_registers_no_device() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("none.json");
    for command in ["status", "rotate", "delete"] {
        let out = podnet(&["warp", "--identity", path.to_str().unwrap(), command]);
        assert_eq!(out.status.code(), Some(78), "{command}: {}", stderr(&out));
        assert!(
            stderr(&out).contains("no WARP identity at"),
            "{}",
            stderr(&out)
        );
        assert!(!path.exists(), "{command} made a file");
    }
}

/// Issue 130: warp ssh-config --write, --check and --remove own one block
/// of the file and change no other byte.
#[test]
fn warp_ssh_config_owns_its_block() {
    let dir = tempfile::tempdir().unwrap();
    let conf = dir.path().join("config");
    let mine = "Host box\n  User me\n";
    std::fs::write(&conf, mine).unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_podnet"))
            .args(["warp", "ssh-config", "--config", conf.to_str().unwrap()])
            .args(args)
            .env_remove("PODNET_ERROR_PROGRAM")
            .output()
            .unwrap()
    };
    assert_eq!(
        run(&["--check", "--host", "github.com"]).status.code(),
        Some(1)
    );
    assert_eq!(
        run(&["--write", "--host", "github.com"]).status.code(),
        Some(0)
    );
    let text = std::fs::read_to_string(&conf).unwrap();
    assert!(
        text.starts_with("# >>> podnet warp ssh-config: github.com >>>\nHost github.com\n"),
        "{text}"
    );
    assert!(text.ends_with(mine), "{text}");
    assert_eq!(
        run(&["--check", "--host", "github.com"]).status.code(),
        Some(0)
    );
    assert_eq!(run(&["--write"]).status.code(), Some(64));
    assert_eq!(run(&["--write", "--host", "box"]).status.code(), Some(1));
    assert_eq!(run(&["--remove"]).status.code(), Some(0));
    assert_eq!(std::fs::read_to_string(&conf).unwrap(), mine);
}

// Issue 252: a --host with a newline wrote an own Host block with a
// ProxyCommand into the SSH config.
#[test]
fn ssh_config_refuses_a_host_with_a_newline_or_a_space() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config");
    std::fs::write(&config, "Host box\n    User me\n").unwrap();
    let config = config.to_str().unwrap();
    for host in ["x\nHost *\n    ProxyCommand evil\n#", "a b", "a\tb", ""] {
        for extra in [&["--write", "--config", config][..], &[]] {
            let mut args = vec!["warp", "ssh-config", "--host", host];
            args.extend_from_slice(extra);
            let out = podnet(&args);
            assert_eq!(out.status.code(), Some(64), "{host:?}: {}", stderr(&out));
            assert!(out.stdout.is_empty(), "{host:?}");
        }
    }
    assert_eq!(
        std::fs::read_to_string(config).unwrap(),
        "Host box\n    User me\n"
    );
}

// Issue 255: warp env with no proxy variables (warp up on the socket only)
// exits 69 and prints nothing on stdout.
#[test]
fn warp_env_with_no_proxy_variables_exits_69() {
    let dir = tempfile::tempdir().unwrap();
    let run_dir = dir.path().join("run");
    std::fs::create_dir(&run_dir).unwrap();
    std::fs::write(
        run_dir.join("warp.env"),
        "# podnet warp up\nPODNET_WARP_SOCKET=/x\n",
    )
    .unwrap();
    // The lock of a live warp up: a child holds it.
    let lock = run_dir.join("warp.lock");
    std::fs::write(&lock, "").unwrap();
    let mut holder = Command::new("flock")
        .arg("-o")
        .arg(&lock)
        .args(["sleep", "30"])
        .spawn()
        .unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while Command::new("flock")
        .args(["-n"])
        .arg(&lock)
        .arg("true")
        .status()
        .unwrap()
        .success()
    {
        assert!(std::time::Instant::now() < until, "the child holds no lock");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let out = Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(["warp", "env"])
        .env("PODNET_RUN_DIR", &run_dir)
        .env_remove("PODNET_ERROR_PROGRAM")
        .output()
        .unwrap();
    let _ = holder.kill();
    let _ = holder.wait();
    assert_eq!(out.status.code(), Some(69), "{}", stderr(&out));
    assert!(
        out.stdout.is_empty(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(
        stderr(&out).contains("no proxy variables"),
        "{}",
        stderr(&out)
    );
}
