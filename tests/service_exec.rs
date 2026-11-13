//! The service `exec` of a peer (T-296, GitHub #471 row E5-7): a program
//! runs for each stream. The link crosses a unix socket. The program has an
//! absolute path, no shell, an environment from an allow list and its
//! stderr goes to the log.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};

use serde_json::Value;

/// One install: a state directory and a run directory of its own.
struct Host {
    dir: tempfile::TempDir,
}

impl Host {
    fn new() -> Host {
        Host {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_podnet"));
        cmd.args(args)
            .env("PODNET_STATE_DIR", self.path("state"))
            .env("PODNET_RUN_DIR", self.path("run"))
            .env_remove("PODNET_LINK_CONTROL")
            .env_remove("RUST_LOG");
        cmd
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command(args).output().unwrap()
    }

    fn fingerprint(&self) -> String {
        let out = self.run(&["link", "status", "--create", "--json"]);
        let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
        doc["identity"]["fingerprint"].as_str().unwrap().to_string()
    }
}

struct Proc(Child);

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn start(cmd: &mut Command) -> (Proc, String) {
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.as_mut().unwrap())
        .read_line(&mut line)
        .unwrap();
    if line.is_empty() {
        let mut err = String::new();
        let _ = child.stderr.take().unwrap().read_to_string(&mut err);
        let _ = child.kill();
        panic!("no first line: {err}");
    }
    (Proc(child), line)
}

use std::os::unix::fs::PermissionsExt as _;

