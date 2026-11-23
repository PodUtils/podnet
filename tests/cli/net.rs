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

/// T-168: `net passwd-shim` writes the passwd shim and prints the preload:
/// one JSON object, or `export` lines only on stdout (for eval). An old
/// LD_PRELOAD stays after the shim. The home of the entry is $HOME (T-363).
#[test]
fn passwd_shim_prints_the_preload() {
    if podnet::vnet::shim::compiler().is_err() {
        eprintln!("skipping: no C compiler");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let out_dir = dir.path().join("shims");
    let out = dir.path().join("shims");
    let out = out.to_str().unwrap();
    let json = Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(["net", "passwd-shim", "--out", out, "--user", "builder"])
        .args(["--uid", "4242", "--json"])
        .env("LD_PRELOAD", "/old.so")
        .env("HOME", dir.path())
        .output()
        .unwrap();
    assert_eq!(json.status.code(), Some(0), "{}", stderr(&json));
    let doc: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    let fakepwd = out_dir.join("fakepwd.so");
    assert_eq!(
        doc["LD_PRELOAD"],
        format!("{}:/old.so", fakepwd.display()),
        "{doc}"
    );
    let passwd = doc["SANDHOME_PASSWD"].as_str().unwrap();
    let text = std::fs::read_to_string(passwd).unwrap();
    assert!(text.contains("builder:x:4242:"), "{text}");
    let home = format!(":{}:/bin/sh\n", dir.path().display());
    assert!(
        text.lines()
            .next()
            .is_some_and(|l| format!("{l}\n").ends_with(&home)),
        "{text}"
    );
    assert!(fakepwd.is_file());
    let shell = Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(["net", "passwd-shim", "--out", out])
        .env_remove("LD_PRELOAD")
        .output()
        .unwrap();
    assert_eq!(shell.status.code(), Some(0), "{}", stderr(&shell));
    let lines = String::from_utf8_lossy(&shell.stdout).into_owned();
    assert!(lines.lines().all(|l| l.starts_with("export ")), "{lines}");
    assert!(lines.contains("export SANDHOME_PASSWD="), "{lines}");
    assert!(stderr(&shell).contains("podnet: fakepwd"));
    // ts-shims stays, with its daemon hints.
    let ts = podnet(&["net", "ts-shims", "--out", out]);
    assert_eq!(ts.status.code(), Some(0), "{}", stderr(&ts));
    assert!(String::from_utf8_lossy(&ts.stdout).contains("tailscaled"));
}

/// T-167: `net socksify --cidr` puts the routes in PODNET_SOCKS_CIDRS, and
/// `net shim --socks-route` puts them in PODNET_VNET_SOCKS_ROUTES with the
/// door. A bad route is exit 64 before a build.
#[test]
fn socksify_takes_cidrs() {
    let bad = podnet(&["net", "socksify", "--cidr", "web:80", "--json"]);
    assert_eq!(bad.status.code(), Some(64), "{}", stderr(&bad));
    assert!(stderr(&bad).contains("web:80"), "{}", stderr(&bad));
    let bad = podnet(&["net", "shim", "--socks-route", "10.0.0.0/40", "--json"]);
    assert_eq!(bad.status.code(), Some(64), "{}", stderr(&bad));
    if podnet::vnet::shim::compiler().is_err() {
        eprintln!("skipping the build: no C compiler");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().to_str().unwrap();
    let json = podnet(&[
        "net",
        "socksify",
        "--out",
        out,
        "--cidr",
        "127.0.0.1:3000",
        "--cidr",
        "tailnet",
        "--json",
    ]);
    assert_eq!(json.status.code(), Some(0), "{}", stderr(&json));
    let doc: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(doc["PODNET_SOCKS_CIDRS"], "127.0.0.1:3000,tailnet", "{doc}");
    let none = podnet(&["net", "socksify", "--out", out, "--json"]);
    let doc: serde_json::Value = serde_json::from_slice(&none.stdout).unwrap();
    assert!(doc.get("PODNET_SOCKS_CIDRS").is_none(), "{doc}");
    let shim = podnet(&[
        "net",
        "shim",
        "--out",
        out,
        "--socks-route",
        "[::1]:8080",
        "--socks-socket",
        "/tmp/door.sock",
        "--json",
    ]);
    assert_eq!(shim.status.code(), Some(0), "{}", stderr(&shim));
    let doc: serde_json::Value = serde_json::from_slice(&shim.stdout).unwrap();
    assert_eq!(doc["PODNET_VNET_SOCKS"], "/tmp/door.sock", "{doc}");
    assert_eq!(doc["PODNET_VNET_SOCKS_ROUTES"], "[::1]:8080", "{doc}");
    // --socks-socket needs --socks-route.
    let alone = podnet(&["net", "shim", "--socks-socket", "/tmp/door.sock"]);
    assert_eq!(alone.status.code(), Some(64), "{}", stderr(&alone));
}

/// H-05: two `net shim` runs in one network namespace get different
/// abstract-name prefixes, and each one is a prefix the shim and the stack
/// can parse.
#[test]
fn shim_prefixes_differ_per_run() {
    if podnet::vnet::shim::compiler().is_err() {
        eprintln!("skipping the build: no C compiler");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().to_str().unwrap();
    let run = || {
        let shim = Command::new(env!("CARGO_BIN_EXE_podnet"))
            .args(["net", "shim", "--out", out, "--json"])
            .env_remove("PODNET_VNET_PREFIX")
            .env_remove("PODNET_ERROR_PROGRAM")
            .output()
            .unwrap();
        assert_eq!(shim.status.code(), Some(0), "{}", stderr(&shim));
        let doc: serde_json::Value = serde_json::from_slice(&shim.stdout).unwrap();
        doc["PODNET_VNET_PREFIX"].as_str().unwrap().to_string()
    };
    let (a, b) = (run(), run());
    assert_ne!(a, b, "{a}");
    assert!(a.starts_with("podnet-vnet-"), "{a}");
    assert!(podnet::vnet::addr::valid_prefix(&a), "{a}");
    assert!(podnet::vnet::addr::valid_prefix(&b), "{b}");
    let name = format!("{a}/4/10.66.0.2/8080");
    assert!(podnet::vnet::VirtAddr::from_abstract_name_with(&name, &a).is_ok());
    // The same prefix still builds a full IPv6 name under MAX_NAME.
    let longest = podnet::vnet::VirtAddr::v6(
        "ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff".parse().unwrap(),
        65535,
    );
    let name = longest.abstract_name_with(&a).unwrap();
    assert!(name.len() <= podnet::vnet::addr::MAX_NAME, "{name}");
    assert_eq!(
        podnet::vnet::VirtAddr::from_abstract_name_with(&name, &a).unwrap(),
        longest
    );
    // An explicit prefix in the environment is the caller's choice.
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_podnet"));
    let chosen = cmd
        .args(["net", "shim", "--out", out, "--json"])
        .env("PODNET_VNET_PREFIX", "chosen-by-caller")
        .env_remove("PODNET_ERROR_PROGRAM")
        .output()
        .unwrap();
    assert_eq!(chosen.status.code(), Some(0), "{}", stderr(&chosen));
    let doc: serde_json::Value = serde_json::from_slice(&chosen.stdout).unwrap();
    assert_eq!(doc["PODNET_VNET_PREFIX"], "chosen-by-caller", "{doc}");
    // A value that cannot build a name is a usage error, not a fallback.
    for bad in ["", "a/b", "a b", &"p".repeat(60)] {
        let out_bad = Command::new(env!("CARGO_BIN_EXE_podnet"))
            .args(["net", "shim", "--out", out, "--json"])
            .env("PODNET_VNET_PREFIX", bad)
            .env_remove("PODNET_ERROR_PROGRAM")
            .output()
            .unwrap();
        assert_eq!(out_bad.status.code(), Some(64), "{bad:?}");
        assert!(
            stderr(&out_bad).contains("PODNET_VNET_PREFIX"),
            "{bad:?}: {}",
            stderr(&out_bad)
        );
    }
    // A non-UTF-8 value is refused before build with exit 64 too.
    {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        let non_utf8_dir = tempfile::tempdir().unwrap();
        let out_bad = Command::new(env!("CARGO_BIN_EXE_podnet"))
            .args([
                "net",
                "shim",
                "--out",
                non_utf8_dir.path().to_str().unwrap(),
                "--json",
            ])
            .env("PODNET_VNET_PREFIX", OsStr::from_bytes(b"bad\xffprefix"))
            .env_remove("PODNET_ERROR_PROGRAM")
            .output()
            .unwrap();
        assert_eq!(out_bad.status.code(), Some(64), "{}", stderr(&out_bad));
        assert!(
            stderr(&out_bad).contains("PODNET_VNET_PREFIX"),
            "{}",
            stderr(&out_bad)
        );
        assert!(
            !non_utf8_dir.path().join("libpodnet-vnet.so").exists(),
            "non-UTF-8 prefix must fail before building the shim"
        );
    }
    // Without --json, stdout holds only the exports, so `eval` is safe.
    let plain = Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(["net", "shim", "--out", out])
        .env_remove("PODNET_VNET_PREFIX")
        .env_remove("PODNET_ERROR_PROGRAM")
        .output()
        .unwrap();
    assert_eq!(plain.status.code(), Some(0), "{}", stderr(&plain));
    let stdout = String::from_utf8(plain.stdout.clone()).unwrap();
    assert!(!stdout.is_empty());
    for line in stdout.lines() {
        assert!(line.starts_with("  export "), "{line:?}");
    }
    assert!(
        stderr(&plain).contains("podnet: shim"),
        "{}",
        stderr(&plain)
    );
}

/// T-357: `net shim --forward` checks each forward and puts them in
/// PODNET_VNET_FORWARD. A bad forward is exit 64 before a build.
#[test]
fn shim_takes_forwards() {
    for bad in [
        "127.0.0.1:7001",
        "web:80=unix:/a",
        "127.0.0.1:80=unix:relative",
        "127.0.0.1:80=tcp:/a",
    ] {
        let out = podnet(&["net", "shim", "--forward", bad, "--json"]);
        assert_eq!(out.status.code(), Some(64), "{bad}: {}", stderr(&out));
        assert!(stderr(&out).contains(bad), "{}", stderr(&out));
    }
    let long = format!("127.0.0.1:80=unix:/{}", "a".repeat(120));
    let out = podnet(&["net", "shim", "--forward", &long, "--json"]);
    assert_eq!(out.status.code(), Some(64), "{}", stderr(&out));
    if podnet::vnet::shim::compiler().is_err() {
        eprintln!("skipping the build: no C compiler");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().to_str().unwrap();
    let shim = podnet(&[
        "net",
        "shim",
        "--out",
        out,
        "--forward",
        "127.0.0.1:7001=unix:/run/podrunner/broker.sock",
        "--forward",
        "[::1]:3128=unix:/run/podrunner/proxy.sock",
        "--json",
    ]);
    assert_eq!(shim.status.code(), Some(0), "{}", stderr(&shim));
    let doc: serde_json::Value = serde_json::from_slice(&shim.stdout).unwrap();
    assert_eq!(
        doc["PODNET_VNET_FORWARD"],
        "127.0.0.1:7001=unix:/run/podrunner/broker.sock,[::1]:3128=unix:/run/podrunner/proxy.sock",
        "{doc}"
    );
    assert!(doc.get("PODNET_VNET_SOCKS").is_none(), "{doc}");
}

/// Issue 612: `expose -- --detach` runs the command of expose in the
/// background: one JSON document, exit 0, and podnet ps shows it.
#[test]
fn expose_detaches_with_the_arguments_after_the_separator() {
    let dir = tempfile::tempdir().unwrap();
    let command = |args: &[&str]| {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_podnet"));
        cmd.args(args)
            .env("PODNET_STATE_DIR", dir.path().join("state"))
            .env("PODNET_RUN_DIR", dir.path().join("run"))
            .env_remove("RUST_LOG")
            .env_remove("PODNET_ERROR_PROGRAM")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        cmd
    };
    let run = |args: &[&str]| command(args).output().unwrap();
    // A process in the foreground does not end: kill it after 60 s.
    let run_bounded = |args: &[&str]| {
        let mut child = command(args).spawn().unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while child.try_wait().unwrap().is_none() {
            if std::time::Instant::now() > deadline {
                child.kill().unwrap();
                let out = child.wait_with_output().unwrap();
                panic!("expose stays in the foreground: {}", stderr(&out));
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        child.wait_with_output().unwrap()
    };
    let listen = format!("unix:{}", dir.path().join("p.socks").display());
    let out = run_bounded(&[
        "expose",
        "127.0.0.1:9",
        "--via",
        "switch",
        "--as",
        "10.66.0.2:8080",
        "--",
        "--listen",
        &listen,
        "--detach",
        "--name",
        "exposed",
        "--ready-timeout",
        "20",
    ]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["name"], "exposed", "{doc}");
    assert_eq!(doc["command"], "net proxy", "{doc}");
    let ps = run(&["ps", "--json"]);
    assert!(
        String::from_utf8_lossy(&ps.stdout).contains("\"exposed\""),
        "{}",
        String::from_utf8_lossy(&ps.stdout)
    );
    assert_eq!(run(&["stop", "exposed"]).status.code(), Some(0));
}
