//! `podnet net`: the socket rule, listeners, tokens and the status.

use super::*;

/// One rule for a unix socket path (podssh, issue 202): net switch, net
/// proxy and relay connect --listen refuse a socket that answers (69) and
/// never remove a file that is not a socket (64).
#[test]
fn the_socket_rule_of_net_and_relay() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("data");
    std::fs::write(&file, "keep").unwrap();
    let live = dir.path().join("live.sock");
    let _listener = std::os::unix::net::UnixListener::bind(&live).unwrap();
    let run = |args: &[&str]| {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_podnet"));
        cmd.args(args).env_remove("PODNET_ERROR_PROGRAM");
        let out = cmd.output().unwrap();
        (out.status.code(), stderr(&out))
    };
    let f = format!("unix:{}", file.display());
    let l = format!("unix:{}", live.display());
    for args in [
        vec!["net", "switch", "--listen", f.as_str(), "--run-for", "1"],
        vec!["net", "proxy", "--listen", f.as_str(), "--run-for", "1"],
    ] {
        let (code, err) = run(&args);
        assert_eq!(code, Some(64), "{args:?}: {err}");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "keep");
    }
    for args in [
        vec!["net", "switch", "--listen", l.as_str(), "--run-for", "1"],
        vec!["net", "proxy", "--listen", l.as_str(), "--run-for", "1"],
    ] {
        let (code, err) = run(&args);
        assert_eq!(code, Some(69), "{args:?}: {err}");
        assert!(err.contains("served by another process"), "{err}");
    }
}

/// Issue 300: net switch and net proxy with a TCP port 0 print the port
/// that the kernel chose, and a client connects to it.
#[test]
fn a_port_0_listener_prints_the_bound_port() {
    test_host::need_loopback!();
    use std::io::BufRead;
    for (args, prefix) in [
        (&["net", "switch"][..], "podnet: switch   tcp://"),
        (&["net", "proxy"][..], "podnet: proxy    tcp://"),
    ] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_podnet"))
            .args(args)
            .args(["--listen", "tcp://127.0.0.1:0", "--run-for", "20"])
            .env_remove("PODNET_ERROR_PROGRAM")
            .env("PODNET_SWITCH_TOKEN", "test-switch-token")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let mut lines = std::io::BufReader::new(child.stdout.take().unwrap()).lines();
        let line = lines
            .find_map(|l| l.ok().filter(|l| l.starts_with(prefix)))
            .unwrap_or_else(|| panic!("{args:?}: no line {prefix}"));
        let addr: std::net::SocketAddr = line[prefix.len()..]
            .trim()
            .parse()
            .unwrap_or_else(|e| panic!("{line}: {e}"));
        let connect = std::net::TcpStream::connect(addr);
        let _ = child.kill();
        let _ = child.wait();
        assert_ne!(addr.port(), 0, "{args:?}: {line}");
        connect.unwrap_or_else(|e| panic!("{args:?}: connect to {addr}: {e}"));
    }
}

/// Issue 309: a TCP switch with no token is a usage error (64), and so is
/// a TCP --switch of net proxy with no token. A unix switch needs none.
#[test]
fn a_tcp_switch_needs_a_token() {
    let run = |args: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_podnet"))
            .args(args)
            .env_remove("PODNET_ERROR_PROGRAM")
            .env_remove("PODNET_SWITCH_TOKEN")
            .output()
            .unwrap();
        (out.status.code(), stderr(&out))
    };
    let (code, err) = run(&[
        "net",
        "switch",
        "--listen",
        "tcp://127.0.0.1:0",
        "--run-for",
        "1",
    ]);
    assert_eq!(code, Some(64), "{err}");
    assert!(err.contains("PODNET_SWITCH_TOKEN"), "{err}");
    let (code, err) = run(&[
        "net",
        "proxy",
        "--switch",
        "tcp://127.0.0.1:9",
        "--address",
        "10.66.0.3",
        "--run-for",
        "1",
    ]);
    assert_eq!(code, Some(64), "{err}");
    assert!(err.contains("PODNET_SWITCH_TOKEN"), "{err}");
    let dir = tempfile::tempdir().unwrap();
    let empty = dir.path().join("token");
    std::fs::write(&empty, "\n").unwrap();
    let (code, err) = run(&[
        "net",
        "switch",
        "--listen",
        "tcp://127.0.0.1:0",
        "--switch-token-file",
        empty.to_str().unwrap(),
        "--run-for",
        "1",
    ]);
    assert_eq!(code, Some(64), "{err}");
    assert!(err.contains("empty"), "{err}");
}

/// podssh: status, ps, stop, logs and wait read the run directory and make
/// none.
#[test]
fn the_read_commands_make_no_run_directory() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = dir.path().join("runtime");
    std::fs::create_dir_all(&runtime).unwrap();
    let state = dir.path().join("state");
    for args in [
        &["status"][..],
        &["ps"],
        &["ps", "--json"],
        &["stop", "x"],
        &["logs", "x"],
        &["wait", "x", "--timeout", "1"],
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_podnet"))
            .args(args)
            .env("XDG_RUNTIME_DIR", &runtime)
            .env("XDG_STATE_HOME", &state)
            .env("HOME", dir.path())
            .env_remove("PODNET_RUN_DIR")
            .env_remove("PODNET_STATE_DIR")
            .env_remove("PODNET_ERROR_PROGRAM")
            .output()
            .unwrap();
        assert_ne!(out.status.code(), Some(101), "{args:?}: {}", stderr(&out));
        assert!(
            !runtime.join("podnet").exists(),
            "{args:?} made the run directory"
        );
        assert!(
            !state.join("podnet").exists(),
            "{args:?} made the state directory"
        );
    }
}

