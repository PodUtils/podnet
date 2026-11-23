//! ARCH-1 and ARCH-2 of podnet#501: the matrix has a row for each carrier
//! and each of the three profiles, and a two-process test crosses a unix
//! socket with no privilege and no network namespace.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

const PROFILES: [&str; 3] = ["direct", "proxy", "restricted"];

fn cases() -> serde_json::Value {
    let path = format!(
        "{}/tests/data/harness/cases.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    serde_json::from_str(&text).unwrap()
}

/// ARCH-1: each carrier of the permitted-service and datagram set, plus the
/// stream carriers, has one row for each of the three profiles.
#[test]
fn the_matrix_has_a_row_for_each_carrier_and_profile() {
    let document = cases();
    let mut carriers = BTreeSet::new();
    let mut rows = 0;
    let mut skips = Vec::new();
    let mut seen = BTreeSet::new();
    // Each road reads its own capture. One capture for two roads is not two
    // measurements.
    let mut captures: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for case in document["cases"].as_array().unwrap() {
        let road = case["road"].as_str().unwrap();
        let profile = case["profile"].as_str().unwrap();
        carriers.insert(road.to_string());
        rows += 1;
        assert!(
            seen.insert((road.to_string(), profile.to_string())),
            "{road} {profile} is twice"
        );
        let capture = case["capture"].as_str();
        let skip = case["skip"].as_str();
        assert!(
            capture.is_some() != skip.is_some(),
            "{road} {profile} needs exactly one of capture and skip"
        );
        if let Some(capture) = capture {
            let path = format!(
                "{}/tests/data/harness/{capture}",
                env!("CARGO_MANIFEST_DIR")
            );
            let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
            assert!(text.starts_with("pod-capture/1"), "{path}");
            captures
                .entry(road.to_string())
                .or_default()
                .insert(capture.to_string());
        }
        if let Some(reason) = skip {
            skips.push((road.to_string(), reason.to_string()));
        }
    }
    for (road, set) in &captures {
        assert_eq!(set.len(), 1, "{road} reads {set:?}");
    }
    let all: BTreeSet<&String> = captures.values().flatten().collect();
    assert_eq!(
        all.len(),
        captures.len(),
        "two roads read one capture: {captures:?}"
    );
    assert_eq!(
        carriers,
        ["dns", "h2", "icmp", "memory", "poll", "quic", "s3", "tcp", "udp", "unix", "ws"]
            .iter()
            .map(|name| name.to_string())
            .collect::<BTreeSet<_>>()
    );
    for road in &carriers {
        for profile in PROFILES {
            assert!(
                seen.contains(&(road.clone(), profile.to_string())),
                "{road} has no {profile} row"
            );
        }
    }
    assert_eq!(rows, carriers.len() * PROFILES.len());
    // The s3 rows are a named skip in the matrix; the icmp rows carry a
    // capture and the harness skips them only on a host without a raw socket.
    assert_eq!(skips.len(), 3, "{skips:?}");
    for (road, reason) in &skips {
        assert_eq!(road, "s3", "{road}: {reason}");
        assert!(reason.contains("bucket"), "{reason}");
        assert!(reason.contains("credentials"), "{reason}");
    }
}

struct Proc(Child);

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn path_of(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn podnet(state: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(args)
        .env("PODNET_STATE_DIR", state)
        .env_remove("PODNET_LINK_CONTROL")
        .env_remove("PODNET_POLICY")
        .env_remove("PODNET_NODE_CONFIG")
        .output()
        .expect("podnet runs")
}

fn fingerprint(state: &Path) -> String {
    let out = podnet(state, &["link", "status", "--create", "--json"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    doc["identity"]["fingerprint"].as_str().unwrap().to_string()
}

/// ARCH-2: two processes cross a unix socket. The listener is one process;
/// the dialer is another. No TCP port and no network namespace is used.
#[test]
fn two_processes_cross_a_unix_socket_with_no_privilege() {
    let dir = tempfile::tempdir().unwrap();
    let server: PathBuf = dir.path().join("server");
    let client: PathBuf = dir.path().join("client");
    std::fs::create_dir_all(&server).unwrap();
    std::fs::create_dir_all(&client).unwrap();
    let server_fp = fingerprint(&server);
    let client_fp = fingerprint(&client);
    let allow = dir.path().join("allow.txt");
    std::fs::write(&allow, &client_fp).unwrap();
    let socket = dir.path().join("link.sock");
    let sock = path_of(&socket);

    let mut listener = Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(["link", "listen", sock, "--allow", path_of(&allow), "--json"])
        .env("PODNET_STATE_DIR", &server)
        .env_remove("PODNET_LINK_CONTROL")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the listener starts");
    let mut ready = String::new();
    BufReader::new(listener.stdout.as_mut().unwrap())
        .read_line(&mut ready)
        .unwrap();
    let doc: serde_json::Value = serde_json::from_str(&ready)
        .unwrap_or_else(|e| panic!("{e}: the listener printed {ready:?}"));
    assert_eq!(doc["fingerprint"].as_str(), Some(server_fp.as_str()));
    let listener = Proc(listener);

    let out = podnet(
        &client,
        &[
            "link", "dial", sock, "--peer", &server_fp, "--check", "--json",
        ],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(std::path::Path::new(sock).exists(), "the socket file");
    drop(listener);
}