/// A script file (a program, not a shell by name) in the install `host`.
fn script(host: &Host, name: &str, body: &str) -> String {
    let path = host.path(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path.to_str().unwrap().to_string()
}

/// A listener with the service `x`: `target` and the extra `flags`.
fn listener(server: &Host, client: &Host, extra: &[&str], envs: &[(&str, &str)]) -> Proc {
    let allow = server.path("allow");
    std::fs::write(&allow, client.fingerprint()).unwrap();
    let sock = server.path("l.sock");
    let mut args = vec![
        "link",
        "listen",
        sock.to_str().unwrap(),
        "--allow",
        allow.to_str().unwrap(),
        "--json",
    ];
    args.extend_from_slice(extra);
    let mut cmd = server.command(&args);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let (l, line) = start(&mut cmd);
    let ready: Value = serde_json::from_str(&line).unwrap();
    assert!(
        ready["services"]["x"]
            .as_str()
            .unwrap()
            .starts_with("exec:"),
        "{ready}"
    );
    l
}

/// Send `input`, close stdin, and give the answer of the service `x`.
fn once(server: &Host, client: &Host, input: &[u8]) -> (String, Option<i32>) {
    let sock = server.path("l.sock");
    let mut dial = client
        .command(&[
            "link",
            "dial",
            sock.to_str().unwrap(),
            "--peer",
            &server.fingerprint(),
            "--service",
            "x",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    dial.stdin.as_mut().unwrap().write_all(input).unwrap();
    drop(dial.stdin.take());
    let mut got = Vec::new();
    dial.stdout.as_mut().unwrap().read_to_end(&mut got).unwrap();
    (text(&got), dial.wait().unwrap().code())
}

#[test]
fn a_program_that_echoes_answers_on_the_stream() {
    let (server, client) = (Host::new(), Host::new());
    let _l = listener(&server, &client, &["--service", "x=exec:/bin/cat"], &[]);
    let (out, code) = once(&server, &client, b"hello exec\n");
    assert_eq!(out, "hello exec\n");
    assert_eq!(code, Some(0));
    // One program for each stream: the second stream is a new process.
    let (out, _) = once(&server, &client, &vec![b'z'; 200_000]);
    assert_eq!(out.len(), 200_000);
}

#[test]
fn the_arguments_are_an_array_and_no_shell_expands_them() {
    let (server, client) = (Host::new(), Host::new());
    let prog = script(&server, "args.sh", "printf '%s|' \"$@\"");
    let service = format!("x=exec:{prog}");
    let _l = listener(
        &server,
        &client,
        &[
            "--service",
            &service,
            "--exec-arg",
            "x=a b",
            "--exec-arg",
            "x=$HOME;ls",
            "--exec-arg",
            "x=*",
        ],
        &[],
    );
    let (out, _) = once(&server, &client, b"");
    assert_eq!(out, "a b|$HOME;ls|*|");
}

#[test]
fn a_relative_path_is_refused() {
    let host = Host::new();
    let allow = host.path("allow");
    std::fs::write(&allow, host.fingerprint()).unwrap();
    for target in [
        "x=exec:cat",
        "x=exec:./cat",
        "x=exec:/bin/sh",
        "x=exec:/no/such/prog",
    ] {
        let sock = host.path("r.sock");
        let out = host.run(&[
            "link",
            "listen",
            sock.to_str().unwrap(),
            "--allow",
            allow.to_str().unwrap(),
            "--service",
            target,
        ]);
        assert_eq!(
            out.status.code(),
            Some(64),
            "{target}: {}",
            text(&out.stderr)
        );
        assert!(!sock.exists(), "{target}: the listener bound a socket");
    }
}

#[test]
fn the_environment_holds_only_the_allowed_names() {
    let (server, client) = (Host::new(), Host::new());
    let prog = script(&server, "env.sh", "exec /usr/bin/env");
    let service = format!("x=exec:{prog}");
    let _l = listener(
        &server,
        &client,
        &[
            "--service",
            &service,
            "--exec-env",
            "x=ALLOWED_ONE",
            "--exec-env",
            "x=ALLOWED_MISSING",
        ],
        &[("ALLOWED_ONE", "one"), ("SECRET_TWO", "two")],
    );
    let (out, code) = once(&server, &client, b"");
    assert_eq!(code, Some(0));
    assert!(out.contains("ALLOWED_ONE=one\n"), "{out}");
    for gone in ["SECRET_TWO", "PODNET_", "HOME=", "PATH=", "ALLOWED_MISSING"] {
        assert!(!out.contains(gone), "{gone} reached the program: {out}");
    }
}

#[test]
fn the_stderr_of_the_program_does_not_reach_the_stream() {
    let (server, client) = (Host::new(), Host::new());
    let prog = script(&server, "err.sh", "echo to-stdout; echo to-stderr >&2");
    let service = format!("x=exec:{prog}");
    let _l = listener(&server, &client, &["--service", &service], &[]);
    let (out, code) = once(&server, &client, b"");
    assert_eq!(out, "to-stdout\n");
    assert_eq!(code, Some(0));
}

#[test]
fn a_program_that_cannot_start_gets_exit_69() {
    let (server, client) = (Host::new(), Host::new());
    // A file with no execute bit and a bad interpreter: the start fails.
    let prog = server.path("bad.sh");
    std::fs::write(&prog, "#!/no/such/interpreter\n").unwrap();
    std::fs::set_permissions(&prog, std::fs::Permissions::from_mode(0o755)).unwrap();
    let service = format!("x=exec:{}", prog.display());
    let _l = listener(&server, &client, &["--service", &service], &[]);
    let (_, code) = once(&server, &client, b"");
    assert_eq!(code, Some(69));
}

#[test]
fn a_user_needs_root() {
    // SAFETY: geteuid has no precondition.
    if unsafe { libc::geteuid() } == 0 {
        eprintln!("skipping, this test runs as root");
        return;
    }
    let host = Host::new();
    let allow = host.path("allow");
    std::fs::write(&allow, host.fingerprint()).unwrap();
    let sock = host.path("u.sock");
    let out = host.run(&[
        "link",
        "listen",
        sock.to_str().unwrap(),
        "--allow",
        allow.to_str().unwrap(),
        "--service",
        "x=exec:/bin/cat",
        "--exec-user",
        "x=nobody",
    ]);
    assert_eq!(out.status.code(), Some(64), "{}", text(&out.stderr));
    assert!(
        text(&out.stderr).contains("needs root"),
        "{}",
        text(&out.stderr)
    );
}
