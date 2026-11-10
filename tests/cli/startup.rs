//! The check mode, the doctor, the start of each command and the environment.

use super::*;

/// With an empty environment (no HOME, no XDG variables, no user name),
/// the commands that need no network run, and the state directory is in
/// the temporary directory, never in the current directory.
#[test]
fn an_empty_environment_works() {
    let dir = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_podnet"))
            .args(args)
            .env_clear()
            .env("TMPDIR", dir.path())
            .current_dir(dir.path())
            .output()
            .unwrap()
    };
    for args in [
        &["--version"][..],
        &["man", "--no-pager", "files"],
        &["ports"],
        &["net", "addresses"],
    ] {
        let out = run(args);
        assert_eq!(
            out.status.code(),
            Some(0),
            "podnet {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let out = run(&["status"]);
    assert_eq!(out.status.code(), Some(0));
    let status: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let path = status["warp_identity"]["path"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(path.starts_with(dir.path().to_str().unwrap()), "{path}");
    assert!(path.contains("/podnet-"), "{path}");
    assert!(!dir.path().join("podnet-warp.json").exists());

    let out = Command::new(env!("CARGO_BIN_EXE_podnet"))
        .arg("status")
        .env_clear()
        .env("PODNET_STATE_DIR", "none")
        .current_dir(dir.path())
        .output()
        .unwrap();
    let status: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(status["warp_identity"]["reason"], "PODNET_STATE_DIR=none");
}

/// `--check` (issue 45): exit 0 or 1, no call to the API, no file written,
/// no token in the output.
#[test]
fn check_mode() {
    test_host::need_loopback!();
    const TOKEN: &str = "eyJhIjogIjAxMjM0NTY3ODlhYmNkZWYwMTIzNDU2Nzg5YWJjZGVmIiwgInQiOiAiZmZkODZiZDUtMWJmZi00NTZiLThjMmMtNzVhMDE0Y2RjNGM4IiwgInMiOiAiQUFFQ0F3UUZCZ2NJQ1FvTERBME9EeEFSRWhNVUZSWVhHQmthR3h3ZEhoOD0ifQ==";
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    std::fs::create_dir(&state).unwrap();
    let site = dir.path().join("site");
    std::fs::create_dir(&site).unwrap();
    // A stand-in for the quick-tunnel API: it must get no connection.
    let api = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    api.set_nonblocking(true).unwrap();
    let service_url = format!("http://{}", api.local_addr().unwrap());
    let closed = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port().to_string()
    };
    let run = |args: &[&str], env: &[(&str, &str)]| {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_podnet"));
        cmd.args(args)
            .args(["--service-url", &service_url])
            .env_remove("PODNET_ERROR_PROGRAM")
            .env_remove("PODNET_EDGE")
            .env_remove("TUNNEL_TOKEN")
            .env("PODNET_STATE_DIR", &state);
        for (k, v) in env {
            cmd.env(k, v);
        }
        let out = cmd.output().unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        (out.status.code(), text, out.stdout)
    };
    let site_arg = site.to_str().unwrap();

    let (code, text, _) = run(&["tunnel", "--check", "--port", &closed], &[]);
    assert_eq!(code, Some(1), "{text}");
    assert!(text.contains(&format!("127.0.0.1:{closed}")), "{text}");

    let (code, text, _) = run(&["serve", "--check", "--dir", site_arg, "--spa"], &[]);
    assert_eq!(code, Some(1), "{text}");
    assert!(text.contains("--spa needs an index.html"), "{text}");

    let (code, text, stdout) = run(&["serve", "--check", "--json", "--dir", site_arg], &[]);
    assert_eq!(code, Some(0), "{text}");
    let doc: serde_json::Value = serde_json::from_slice(&stdout).unwrap();
    assert_eq!(doc["ok"], true, "{doc}");
    assert_eq!(doc["plan"]["tunnel"]["kind"], "quick");

    for env in [&[("TUNNEL_TOKEN", TOKEN)][..], &[]] {
        let mut args = vec!["demo", "--check", "--transport", "warp"];
        if env.is_empty() {
            args.extend(["--token", TOKEN]);
        }
        let (code, text, _) = run(&args, env);
        assert_eq!(code, Some(0), "{text}");
        assert!(
            text.contains("ffd86bd5-1bff-456b-8c2c-75a014cdc4c8"),
            "{text}"
        );
        assert!(
            !text.contains(TOKEN) && !text.contains("AAECAwQF"),
            "{text}"
        );
    }

    let file = site.join("a.txt");
    std::fs::write(&file, "x").unwrap();
    let (code, text, _) = run(
        &[
            "share",
            "--check",
            "--mode",
            "upload",
            "--dir",
            file.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(code, Some(1), "{text}");
    assert!(text.contains("not a directory"), "{text}");

    let config = dir.path().join("config.yml");
    std::fs::write(
        &config,
        "credentails-file: /x\ningress:\n  - service: http_status:404\n",
    )
    .unwrap();
    let (code, text, _) = run(
        &["tunnel", "--check", "--config", config.to_str().unwrap()],
        &[],
    );
    assert_eq!(code, Some(78), "{text}");
    assert!(text.contains("did you mean credentials-file"), "{text}");

    assert!(
        api.accept().is_err(),
        "a check connected to the quick-tunnel API"
    );
    assert_eq!(
        std::fs::read_dir(&state).unwrap().count(),
        0,
        "a check wrote a file"
    );
}

/// `net doctor` under a seccomp filter that refuses listen(2) and sendto(2)
/// with EPERM: each bind stays OK, each listen line is FAIL, and the UDP line
/// is refused (issue 12).
#[test]
fn doctor_tells_listen_from_bind() {
    test_host::need_loopback!();
    use std::os::unix::process::CommandExt;
    fn deny_listen_and_sendto() -> std::io::Result<()> {
        const RET_ALLOW: u32 = 0x7fff_0000;
        const RET_ERRNO: u32 = 0x0005_0000;
        let deny = RET_ERRNO | libc::EPERM as u32;
        let stmt = |code: u16, k: u32| libc::sock_filter {
            code,
            jt: 0,
            jf: 0,
            k,
        };
        let jeq = |k: u32, jt: u8, jf: u8| libc::sock_filter {
            code: (libc::BPF_JMP | libc::BPF_JEQ | libc::BPF_K) as u16,
            jt,
            jf,
            k,
        };
        let filter = [
            // Load the syscall number (offset 0 of seccomp_data).
            stmt((libc::BPF_LD | libc::BPF_W | libc::BPF_ABS) as u16, 0),
            jeq(libc::SYS_listen as u32, 2, 0),
            jeq(libc::SYS_sendto as u32, 1, 0),
            stmt((libc::BPF_RET | libc::BPF_K) as u16, RET_ALLOW),
            stmt((libc::BPF_RET | libc::BPF_K) as u16, deny),
        ];
        let prog = libc::sock_fprog {
            len: filter.len() as u16,
            filter: filter.as_ptr() as *mut libc::sock_filter,
        };
        // SAFETY: prctl with valid arguments; `prog` lives for the call.
        unsafe {
            if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0
                || libc::prctl(
                    libc::PR_SET_SECCOMP,
                    libc::SECCOMP_MODE_FILTER,
                    &prog as *const libc::sock_fprog,
                ) != 0
            {
                return Err(std::io::Error::last_os_error());
            }
        }
        Ok(())
    }
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_podnet"));
    cmd.args(["net", "doctor"])
        .env_remove("PODNET_ERROR_PROGRAM")
        .env_remove("PODNET_EDGE")
        .env("CC", "/nonexistent-cc");
    // SAFETY: the closure calls only prctl, which is async-signal-safe.
    unsafe {
        cmd.pre_exec(deny_listen_and_sendto);
    }
    let out = cmd.output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    let line = |name: &str| {
        text.lines()
            .find(|l| l.starts_with(name))
            .unwrap_or_else(|| panic!("no line {name}:\n{text}"))
            .to_string()
    };
    for (bind, listen) in [
        ("bind(AF_INET, 127.0.0.1:0)", "listen(AF_INET, 127.0.0.1:0)"),
        ("bind(AF_UNIX, path)", "listen(AF_UNIX, path)"),
    ] {
        assert!(line(bind).contains("ok       allowed"), "{text}");
        assert!(line(listen).contains("fail     EPERM"), "{text}");
    }
    assert!(line("UDP egress").contains("fail     refused"), "{text}");
}

/// Issue 41: `net tailscale` with no tailscaled socket fails at start with
/// exit 69, and `pod -x` to a dead upstream warns and still runs.
#[test]
fn a_missing_upstream_is_named_at_start() {
    test_host::need_loopback!();
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("nope.sock");
    let out = podnet(&[
        "net",
        "tailscale",
        "--socket",
        socket.to_str().unwrap(),
        "--listen",
        &format!("unix:{}", dir.path().join("front.sock").display()),
        "--run-for",
        "5",
    ]);
    assert_eq!(out.status.code(), Some(69), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("the LocalAPI socket") && stderr(&out).contains("nope.sock"),
        "{}",
        stderr(&out)
    );

    let closed = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let out = podnet(&[
        "pod",
        "-x",
        &format!("socks5://127.0.0.1:{closed}"),
        "--",
        "true",
    ]);
    assert!(
        stderr(&out).contains(&format!("the upstream 127.0.0.1:{closed} does not answer")),
        "{}",
        stderr(&out)
    );
}

/// Issue 63: `net socksify --socket PATH` and the default work. Before, the
/// option was --proxy, which the global proxy parser took first: each run
/// exited 64. Skipped with no C compiler.
#[test]
fn socksify_takes_a_socket_path() {
    if podnet::vnet::shim::compiler().is_err() {
        eprintln!("skipping: no C compiler");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().to_str().unwrap();
    let with = podnet(&[
        "net",
        "socksify",
        "--out",
        out,
        "--socket",
        "/tmp/x.sock",
        "--json",
    ]);
    assert_eq!(with.status.code(), Some(0), "{}", stderr(&with));
    let text = String::from_utf8_lossy(&with.stdout);
    assert!(text.contains("/tmp/x.sock"), "{text}");
    let default = podnet(&["net", "socksify", "--out", out, "--json"]);
    assert_eq!(default.status.code(), Some(0), "{}", stderr(&default));
    assert!(String::from_utf8_lossy(&default.stdout).contains("/tmp/podnet-socks.sock"));
}

/// Issue 247: `net ts-shims` with no `--out` does not use a directory in
/// the temporary directory that other users can write.
#[test]
fn ts_shims_skips_a_directory_open_to_others() {
    use std::os::unix::fs::PermissionsExt;
    if podnet::vnet::shim::compiler().is_err() {
        eprintln!("skipping: no C compiler");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let tmp = dir.path().join("tmp");
    let hostile = tmp.join("podnet-ts-shims");
    std::fs::create_dir_all(&hostile).unwrap();
    std::fs::set_permissions(&hostile, std::fs::Permissions::from_mode(0o777)).unwrap();
    let state = dir.path().join("state");
    let out = Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(["net", "ts-shims"])
        .env("TMPDIR", &tmp)
        .env("PODNET_STATE_DIR", &state)
        .env_remove("XDG_RUNTIME_DIR")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let text = String::from_utf8_lossy(&out.stdout);
    let chosen = state.join("podnet-ts-shims");
    assert!(text.contains(&*chosen.to_string_lossy()), "{text}");
    assert_eq!(std::fs::read_dir(&hostile).unwrap().count(), 0);
}

/// Issue 227: `update --version` with a path is a usage error, and podnet
/// sends no request.
#[test]
fn update_refuses_a_tag_with_a_path() {
    let out = Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(["update", "--version", "v1/../x", "--check"])
        .env("HTTPS_PROXY", "http://127.0.0.1:9")
        .env("PODNET_STATE_DIR", "none")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(64), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("not a release tag"),
        "{}",
        stderr(&out)
    );
}

/// Issue 66: PODNET_EDGE is checked the same way by tunnel and the doctors;
/// a bad PODNET_UDP gives one failed legs row in --check, not two rows.
#[test]
fn the_environment_is_checked_alike() {
    for args in [&["doctor", "--samples", "1"][..], &["net", "doctor"][..]] {
        let out = Command::new(env!("CARGO_BIN_EXE_podnet"))
            .args(args)
            .env("PODNET_EDGE", "not-an-address")
            .env_remove("PODNET_ERROR_PROGRAM")
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(78), "{args:?}: {}", stderr(&out));
        assert!(stderr(&out).contains("PODNET_EDGE"), "{}", stderr(&out));
    }
    let out = Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(["tunnel", "--hello-world", "--check", "--json"])
        .env("PODNET_UDP", "bogus")
        .env_remove("PODNET_EDGE")
        .env_remove("PODNET_ERROR_PROGRAM")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let legs: Vec<_> = doc["checks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["name"] == "legs")
        .collect();
    assert_eq!(legs.len(), 1, "{doc}");
    assert_eq!(legs[0]["status"], "fail");
    // Issue 111: a bad PODNET_WARP_TCP_BURST is exit 78 before any network
    // wait. Before, it gave the default with no message.
    for args in [&["warp", "probe"][..], &["agent", "up"][..]] {
        let out = Command::new(env!("CARGO_BIN_EXE_podnet"))
            .args(args)
            .env("PODNET_WARP_TCP_BURST", "abc")
            .env("PODNET_STATE_DIR", "none")
            .env_remove("PODNET_ERROR_PROGRAM")
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(78), "{args:?}: {}", stderr(&out));
        assert!(
            stderr(&out).contains("PODNET_WARP_TCP_BURST"),
            "{}",
            stderr(&out)
        );
    }
    let out = Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(["status"])
        .env("RUST_LOG", "podnet=deubg")
        .output()
        .unwrap();
    assert!(stderr(&out).contains("RUST_LOG"), "{}", stderr(&out));
}

/// Issue 171: protocol and ha-connections of a config file are used; a
/// flag wins over the file.
#[test]
fn the_config_file_gives_the_protocol_and_the_connections() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.yml");
    std::fs::write(
        &path,
        "protocol: http2\nha-connections: 1\ningress:\n  - service: http_status:404\n",
    )
    .unwrap();
    let path = path.to_str().unwrap();
    let plan = |extra: &[&str]| {
        let mut args = vec!["tunnel", "--check", "--json", "--config", path];
        args.extend_from_slice(extra);
        let out = podnet(&args);
        assert!(out.status.success(), "{}", stderr(&out));
        let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        doc["plan"].clone()
    };
    let p = plan(&[]);
    assert_eq!(p["ha_connections"], 1, "{p}");
    assert_eq!(p["legs"], serde_json::json!(["http2"]), "{p}");
    let p = plan(&["--ha-connections", "3", "--protocol", "quic"]);
    assert_eq!(p["ha_connections"], 3, "{p}");
    assert_eq!(p["legs"][0], "quic", "{p}");
}

/// Issue 199: --tcp takes tcp://host:port; the check warns that a quick
/// tunnel receives HTTP; the public URL is not verified.
#[test]
fn tunnel_tcp_takes_the_scheme_and_warns() {
    test_host::need_loopback!();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let url = format!("tcp://{addr}");
    let out = podnet(&["tunnel", "--tcp", &url, "--check", "--json"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["plan"]["ingress"][0]["service"], url, "{doc}");
    assert_eq!(doc["plan"]["verify"], false, "{doc}");
    let origin = doc["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "origin 1")
        .unwrap();
    assert_eq!(origin["status"], "warn", "{doc}");
    let out = podnet(&["tunnel", "--tcp", "http://127.0.0.1:1", "--check"]);
    assert_eq!(out.status.code(), Some(64), "{}", stderr(&out));
}

/// The program of PODNET_ERROR_PROGRAM gets the report, and no secret of
/// podnet in its environment (podssh).
#[test]
fn the_error_program_gets_no_secret() {
    let dir = tempfile::tempdir().unwrap();
    let program = dir.path().join("report.sh");
    let seen = dir.path().join("env.txt");
    std::fs::write(
        &program,
        format!("#!/bin/sh\ncat > /dev/null\nenv > {}\n", seen.display()),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(["tunnel", "--bogus-flag"])
        .env("PODNET_ERROR_PROGRAM", &program)
        .env("TUNNEL_TOKEN", "secret-token")
        .env("PODNET_RELAY_TOKEN", "secret-relay")
        .env("PODNET_PROXY", "http://user:secret-pw@127.0.0.1:1")
        .env("PODNET_SOMETHING", "kept")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(64));
    let env = std::fs::read_to_string(&seen).unwrap();
    assert!(!env.contains("secret"), "{env}");
    assert!(env.contains("PODNET_SOMETHING=kept"), "{env}");
}

/// podssh #118: root with the HOME of another user writes no file there.
/// ssh-config --write exits 78; PODNET_FOREIGN_HOME=allow writes. The state
/// directory skips that HOME. Needs root (to chown); skipped otherwise.
#[test]
fn root_writes_no_file_in_the_home_of_another_user() {
    if unsafe { libc::geteuid() } != 0 {
        eprintln!("skipping: not root");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(home.join(".ssh")).unwrap();
    let c = |p: &std::path::Path| {
        let p = std::ffi::CString::new(p.to_str().unwrap()).unwrap();
        unsafe { libc::chown(p.as_ptr(), 4242, 4242) }
    };
    // Issue 217: root of a user namespace (unshare -U) has no uid 4242.
    if c(&home) != 0 {
        let err = std::io::Error::last_os_error();
        eprintln!("skipping, root cannot give a file to uid 4242 (a user namespace?): {err}");
        return;
    }
    assert_eq!(c(&home.join(".ssh")), 0);
    let run = |extra: &[(&str, &str)]| {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_podnet"));
        cmd.args(["warp", "ssh-config", "--write", "--host", "example.org"])
            .env("HOME", &home)
            .env_remove("XDG_STATE_HOME")
            .env_remove("PODNET_STATE_DIR")
            .env_remove("PODNET_FOREIGN_HOME")
            .env_remove("PODNET_ERROR_PROGRAM");
        for (k, v) in extra {
            cmd.env(k, v);
        }
        cmd.output().unwrap()
    };
    let out = run(&[]);
    assert_eq!(out.status.code(), Some(78), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("PODNET_FOREIGN_HOME=allow"),
        "{}",
        stderr(&out)
    );
    assert!(!home.join(".ssh/config").exists());
    let out = run(&[("PODNET_FOREIGN_HOME", "allow")]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(home.join(".ssh/config").exists());
    // The state directory is not made in that HOME.
    let out = Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(["status"])
        .env("HOME", &home)
        .env_remove("XDG_STATE_HOME")
        .env_remove("PODNET_STATE_DIR")
        .env_remove("PODNET_ERROR_PROGRAM")
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(!home.join(".local/state/podnet").exists());
}
