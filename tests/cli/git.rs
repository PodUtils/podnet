//! `podnet git` (T-094): init, urls and serve on local listeners. The
//! listeners are unix sockets, and TCP on 127.0.0.1 when this host has a
//! loopback interface. A test is skipped, with one line, when `git` is not
//! on `PATH`.

use super::serve::{http_send, private_file};
use super::*;

/// True (with one line) when `git` is not on `PATH`.
fn git_missing() -> bool {
    let missing = Command::new("git").arg("--version").output().is_err();
    if missing {
        eprintln!("skipped: git is not on PATH");
    }
    missing
}

/// podnet with the state directory `state`.
fn podnet_in(state: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(args)
        .env("PODNET_STATE_DIR", state)
        .env_remove("PODNET_ERROR_PROGRAM")
        .env_remove("PODNET_AUTH_USER")
        .env_remove("PODNET_AUTH_PASSWORD_FILE")
        .output()
        .unwrap()
}

/// A `git` command with no user configuration, no proxy and no prompt.
fn git_cmd(home: &std::path::Path, args: &[&str]) -> Output {
    let mut cmd = Command::new("git");
    cmd.current_dir(home).args(args);
    for name in [
        "http_proxy",
        "HTTP_PROXY",
        "https_proxy",
        "HTTPS_PROXY",
        "all_proxy",
        "ALL_PROXY",
        "GIT_DIR",
    ] {
        cmd.env_remove(name);
    }
    cmd.env("HOME", home)
        .env("XDG_CONFIG_HOME", home)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("NO_PROXY", "*")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t");
    cmd.output().unwrap()
}

/// T-094: `git init` makes a bare repository and refuses `..` and a path
/// in use; `git serve` serves smart HTTP and SSH on local listeners and
/// prints the fingerprint of the host key; `git urls` prints the clone
/// URLs of each repository and the same fingerprint. With a loopback
/// interface, git clones over HTTP, an anonymous push fails, and a push
/// of --push-user passes.
#[test]
fn git_serve_init_and_urls() {
    use std::io::BufRead;
    if git_missing() {
        return;
    }
    let base = tempfile::tempdir().unwrap();
    let (root, state) = (base.path().join("root"), base.path().join("state"));
    std::fs::create_dir(&root).unwrap();
    let root_arg = root.to_str().unwrap();
    let out = podnet_in(&state, &["git", "init", "--root", root_arg, "team/app"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(root.join("team/app.git/HEAD").is_file());
    for name in ["team/app", "team/app.git", "../x", "a/.git"] {
        let out = podnet_in(&state, &["git", "init", "--root", root_arg, name]);
        assert_eq!(out.status.code(), Some(64), "{name}: {}", stderr(&out));
    }
    let out = podnet_in(&state, &["git", "urls", "--root", root_arg, "--json"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["running"], false, "{doc}");
    assert_eq!(doc["repos"][0]["repo"], "team/app.git", "{doc}");
    assert!(doc["fingerprint"].is_null(), "{doc}");

    let pass = base.path().join("pass");
    private_file(&pass, "pw\n");
    let http_sock = base.path().join("http.sock");
    let tcp = test_host::loopback().is_ok();
    let http_local = format!("unix:{}", http_sock.display());
    let ssh_listen = if tcp {
        "127.0.0.1:0".to_string()
    } else {
        format!("unix:{}", base.path().join("ssh.sock").display())
    };
    let mut args = vec![
        "git",
        "serve",
        "--root",
        root_arg,
        "--local",
        &http_local,
        "--ssh",
        &ssh_listen,
        "--anon-read",
        "--push-user",
        "u",
        "--push-password-file",
        pass.to_str().unwrap(),
        "--json",
        "--run-for",
        "120",
    ];
    if tcp {
        args.extend(["--local", "127.0.0.1:0"]);
    }
    let mut child = Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(&args)
        .env("PODNET_STATE_DIR", &state)
        .env_remove("PODNET_ERROR_PROGRAM")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            let ready = line.contains("\"event\":\"ready\"");
            let _ = tx.send(line);
            if ready {
                break;
            }
        }
    });
    let ready = loop {
        let line = rx
            .recv_timeout(std::time::Duration::from_secs(20))
            .expect("a ready line");
        let doc: serde_json::Value = serde_json::from_str(&line).unwrap();
        if doc["event"] == "ready" {
            break doc;
        }
    };
    let fingerprint = ready["fingerprint"].as_str().unwrap().to_string();
    assert!(fingerprint.starts_with("SHA256:"), "{ready}");

    let stream = std::os::unix::net::UnixStream::connect(&http_sock).unwrap();
    let refs = "/team/app/info/refs?service=git-upload-pack";
    let (status, head, _) = http_send(stream, "GET", refs, &[], b"");
    assert!(status.contains(" 200 "), "{status}");
    assert!(
        head.contains("application/x-git-upload-pack-advertisement"),
        "{head}"
    );
    let stream = std::os::unix::net::UnixStream::connect(&http_sock).unwrap();
    let push = "/team/app/info/refs?service=git-receive-pack";
    let (status, _, _) = http_send(stream, "GET", push, &[], b"");
    assert!(status.contains(" 401 "), "{status}");

    let out = podnet_in(&state, &["git", "urls", "--root", root_arg, "--json"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["running"], true, "{doc}");
    assert_eq!(doc["fingerprint"], fingerprint.as_str(), "{doc}");
    let urls: Vec<&str> = doc["repos"][0]["urls"]
        .as_array()
        .unwrap()
        .iter()
        .map(|u| u.as_str().unwrap())
        .collect();
    if tcp {
        let http = urls
            .iter()
            .find(|u| u.starts_with("http://127.0.0.1:"))
            .unwrap_or_else(|| panic!("{doc}"));
        assert!(
            urls.iter()
                .any(|u| u.starts_with("ssh://git@127.0.0.1:") && u.ends_with("/team/app.git")),
            "{doc}"
        );
        let home = base.path();
        let out = git_cmd(home, &["clone", "-q", http, "work"]);
        assert!(out.status.success(), "{}", stderr(&out));
        let work = home.join("work");
        let out = git_cmd(&work, &["commit", "-q", "--allow-empty", "-m", "one"]);
        assert!(out.status.success(), "{}", stderr(&out));
        let out = git_cmd(&work, &["push", "-q", http, "HEAD:main"]);
        assert!(!out.status.success(), "an anonymous push passes");
        let with_user = http.replacen("http://", "http://u:pw@", 1);
        let out = git_cmd(&work, &["push", "-q", &with_user, "HEAD:main"]);
        assert!(out.status.success(), "{}", stderr(&out));
    }
    unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
    let status = child.wait().unwrap();
    assert_eq!(status.code(), Some(0), "{status:?}");
    // The record of git urls ends with the server.
    let out = podnet_in(&state, &["git", "urls", "--root", root_arg]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("no git serve"), "{text}");
    assert!(text.contains(&fingerprint), "{text}");
}
