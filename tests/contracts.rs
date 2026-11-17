//! T-451 (GitHub #641, #642): the contract of the JSON documents of
//! podnet. One table holds each command with `--json` (or `--detach`)
//! that runs on a host with no network; one check applies the rules to
//! each document. `docs/cli.md`, "JSON documents", gives the rules:
//!
//! 1. Shape: the stdout of a document is one JSON object. The stdout of
//!    an event stream (a server command with `--json` in the foreground)
//!    is one JSON object on each line, and each line has a string `event`.
//! 2. Stamp: each object has `schema` (a positive integer) and `podnet`
//!    (the version line of this binary). The exception is an environment
//!    document (`net shim`, `net socksify`, `net passwd-shim`,
//!    `net ts-shims`): each key is a variable name, each value a string.
//! 3. Endpoint: each `ready` event and each `detached` document has `url`
//!    (a string or null), `listen` (a string or null) and `listens` (an
//!    array of strings); `listen` is the first item of `listens`, or null
//!    when `listens` is empty. A command with a local listener has at
//!    least one item in `listens`. A `detached` document also has `name`,
//!    `pid`, `command` and `log`.
//! 4. Secrets: no document holds the value of a secret of the
//!    environment or of the command line (`TUNNEL_TOKEN`, the password of
//!    a proxy), and no key `token`, `password`, `secret`, `passphrase`,
//!    `private_key` or `seed` has a value.
//!
//! Commands that need the network (tunnel, share, demo, ssh-relay, send,
//! receive, relay pair, relay node, warp status, warp exit, warp scan,
//! doctor, connect, link mesh of a node with no announce) are not in the
//! table. A row that needs TCP on the loopback is skipped, with a line,
//! on a host that has none.

mod test_host;

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::Value;

/// The values of the secrets that the commands get.
const TOKEN: &str = "t451-token-of-the-contract";
const PASSWORD: &str = "t451-password-of-the-contract";

/// The keys that must not have a value in a document.
const SECRET_KEYS: &[&str] = &[
    "token",
    "password",
    "secret",
    "passphrase",
    "private_key",
    "seed",
];

/// The form of the output of a row.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Form {
    /// One JSON object, then the command exits 0.
    One,
    /// One JSON object for each line (an event stream); exit 0.
    Lines,
    /// The first line of a command that runs until a signal.
    FirstLine,
    /// The document of a `--detach` start (`event` is `detached`).
    Detached,
    /// An environment document (rule 2, the exception).
    Env,
}

struct Row {
    args: Vec<String>,
    form: Form,
    /// The row binds TCP on 127.0.0.1.
    tcp: bool,
    /// The command has a local listener (rule 3).
    listener: bool,
}

fn row(args: &[&str], form: Form) -> Row {
    Row {
        args: args.iter().map(|a| a.to_string()).collect(),
        form,
        tcp: false,
        listener: false,
    }
}

impl Row {
    fn tcp(mut self) -> Row {
        self.tcp = true;
        self
    }

    fn listener(mut self) -> Row {
        self.listener = true;
        self
    }

    fn name(&self) -> String {
        format!("podnet {}", self.args.join(" "))
    }
}

/// One install: each directory of podnet in a temporary directory.
struct Host {
    dir: tempfile::TempDir,
}

/// The binary under test: `PODNET_TEST_BINARY` (the gate gives the static
/// release binary, T-451 step 3), else the binary that cargo built.
fn binary() -> PathBuf {
    std::env::var_os("PODNET_TEST_BINARY")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_podnet")))
}

impl Host {
    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn arg(&self, name: &str) -> String {
        self.path(name).display().to_string()
    }

    fn command(&self, args: &[String]) -> Command {
        let mut cmd = Command::new(binary());
        let root = self.dir.path();
        cmd.args(args)
            .env("HOME", root)
            .env("XDG_STATE_HOME", root.join("state"))
            .env("XDG_CONFIG_HOME", root.join("config"))
            .env("XDG_RUNTIME_DIR", root.join("runtime"))
            .env("PODNET_RUN_DIR", root.join("run"))
            .env("PODNET_STATE_DIR", root.join("pstate"))
            .env("TUNNEL_TOKEN", TOKEN)
            .env("HTTPS_PROXY", format!("http://user:{PASSWORD}@127.0.0.1:9"))
            .env("NO_PROXY", "127.0.0.1,localhost")
            .env_remove("ALL_PROXY")
            .env_remove("PODNET_PROXY")
            .env_remove("RUST_LOG")
            .env_remove("PODNET_ERROR_PROGRAM")
            .env_remove("PODNET_EDGE")
            .stdin(Stdio::null());
        cmd
    }