/// E4: --token-file and TUNNEL_TOKEN_FILE read the token; it is not on the
/// command line. An empty file, or a token flag and a file, exit 64.
#[test]
fn a_token_comes_from_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("token");
    std::fs::write(&file, "  bad*token*value\n").unwrap();
    let path = file.to_str().unwrap();
    let run = |args: &[&str], env: Option<&str>| {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_podnet"));
        cmd.args(args)
            .env_remove("TUNNEL_TOKEN")
            .env_remove("TUNNEL_TOKEN_FILE")
            .env("PODNET_STATE_DIR", "none");
        if let Some(env) = env {
            cmd.env("TUNNEL_TOKEN_FILE", env);
        }
        cmd.output().unwrap()
    };
    for out in [
        run(
            &[
                "tunnel",
                "--token-file",
                path,
                "--url",
                "http://127.0.0.1:1",
            ],
            None,
        ),
        run(
            &[
                "tunnel",
                &format!("--token-file={path}"),
                "--url",
                "http://127.0.0.1:1",
            ],
            None,
        ),
        run(&["tunnel", "--url", "http://127.0.0.1:1"], Some(path)),
    ] {
        let err = stderr(&out);
        // The token of the file was read: it is not a valid token.
        assert!(err.contains("not valid base64"), "{err}");
        assert!(!err.contains("bad*token*value"), "{err}");
    }
    std::fs::write(&file, " \n").unwrap();
    let out = run(&["tunnel", "--token-file", path], None);
    assert_eq!(out.status.code(), Some(64), "{}", stderr(&out));
    assert!(stderr(&out).contains("is empty"));
    let out = run(&["tunnel", "--token", "x", "--token-file", path], None);
    assert_eq!(out.status.code(), Some(64), "{}", stderr(&out));
}

/// E11: status gives the kind of the proxy that podnet dials (before, each
/// proxy was "http"), and its source.
#[test]
fn status_names_the_kind_and_the_source_of_the_proxy() {
    let status = |args: &[&str], proxy: Option<&str>| -> serde_json::Value {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_podnet"));
        cmd.args(args).arg("status");
        for key in [
            "HTTPS_PROXY",
            "https_proxy",
            "ALL_PROXY",
            "all_proxy",
            "PODNET_PROXY",
        ] {
            cmd.env_remove(key);
        }
        if let Some(proxy) = proxy {
            cmd.env("HTTPS_PROXY", proxy);
        }
        let out = cmd.output().unwrap();
        serde_json::from_slice(&out.stdout).unwrap()
    };
    let doc = status(&[], Some("socks5h://u:p@127.0.0.1:1080"));
    assert_eq!(doc["proxy"]["kind"], "socks5h");
    assert_eq!(doc["proxy"]["source"], "HTTPS_PROXY");
    assert_eq!(doc["proxy"]["credentials"], true);
    assert!(!doc.to_string().contains("u:p@"), "{doc}");
    let doc = status(&["--no-proxy"], Some("http://127.0.0.1:3128"));
    assert_eq!(doc["proxy"]["kind"], "direct");
    let doc = status(&["--proxy", "socks5h+unix:/tmp/x.sock"], None);
    assert_eq!(doc["proxy"]["kind"], "socks5h+unix");
    assert_eq!(doc["proxy"]["source"], "--proxy");
}

/// Issue 205: --auth-user is refused with a raw --tcp origin; a password
/// file that others can read is a configuration error.
#[test]
fn auth_user_flags() {
    test_host::need_loopback!();
    let dir = tempfile::tempdir().unwrap();
    let pass = dir.path().join("pass");
    std::fs::write(&pass, "secret\n").unwrap();
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(&pass, std::fs::Permissions::from_mode(0o600)).unwrap();
    let pass = pass.to_str().unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let out = podnet(&[
        "tunnel",
        "--tcp",
        &addr,
        "--auth-user",
        "u",
        "--auth-password-file",
        pass,
        "--check",
    ]);
    assert_eq!(out.status.code(), Some(64), "{}", stderr(&out));
    let out = podnet(&[
        "tunnel",
        "--hello-world",
        "--auth-user",
        "u",
        "--auth-password-file",
        pass,
        "--check",
    ]);
    assert!(out.status.success(), "{}", stderr(&out));
    std::fs::set_permissions(pass, std::fs::Permissions::from_mode(0o644)).unwrap();
    let out = podnet(&[
        "tunnel",
        "--hello-world",
        "--auth-user",
        "u",
        "--auth-password-file",
        pass,
        "--check",
    ]);
    assert_eq!(out.status.code(), Some(78), "{}", stderr(&out));
    let out = podnet(&["tunnel", "--hello-world", "--auth-user", "u", "--check"]);
    assert_eq!(out.status.code(), Some(64), "{}", stderr(&out));
}
