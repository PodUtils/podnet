//! EGRESS-1 to EGRESS-5 and RUN-1: a destination is classified before a
//! socket, a refusal names its reason, and a shadowed rule is named.
//!
//! The first check is a connector that counts its calls: a refused
//! destination makes none. The second is a scan of the source tree: every
//! construct that dials a destination (`TcpStream::connect`,
//! `egress::dial_addrs`, `TcpSocket::new_*`, `UdpSocket::connect`) must be
//! preceded by `policy::permit`, and a local bind or a reply may instead
//! carry `egress-exempt:` with its reason. A self-written exemption does
//! not excuse a dial, so the scan cannot be silenced the way the old one
//! was.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use podnet::policy::{self, Connector, Destination, Policy};

struct Counting(AtomicUsize);

impl Connector for Counting {
    fn connect(&self, _destination: &Destination) -> Result<(), String> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[test]
fn a_destination_that_matches_no_rule_is_refused_before_a_connect() {
    let policy = Policy::parse(r#"{"default":"refuse","rules":[]}"#).unwrap();
    let counting = Counting(AtomicUsize::new(0));
    let err =
        policy::dial_with(&policy, &Destination::new("example.net", 443), &counting).unwrap_err();
    assert!(err.reason.contains("no rule matches"), "{err}");
    assert_eq!(counting.0.load(Ordering::SeqCst), 0);
}

#[test]
fn a_queue_and_a_refusal_are_not_a_timeout() {
    for action in ["refuse", "queue"] {
        let text = format!(r#"{{"default":"{action}","rules":[]}}"#);
        let policy = Policy::parse(&text).unwrap();
        let verdict = policy.classify(&Destination::new("example.net", 443));
        let reason = verdict.reason.expect("a reason");
        assert!(reason.contains("before any connect"), "{reason}");
        assert!(!reason.contains("time"), "{reason}");
    }
}

#[test]
fn the_shadowed_rule_is_named_and_the_command_exits_78() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("rules.json");
    std::fs::write(
        &file,
        r#"{"default":"refuse","rules":[
            {"name":"all","host":"*","port":"*","action":"proxy"},
            {"name":"later","host":"api.example","port":"443","action":"direct"}
        ]}"#,
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(["rules", "check", file.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(podnet::exit::CONFIG));
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(text.contains("later"), "{text}");
    assert!(text.contains("all"), "{text}");
    assert!(text.contains("unreachable"), "{text}");

    std::fs::write(
        &file,
        r#"{"default":"refuse","rules":[
            {"name":"api","host":"api.example","port":"443","action":"direct"}
        ]}"#,
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(["rules", "check", file.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn source_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            if path.file_name().is_some_and(|name| name == "tests") {
                continue;
            }
            source_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            let name = path.file_name().unwrap().to_string_lossy();
            if name == "tests.rs" || name.ends_with("_tests.rs") {
                continue;
            }
            out.push(path);
        }
    }
}

/// A construct that opens a connection to a destination. No self-written
/// exemption excuses one of these: the scan requires the policy call.
const DIALS: &[(&str, &str)] = &[
    ("TcpStream::connect", "TcpStream::connect"),
    ("TcpStream :: connect", "TcpStream::connect"),
    ("egress::dial_addrs(", "egress::dial_addrs"),
    ("dial_addrs(addrs", "dial_addrs"),
    ("UdpSocket::connect", "UdpSocket::connect"),
    ("socket.connect(", "socket.connect"),
];

/// A construct that makes a local socket or answers the peer that sent a
/// datagram. It may carry `egress-exempt:` with its reason instead of a
/// policy call.
const LOCALS: &[(&str, &str)] = &[
    ("TcpSocket::new_v4", "TcpSocket::new_v4"),
    ("TcpSocket::new_v6", "TcpSocket::new_v6"),
    ("UdpSocket::bind", "UdpSocket::bind"),
    ("send_to(", "send_to"),
];

/// How far above a construct its justification may sit.
const WINDOW: usize = 12;

fn justified(lines: &[&str], index: usize, exemption: bool) -> bool {
    let start = index.saturating_sub(WINDOW);
    let near = &lines[start..=index];
    near.iter().any(|line| {
        line.contains("policy::permit")
            || line.contains("permit_fault")
            || line.contains("permit_io")
            || line.contains("permit_failure")
            || (exemption && line.contains("egress-exempt:"))
    })
}

/// Every unexplained direct egress in `root`, as `path:line: reason`.
fn scan(root: &Path) -> Vec<String> {
    let mut files = Vec::new();
    source_files(root, &mut files);
    let mut faults = Vec::new();
    for path in files {
        let text = std::fs::read_to_string(&path).unwrap();
        // A file that is itself test-only reaches no path of the binary.
        if text
            .lines()
            .take(8)
            .any(|line| line.trim() == "#![cfg(test)]")
        {
            continue;
        }
        let lines: Vec<&str> = text.lines().collect();
        // A `#[cfg(test)]` module connects to its own listener: it is not a
        // path of the binary. The scan tracks the braces so that the module
        // and each nested block stay outside the check.
        let mut depth: i64 = 0;
        let mut test_at: Option<i64> = None;
        let mut pending_test = false;
        for (index, line) in lines.iter().enumerate() {
            let trimmed = line.trim();
            if pending_test {
                pending_test = false;
                if trimmed.contains('{') {
                    test_at = Some(depth);
                }
            }
            if test_at.is_none() && !trimmed.starts_with("//") {
                // A function declaration names the construct; its body is
                // where the check belongs.
                let declaration = trimmed.contains("fn ");
                for (token, name) in DIALS {
                    if line.contains(token) && !declaration && !justified(&lines, index, false) {
                        faults.push(format!(
                            "{}:{}: {name} without a policy check",
                            path.strip_prefix(root).unwrap_or(&path).display(),
                            index + 1
                        ));
                    }
                }
                for (token, name) in LOCALS {
                    if line.contains(token) && !declaration && !justified(&lines, index, true) {
                        faults.push(format!(
                            "{}:{}: {name} without a policy check or `egress-exempt: REASON`",
                            path.strip_prefix(root).unwrap_or(&path).display(),
                            index + 1
                        ));
                    }
                }
            }
            if trimmed == "#[cfg(test)]" {
                pending_test = true;
            }
            depth += line.matches('{').count() as i64;
            depth -= line.matches('}').count() as i64;
            if test_at.is_some_and(|at| depth <= at) {
                test_at = None;
            }
        }
    }
    faults
}

#[test]
fn no_source_path_dials_without_a_policy_check() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    source_files(&root, &mut files);
    assert!(files.len() > 100, "{} files", files.len());
    let faults = scan(&root);
    assert!(faults.is_empty(), "{}", faults.join("\n"));
}

#[test]
fn the_scan_refuses_a_self_written_exemption_on_a_dial() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("exempt.rs"),
        "// egress-exempt: the scan must not accept this\nTcpStream::connect(addr).await?;\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("plain.rs"),
        "TcpStream::connect(addr).await?;\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("checked.rs"),
        "policy::permit(host, port)?;\nTcpStream::connect(addr).await?;\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("dial.rs"),
        "egress::dial_addrs(&addrs, deadline).await?;\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("socket.rs"),
        "let socket = TcpSocket::new_v4()?;\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("local.rs"),
        "// egress-exempt: a listener that answers its clients\nUdpSocket::bind(addr)?;\nsend_to(&packet, from)?;\n",
    )
    .unwrap();
    let faults = scan(dir.path());
    let text = faults.join("\n");
    assert!(text.contains("exempt.rs:2"), "{text}");
    assert!(text.contains("plain.rs:1"), "{text}");
    assert!(text.contains("dial.rs:1"), "{text}");
    assert!(text.contains("socket.rs:1"), "{text}");
    assert!(!text.contains("checked.rs"), "{text}");
    assert!(!text.contains("local.rs"), "{text}");
    assert_eq!(faults.len(), 4, "{text}");
}