    fn output(&self, args: &[&str]) -> Output {
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        self.command(&args).output().unwrap()
    }

    /// Run `row`; give its stdout.
    fn run(&self, row: &Row) -> String {
        if row.form == Form::FirstLine {
            return self.first_line(row);
        }
        let out = self.command(&row.args).output().unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        assert_eq!(
            out.status.code(),
            Some(0),
            "{}: {}{stdout}",
            row.name(),
            String::from_utf8_lossy(&out.stderr)
        );
        stdout
    }

    fn first_line(&self, row: &Row) -> String {
        let mut child = self
            .command(&row.args)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut line = String::new();
        let read = BufReader::new(child.stdout.as_mut().unwrap()).read_line(&mut line);
        unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
        let _ = child.wait();
        read.unwrap();
        line
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.output(&["stop", "--all"]);
    }
}

/// The objects of the output of `row` (rule 1).
fn documents(row: &Row, stdout: &str) -> Vec<Value> {
    let name = row.name();
    let parse = |text: &str| -> Value {
        let doc: Value = serde_json::from_str(text)
            .unwrap_or_else(|e| panic!("{name}: not one JSON value ({e}): {stdout}"));
        assert!(doc.is_object(), "{name}: not a JSON object: {stdout}");
        doc
    };
    match row.form {
        Form::Lines | Form::FirstLine => {
            let docs: Vec<Value> = stdout
                .lines()
                .filter(|l| !l.trim().is_empty())
                .map(parse)
                .collect();
            assert!(!docs.is_empty(), "{name}: no line");
            for doc in &docs {
                assert!(doc["event"].is_string(), "{name}: no event: {doc}");
            }
            docs
        }
        _ => vec![parse(stdout.trim())],
    }
}

/// Rules 2, 3 and 4 for one document of `row`.
fn check(row: &Row, doc: &Value, text: &str) {
    let name = row.name();
    // Rule 4.
    for secret in [TOKEN, PASSWORD] {
        assert!(!text.contains(secret), "{name}: a secret: {text}");
    }
    secret_keys(&name, doc);
    // Rule 2.
    let object = doc.as_object().unwrap();
    if row.form == Form::Env {
        for (key, value) in object {
            let variable = !key.is_empty()
                && !key.starts_with(|c: char| c.is_ascii_digit())
                && key
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
            assert!(variable && value.is_string(), "{name}: {key}: {doc}");
        }
        return;
    }
    assert!(
        doc["schema"].as_u64().is_some_and(|s| s >= 1),
        "{name}: schema: {doc}"
    );
    assert!(
        doc["podnet"]
            .as_str()
            .is_some_and(|v| v.starts_with(env!("CARGO_PKG_VERSION"))),
        "{name}: podnet: {doc}"
    );
    // Rule 3.
    let event = doc["event"].as_str();
    if row.form == Form::Detached {
        assert_eq!(event, Some("detached"), "{name}: {doc}");
        for key in ["name", "command", "log"] {
            assert!(doc[key].is_string(), "{name}: {key}: {doc}");
        }
        assert!(doc["pid"].is_u64(), "{name}: pid: {doc}");
    }
    if !matches!(event, Some("ready" | "detached")) {
        return;
    }
    for key in ["url", "listen"] {
        assert!(
            object
                .get(key)
                .is_some_and(|v| v.is_string() || v.is_null()),
            "{name}: {key}: {doc}"
        );
    }
    let listens = doc["listens"]
        .as_array()
        .unwrap_or_else(|| panic!("{name}: listens: {doc}"));
    assert!(listens.iter().all(Value::is_string), "{name}: {doc}");
    let first = listens.first().cloned().unwrap_or(Value::Null);
    assert_eq!(doc["listen"], first, "{name}: listen: {doc}");
    if row.listener {
        assert!(!listens.is_empty(), "{name}: no listener: {doc}");
    }
}

