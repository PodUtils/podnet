//! `serve DIR --at peer:FP/SERVICE` (T-297, GitHub #471 row E5-8): the file
//! server answers on a stream of a link. The link crosses a unix socket.

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

/// A file server at a peer: the tree, the server install, the client
/// install and the running `serve`.
struct At {
    server: Host,
    client: Host,
    server_fp: String,
    _serve: Proc,
}

fn at(extra: &[&str]) -> At {
    let (server, client) = (Host::new(), Host::new());
    let (sfp, cfp) = (server.fingerprint(), client.fingerprint());
    let tree = server.path("tree");
    std::fs::create_dir_all(tree.join("sub")).unwrap();
    std::fs::write(tree.join("a.txt"), "0123456789abcdef").unwrap();
    std::fs::write(tree.join("sub").join("inner.txt"), "inner").unwrap();
    std::fs::create_dir_all(tree.join("site")).unwrap();
    std::fs::write(tree.join("site").join("index.html"), "<p>the index</p>").unwrap();
    let sock = server.path("l.sock");
    let at = format!("peer:{cfp}/files");
    let mut args = vec![
        "serve",
        tree.to_str().unwrap(),
        "--at",
        &at,
        "--at-listen",
        sock.to_str().unwrap(),
    ];
    args.extend_from_slice(extra);
    let mut cmd = server.command(&args);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().unwrap();
    // The listener prints `link listening` when it is bound.
    let mut lines = BufReader::new(child.stdout.take().unwrap());
    let mut seen = String::new();
    loop {
        let mut line = String::new();
        if lines.read_line(&mut line).unwrap() == 0 {
            let mut err = String::new();
            let _ = child.stderr.take().unwrap().read_to_string(&mut err);
            let _ = child.kill();
            panic!("serve ended: {seen} {err}");
        }
        seen.push_str(&line);
        if line.contains("link listening") {
            break;
        }
    }
    // Keep the pipe open and drained, so that the server never blocks.
    std::thread::spawn(move || {
        let mut sink = String::new();
        let _ = lines.read_to_string(&mut sink);
    });
    At {
        server,
        client,
        server_fp: sfp,
        _serve: Proc(child),
    }
}

impl At {
    /// Send one HTTP request on a stream of the service `files` and give
    /// the whole answer.
    fn request(&self, line: &str) -> String {
        self.request_with(line, "")
    }

    fn request_with(&self, line: &str, header: &str) -> String {
        let sock = self.server.path("l.sock");
        let mut dial = self
            .client
            .command(&[
                "link",
                "dial",
                sock.to_str().unwrap(),
                "--peer",
                &self.server_fp,
                "--service",
                "files",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let request =
            format!("{line} HTTP/1.1\r\nHost: files\r\nConnection: close\r\n{header}\r\n");
        dial.stdin
            .as_mut()
            .unwrap()
            .write_all(request.as_bytes())
            .unwrap();
        let mut got = Vec::new();
        dial.stdout.as_mut().unwrap().read_to_end(&mut got).unwrap();
        drop(dial.stdin.take());
        let _ = dial.wait();
        text(&got)
    }
}

fn body(answer: &str) -> &str {
    answer.split_once("\r\n\r\n").map_or("", |(_, b)| b)
}

#[test]
fn a_get_of_a_file_answers_the_bytes() {
    let at = at(&[]);
    let answer = at.request("GET /a.txt");
    assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
    assert_eq!(body(&answer), "0123456789abcdef");
    let answer = at.request("GET /sub/inner.txt");
    assert_eq!(body(&answer), "inner");
}

#[test]
fn a_directory_with_an_index_answers_the_index() {
    let at = at(&[]);
    let answer = at.request("GET /site/");
    assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
    assert!(body(&answer).contains("the index"), "{answer}");
}

#[test]
fn a_range_answers_206_with_the_bytes_of_the_range() {
    let at = at(&[]);
    let answer = at.request_with("GET /a.txt", "Range: bytes=2-5\r\n");
    assert!(answer.starts_with("HTTP/1.1 206"), "{answer}");
    assert!(
        answer
            .to_lowercase()
            .contains("content-range: bytes 2-5/16"),
        "{answer}"
    );
    assert_eq!(body(&answer), "2345");
}

#[test]
fn a_head_answers_the_head_and_no_body() {
    let at = at(&[]);
    let answer = at.request("HEAD /a.txt");
    assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
    assert!(
        answer.to_lowercase().contains("content-length: 16"),
        "{answer}"
    );
    assert_eq!(body(&answer), "");
}

#[test]
fn a_directory_has_no_listing_by_default() {
    let at = at(&[]);
    for path in ["GET /", "GET /sub/"] {
        let answer = at.request(path);
        assert!(!answer.contains("inner.txt"), "{path}: {answer}");
        assert!(!answer.contains("a.txt"), "{path}: {answer}");
        assert!(answer.starts_with("HTTP/1.1 404"), "{path}: {answer}");
    }
}

#[test]
fn a_listing_comes_only_with_list() {
    let at = at(&["--list"]);
    let answer = at.request("GET /sub/");
    assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
    assert!(answer.contains("inner.txt"), "{answer}");
}

#[test]
fn a_peer_that_is_not_the_one_of_at_is_refused() {
    let at = at(&[]);
    let stranger = Host::new();
    let sock = at.server.path("l.sock");
    let out = stranger.run(&[
        "link",
        "dial",
        sock.to_str().unwrap(),
        "--peer",
        &at.server_fp,
        "--service",
        "files",
        "--check",
    ]);
    assert_eq!(out.status.code(), Some(77), "{}", text(&out.stderr));
}

#[test]
fn at_needs_at_listen_and_a_good_spec() {
    let host = Host::new();
    let tree = host.path("tree");
    std::fs::create_dir_all(&tree).unwrap();
    let t = tree.to_str().unwrap();
    for args in [
        vec!["serve", t, "--at", "peer:AB/files"],
        vec!["serve", t, "--at", "AB/files", "--at-listen", "x.sock"],
        vec!["serve", t, "--at", "peer:AB", "--at-listen", "x.sock"],
    ] {
        let out = host.run(&args);
        assert_eq!(
            out.status.code(),
            Some(64),
            "{args:?}: {}",
            text(&out.stderr)
        );
    }
}
