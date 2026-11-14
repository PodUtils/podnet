//! `podnet send` and `podnet receive` through the binary, on the loopback
//! (T-105): a stand-in mailbox of magic-wormhole runs in this process, and
//! two processes of podnet meet at it. The sender prints the code as JSON;
//! the receiver reads it from stdin.

#[path = "../transfer_harness/mod.rs"]
mod harness;

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};

use sha2::{Digest, Sha256};

use crate::test_host;

const PROXIES: [&str; 7] = [
    "HTTPS_PROXY",
    "https_proxy",
    "HTTP_PROXY",
    "http_proxy",
    "ALL_PROXY",
    "all_proxy",
    "PODNET_PROXY",
];

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
}

fn podnet(args: &[&str]) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_podnet"));
    cmd.args(args)
        .env_remove("PODNET_MAILBOX")
        .env("NO_PROXY", "*");
    for key in PROXIES {
        cmd.env_remove(key);
    }
    cmd
}

/// The flags of an end at `mailbox`, on the loopback, with no relay.
fn at(mailbox: &str) -> Vec<String> {
    [
        "--mailbox",
        mailbox,
        "--listen",
        "127.0.0.1:0",
        "--transit-relay",
        "none",
        "--json",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// A sender of `what` (a path, or `--text`), and the code it printed.
fn sender(mailbox: &str, what: &[&str]) -> (Child, String, BufReader<std::process::ChildStdout>) {
    let mut args: Vec<String> = vec!["send".into()];
    args.extend(what.iter().map(|s| s.to_string()));
    args.extend(at(mailbox));
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let mut child = podnet(&refs)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    out.read_line(&mut line).unwrap();
    let event: serde_json::Value =
        serde_json::from_str(&line).unwrap_or_else(|_| panic!("a JSON line: {line:?}"));
    (
        child,
        event["code"].as_str().expect("the code").to_string(),
        out,
    )
}

/// A receiver at `mailbox` that reads `code` on stdin, with `more` flags.
fn receiver(mailbox: &str, code: &str, more: &[&str]) -> Output {
    let mut args: Vec<String> = vec!["receive".into()];
    args.extend(at(mailbox));
    args.extend(more.iter().map(|s| s.to_string()));
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let mut child = podnet(&refs)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    writeln!(child.stdin.take().unwrap(), "{code}").unwrap();
    child.wait_with_output().unwrap()
}

fn finish(
    mut child: Child,
    mut out: BufReader<std::process::ChildStdout>,
) -> (Option<i32>, String, String) {
    let mut rest = String::new();
    std::io::Read::read_to_string(&mut out, &mut rest).unwrap();
    let mut err = String::new();
    std::io::Read::read_to_string(&mut child.stderr.take().unwrap(), &mut err).unwrap();
    (child.wait().unwrap().code(), rest, err)
}

fn sha256(path: &Path) -> String {
    pod_wormhole::hex(&Sha256::digest(std::fs::read(path).unwrap()))
}

fn text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
fn send_and_receive_on_the_loopback() {
    test_host::need_loopback!();
    let rt = runtime();
    let mailbox = rt.block_on(harness::Mailbox::start());
    let (src, out) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());

    // A file.
    let file = src.path().join("photo.raw");
    std::fs::write(
        &file,
        (0..400_000u32)
            .map(|i| (i % 253) as u8)
            .collect::<Vec<u8>>(),
    )
    .unwrap();
    let (child, code, rest) = sender(&mailbox.url, &[file.to_str().unwrap()]);
    assert!(code.split('-').count() >= 3, "{code}");
    let got = receiver(
        &mailbox.url,
        &code,
        &["--yes", "--out", out.path().to_str().unwrap()],
    );
    assert_eq!(got.status.code(), Some(0), "{}", text(&got));
    let (status, said, err) = finish(child, rest);
    assert_eq!(status, Some(0), "{said}{err}");
    assert_eq!(sha256(&out.path().join("photo.raw")), sha256(&file));
    let sent: serde_json::Value = serde_json::from_str(said.trim()).unwrap();
    assert_eq!(sent["sha256"].as_str(), Some(sha256(&file).as_str()));

    // A directory.
    let dir = src.path().join("tree");
    std::fs::create_dir_all(dir.join("x/y")).unwrap();
    std::fs::write(dir.join("x/y/z.txt"), b"zed").unwrap();
    let (child, code, rest) = sender(&mailbox.url, &[dir.to_str().unwrap()]);
    let got = receiver(
        &mailbox.url,
        &code,
        &["--yes", "--out", out.path().to_str().unwrap()],
    );
    assert_eq!(got.status.code(), Some(0), "{}", text(&got));
    assert_eq!(finish(child, rest).0, Some(0));
    assert_eq!(
        std::fs::read(out.path().join("tree/x/y/z.txt")).unwrap(),
        b"zed"
    );

    // A text, with no --yes.
    let (child, code, rest) = sender(&mailbox.url, &["--text", "hello there"]);
    let got = receiver(&mailbox.url, &code, &[]);
    assert_eq!(got.status.code(), Some(0), "{}", text(&got));
    let event: serde_json::Value = serde_json::from_slice(&got.stdout).unwrap();
    assert_eq!(event["text"], "hello there");
    assert_eq!(finish(child, rest).0, Some(0));
}

#[test]
fn a_wrong_code_ends_the_code() {
    test_host::need_loopback!();
    let rt = runtime();
    let mailbox = rt.block_on(harness::Mailbox::start());
    let out = tempfile::tempdir().unwrap();
    let (child, code, rest) = sender(&mailbox.url, &["--text", "secret"]);
    let (head, last) = code.rsplit_once('-').unwrap();
    let other = if last == pod_wormhole::words::ODD[0] {
        pod_wormhole::words::ODD[1]
    } else {
        pod_wormhole::words::ODD[0]
    };
    let got = receiver(
        &mailbox.url,
        &format!("{head}-{other}"),
        &["--out", out.path().to_str().unwrap()],
    );
    assert_eq!(got.status.code(), Some(77), "{}", text(&got));
    let (status, said, err) = finish(child, rest);
    assert_eq!(status, Some(77), "{said}{err}");
    assert!(err.contains("the code is spent"), "{err}");
    // The right code after the wrong one finds no giver.
    let again = receiver(
        &mailbox.url,
        &code,
        &["--out", out.path().to_str().unwrap()],
    );
    assert_eq!(again.status.code(), Some(69), "{}", text(&again));
}

#[test]
fn receive_takes_no_code_from_the_command_line() {
    let out = podnet(&["receive", "7-guitarist-revenge"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(64), "{}", text(&out));
}

#[test]
fn receive_of_a_file_needs_yes_when_stdin_is_not_a_terminal() {
    test_host::need_loopback!();
    let rt = runtime();
    let mailbox = rt.block_on(harness::Mailbox::start());
    let (src, out) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let file = src.path().join("f");
    std::fs::write(&file, b"x").unwrap();
    let (child, code, rest) = sender(&mailbox.url, &[file.to_str().unwrap()]);
    let got = receiver(
        &mailbox.url,
        &code,
        &["--out", out.path().to_str().unwrap()],
    );
    assert_eq!(got.status.code(), Some(64), "{}", text(&got));
    assert!(text(&got).contains("give --yes"), "{}", text(&got));
    let (status, _, err) = finish(child, rest);
    assert_eq!(status, Some(1), "{err}");
    assert!(err.contains("the receiver refused the transfer"), "{err}");
    assert_eq!(std::fs::read_dir(out.path()).unwrap().count(), 0);
}

#[test]
fn receive_refuses_a_name_that_exists_with_65() {
    test_host::need_loopback!();
    let rt = runtime();
    let mailbox = rt.block_on(harness::Mailbox::start());
    let (src, out) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    std::fs::write(src.path().join("f"), b"new").unwrap();
    std::fs::write(out.path().join("f"), b"old").unwrap();
    let (child, code, rest) = sender(&mailbox.url, &[src.path().join("f").to_str().unwrap()]);
    let got = receiver(
        &mailbox.url,
        &code,
        &["--yes", "--out", out.path().to_str().unwrap()],
    );
    assert_eq!(got.status.code(), Some(65), "{}", text(&got));
    assert_eq!(finish(child, rest).0, Some(1));
    assert_eq!(std::fs::read(out.path().join("f")).unwrap(), b"old");
}

// T-338: `--words N` gives a code of N words after its number; a count out
// of 2 to 8 exits 64 before any mailbox.
#[test]
fn send_words_sets_the_length_of_the_code() {
    test_host::need_loopback!();
    let rt = runtime();
    let mailbox = rt.block_on(harness::Mailbox::start());
    let out = tempfile::tempdir().unwrap();
    for words in [2usize, 8] {
        let n = words.to_string();
        let (child, code, rest) = sender(&mailbox.url, &["--text", "short or long", "--words", &n]);
        assert_eq!(code.split('-').count(), words + 1, "{code}");
        let got = receiver(
            &mailbox.url,
            &code,
            &["--out", out.path().to_str().unwrap()],
        );
        assert_eq!(got.status.code(), Some(0), "{}", text(&got));
        assert!(text(&got).contains("short or long"), "{}", text(&got));
        assert_eq!(finish(child, rest).0, Some(0));
    }
    for bad in ["1", "9", "0"] {
        let got = podnet(&[
            "send",
            "--text",
            "x",
            "--words",
            bad,
            "--mailbox",
            &mailbox.url,
        ])
        .output()
        .unwrap();
        assert_eq!(got.status.code(), Some(64), "--words {bad}: {}", text(&got));
    }
    assert_eq!(mailbox.nameplates(), 0, "no code was made");
}