fn secret_keys(name: &str, value: &Value) {
    match value {
        Value::Object(map) => {
            for (key, item) in map {
                let has = !item.is_null() && item != "";
                assert!(
                    !(has && SECRET_KEYS.contains(&key.as_str())),
                    "{name}: the key {key} has a value"
                );
                secret_keys(name, item);
            }
        }
        Value::Array(items) => items.iter().for_each(|i| secret_keys(name, i)),
        _ => {}
    }
}

/// Run each row of `rows` and check its documents; add each broken rule
/// to `faults`. A TCP row on a host with no loopback is skipped with a
/// line.
fn run_rows(host: &Host, rows: &[Row], faults: &mut Vec<String>) -> Vec<Vec<Value>> {
    let tcp = test_host::loopback();
    let mut out = Vec::new();
    for row in rows {
        if let (true, Err(why)) = (row.tcp, &tcp) {
            test_host::skip_line(&format!("{}: {why}", row.name()));
            out.push(Vec::new());
            continue;
        }
        let checked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let stdout = host.run(row);
            let docs = documents(row, &stdout);
            for doc in &docs {
                check(row, doc, &stdout);
            }
            docs
        }));
        match checked {
            Ok(docs) => out.push(docs),
            Err(panic) => {
                let text = panic
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_default();
                faults.push(text.chars().take(600).collect());
                out.push(Vec::new());
            }
        }
    }
    out
}

fn fingerprint(host: &Host) -> String {
    let out = host.output(&["link", "status", "--create", "--json"]);
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    doc["identity"]["fingerprint"].as_str().unwrap().to_string()
}

fn setup(host: &Host) -> String {
    std::fs::create_dir_all(host.path("site")).unwrap();
    std::fs::write(host.path("site/index.html"), "contract\n").unwrap();
    let out = host.output(&["git", "init", "--root", &host.arg("repos"), "x"]);
    assert!(out.status.success(), "{out:?}");
    let route = "networks:\n  home: direct\ndoors:\n  - listen: tcp://127.0.0.1:0\n    via: home\n";
    std::fs::write(host.path("r.yml"), route).unwrap();
    // An autostart entry with TUNNEL_TOKEN in its environment file.
    let out = host.output(&["autostart", "add", "--manual", "web", "--", "status"]);
    assert!(out.status.success(), "{out:?}");
    let fp = fingerprint(host);
    std::fs::write(host.path("allow"), format!("{fp}\n")).unwrap();
    fp
}

fn path_of(path: &Path) -> &str {
    path.to_str().unwrap()
}

