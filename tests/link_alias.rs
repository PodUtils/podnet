//! Names for live links (T-308, GitHub #476 row E10-3). The links cross a
//! unix socket, so no test needs a TCP loopback.

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
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

    fn allow(&self, name: &str, lines: &[&str]) -> PathBuf {
        let path = self.path(name);
        std::fs::write(&path, lines.join("\n")).unwrap();
        path
    }
}

struct Proc(Child);

impl Proc {
    fn kill(mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Start `cmd` and read its first line of stdout.
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

/// A listener at `sock` under the name `name`.
fn listen(host: &Host, sock: &Path, allow: &Path, name: Option<&str>) -> (Proc, String) {
    let mut args = vec![
        "link",
        "listen",
        sock.to_str().unwrap(),
        "--allow",
        allow.to_str().unwrap(),
        "--json",
    ];
    if let Some(name) = name {
        args.extend(["--name", name]);
    }
    start(&mut host.command(&args))
}

fn connect_args<'a>(sock: &'a str, trust: &'a [&'a str], name: &'a str) -> Vec<&'a str> {
    let mut args = vec!["link", "connect", sock];
    args.extend(trust);
    args.extend(["--name", name, "--json"]);
    args
}

#[test]
fn a_bad_name_exits_64() {
    let host = Host::new();
    let allow = host.allow("allow", &[]);
    let sock = host.path("l.sock");
    let long = "a".repeat(33);
    for bad in ["Web", "a_b", "a.b", long.as_str()] {
        let out = host.run(&[
            "link",
            "listen",
            sock.to_str().unwrap(),
            "--allow",
            allow.to_str().unwrap(),
            "--name",
            bad,
        ]);
        assert_eq!(out.status.code(), Some(64), "{bad}: {}", text(&out.stderr));
        assert!(text(&out.stderr).contains("a-z 0-9 -"), "{bad}");
    }
}

#[test]
fn a_duplicate_name_exits_64_and_ps_shows_the_name() {
    let host = Host::new();
    let allow = host.allow("allow", &[]);
    let (_first, _) = listen(&host, &host.path("a.sock"), &allow, Some("web-1"));
    let out = host.run(&[
        "link",
        "listen",
        host.path("b.sock").to_str().unwrap(),
        "--allow",
        allow.to_str().unwrap(),
        "--name",
        "web-1",
    ]);
    assert_eq!(out.status.code(), Some(64), "{}", text(&out.stderr));
    assert!(text(&out.stderr).contains("web-1"));
    let ps = host.run(&["ps"]);
    assert!(text(&ps.stdout).contains("web-1"), "{}", text(&ps.stdout));
    let status = host.run(&["link", "status", "web-1", "--json"]);
    let doc: Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(doc["links"][0]["name"], "web-1", "{doc}");
}

#[test]
fn a_name_does_not_replace_the_peer_key() {
    let host = Host::new();
    let out = host.run(&["link", "connect", "x.sock", "--name", "web"]);
    assert_eq!(out.status.code(), Some(64), "{}", text(&out.stderr));
    assert!(
        text(&out.stderr).contains("--peer"),
        "{}",
        text(&out.stderr)
    );
}

#[test]
fn a_name_keeps_its_fingerprint_and_a_changed_key_exits_77() {
    let (a, b, client) = (Host::new(), Host::new(), Host::new());
    let (afp, bfp, cfp) = (a.fingerprint(), b.fingerprint(), client.fingerprint());
    let sock = a.path("l.sock");
    let sock_text = sock.to_str().unwrap().to_string();
    let (listener_a, _) = listen(&a, &sock, &a.allow("allow", &[&cfp]), None);
    let both = client.allow("both", &[&afp, &bfp]);
    let trust = ["--allow", both.to_str().unwrap()];

    let (first, line) = start(&mut client.command(&connect_args(&sock_text, &trust, "web")));
    let doc: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(doc["peer"], afp.as_str(), "{doc}");
    // The process ends with no clean stop: its record and its binding stay.
    first.kill();
    let stored = client.path("run/web.peer");
    assert_eq!(std::fs::read_to_string(&stored).unwrap().trim(), afp);

    // A reconnect to the same peer passes.
    let (second, _) = start(&mut client.command(&connect_args(&sock_text, &trust, "web")));
    second.kill();

    // The key at the address changes: the name stays with the old key.
    listener_a.kill();
    let (_listener_b, _) = listen(&b, &sock, &b.allow("allow", &[&cfp]), None);
    let out = client
        .command(&connect_args(&sock_text, &trust, "web"))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(77), "{}", text(&out.stderr));
    assert!(
        text(&out.stderr).contains("key changed"),
        "{}",
        text(&out.stderr)
    );
    assert_eq!(std::fs::read_to_string(&stored).unwrap().trim(), afp);
    // Another name takes the new key.
    let (third, line) = start(&mut client.command(&connect_args(&sock_text, &trust, "web-2")));
    let doc: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(doc["peer"], bfp.as_str(), "{doc}");
    third.kill();
}