/// Rules 1 to 4 for each command of the table.
#[test]
fn each_json_document_keeps_the_contract() {
    let host = Host {
        dir: tempfile::tempdir().unwrap(),
    };
    let fp = setup(&host);
    let (site, repos, allow) = (host.arg("site"), host.arg("repos"), host.arg("allow"));
    let (sock, sock2) = (host.path("l.sock"), host.path("f.sock"));
    let (sock, sock2) = (path_of(&sock), path_of(&sock2));
    let switch = format!("unix:{}", host.arg("sw.sock"));
    let git_local = format!("unix:{}", host.arg("git.sock"));
    let git_fg = format!("unix:{}", host.arg("git2.sock"));
    let proxy = format!("http://user:{PASSWORD}@127.0.0.1:9");
    let route = host.arg("r.yml");
    use Form::*;
    let listen = ["link", "listen", sock, "--allow", allow.as_str()];
    let listen_detach = [&listen[..], &["--detach", "--name", "lk", "--no-restart"]].concat();
    let git_detach = [
        "git",
        "serve",
        "--root",
        &repos,
        "--local",
        &git_local,
        "--anon-read",
        "--detach",
        "--name",
        "g1",
        "--no-restart",
    ];
    let rows = vec![
        row(&["status"], One),
        row(&["ps", "--json"], One),
        row(&["components", "--json"], One),
        row(&["ports", "--json"], One),
        row(&["ports", "--probe", "--json"], One).tcp(),
        row(&["link", "carriers", "--json"], One),
        row(&["link", "status", "--json"], One),
        row(&["autostart", "list", "--json"], One),
        row(&["git", "urls", "--root", &repos, "--json"], One),
        row(&["agent", "status", "--json"], One),
        row(&["net", "addresses", "--json"], One),
        row(&["net", "doctor", "--json"], One),
        row(&["man", "--json"], One),
        row(&["man", "--json", "tunnel"], One),
        row(&["man", "--json", "exit-status"], One),
        row(&["route", "--help", "--json"], One),
        row(
            &[
                "serve",
                "--check",
                "--json",
                "--local",
                "127.0.0.1:0",
                "--dir",
                &site,
            ],
            One,
        )
        .tcp(),
        row(&["net", "shim", "--json"], Env),
        row(&["net", "socksify", "--json"], Env),
        row(&["net", "passwd-shim", "--json"], Env),
        row(&["net", "ts-shims", "--json"], Env),
        row(
            &[
                "serve",
                "--local",
                "127.0.0.1:0",
                "--dir",
                &site,
                "--detach",
                "--name",
                "s1",
                "--no-restart",
            ],
            Detached,
        )
        .tcp()
        .listener(),
        row(
            &[
                "net",
                "proxy",
                "--listen",
                "tcp://127.0.0.1:0",
                "--proxy",
                &proxy,
                "--detach",
                "--name",
                "np",
                "--no-restart",
            ],
            Detached,
        )
        .tcp()
        .listener(),
        row(
            &[
                "net",
                "switch",
                "--listen",
                &switch,
                "--detach",
                "--name",
                "sw",
                "--no-restart",
            ],
            Detached,
        )
        .listener(),
        row(&listen_detach, Detached).listener(),
        row(
            &[
                "route",
                "--config",
                &route,
                "--detach",
                "--name",
                "r1",
                "--no-restart",
            ],
            Detached,
        )
        .tcp()
        .listener(),
        row(&git_detach, Detached).listener(),
        row(
            &[
                "serve",
                "--local",
                "127.0.0.1:0",
                "--dir",
                &site,
                "--json",
                "--run-for",
                "1",
            ],
            Lines,
        )
        .tcp()
        .listener(),
        row(
            &[
                "git",
                "serve",
                "--root",
                &repos,
                "--local",
                &git_fg,
                "--anon-read",
                "--json",
                "--run-for",
                "1",
            ],
            Lines,
        )
        .listener(),
        row(
            &["link", "listen", sock2, "--allow", &allow, "--json"],
            FirstLine,
        )
        .listener(),
    ];
    // Each fault is in the list at the end; the hook of the panic of a row
    // stays silent.
    std::panic::set_hook(Box::new(|_| {}));
    let mut faults = Vec::new();
    let docs = run_rows(&host, &rows, &mut faults);
    let s1 = rows.iter().position(|r| r.args.iter().any(|a| a == "s1"));
    let served = s1
        .and_then(|i| docs[i].first())
        .map(|d| d["url"].as_str().unwrap().to_string());

    let mut rows = vec![
        row(&["wait", "lk"], One),
        row(&["wait", "g1"], One),
        // A second start with the same arguments prints the same document.
        row(&git_detach, Detached).listener(),
        row(
            &["link", "dial", sock, "--peer", &fp, "--check", "--json"],
            One,
        ),
        row(
            &["link", "connect", sock, "--peer", &fp, "--check", "--json"],
            One,
        ),
        row(
            &[
                "link",
                "connect",
                sock,
                "--peer",
                &fp,
                "--detach",
                "--name",
                "lc",
                "--no-restart",
            ],
            Detached,
        ),
        row(
            &["link", "connect", sock, "--peer", &fp, "--json"],
            FirstLine,
        ),
        row(&["ps", "--json"], One),
        row(&["link", "status", "--json"], One),
    ];
    if let Some(url) = &served {
        rows.push(row(&["bench", url, "--json", "--duration", "1"], One).tcp());
    }
    let docs = run_rows(&host, &rows, &mut faults);
    // The `wait` documents are the detached documents again.
    for doc in docs[..2].iter().filter_map(|d| d.first()) {
        if doc["event"] != "detached" {
            faults.push(format!("wait: not the detached document: {doc}"));
        }
    }
    let _ = std::panic::take_hook();
    assert!(
        faults.is_empty(),
        "{} faults:\n{}",
        faults.len(),
        faults.join("\n")
    );
}
