//! The verification harness at the podnet surface, podnet#501.
//!
//! The fixtures are the three classifiers, the cases of eleven roads by
//! three profiles, one capture for each road and the benign capture. The
//! table has the date of the machine, the profiles, and each named skip. A
//! padded road scores lower than the unpadded one. The benign false-positive
//! rate is measured over many draws and bounded. The archive holds the
//! harness binary, so it opens on a host with no repository and no toolchain
//! and prints the matrix.

use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt as _;
use std::process::Command;

const ROADS: [&str; 10] = [
    "unix", "memory", "tcp", "ws", "h2", "poll", "quic", "udp", "dns", "icmp",
];

fn data() -> String {
    format!("{}/tests/data/harness", env!("CARGO_MANIFEST_DIR"))
}

fn harness(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(["harness", "--data", &data()])
        .args(args)
        .output()
        .expect("podnet harness runs")
}

fn json() -> serde_json::Value {
    let out = harness(&["--json"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

#[test]
fn the_table_has_the_date_the_profiles_and_the_skips() {
    let doc = json();
    let today = pod_harness::date::today();
    assert_eq!(doc["date"], today);
    assert_eq!(doc["conditions"]["runs"], 1);
    assert!(!doc["conditions"]["host"].as_str().unwrap().is_empty());
    assert!(!doc["conditions"]["podnet"].as_str().unwrap().is_empty());
    assert_eq!(doc["conditions"]["podcrates"], "0.79.0");
    let rows = doc["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 33, "{} rows", rows.len());
    let skips: Vec<(&str, &str)> = rows
        .iter()
        .filter(|row| row["state"] == "skipped")
        .map(|row| {
            (
                row["road"].as_str().unwrap(),
                row["reason"].as_str().unwrap(),
            )
        })
        .collect();
    let raw = podnet::harness::raw_socket_available();
    let expected_skips = if raw { 3 } else { 6 };
    assert_eq!(skips.len(), expected_skips, "{skips:?}");
    for (road, reason) in &skips {
        match *road {
            "s3" => {
                assert!(reason.contains("bucket"), "{reason}");
                assert!(reason.contains("credentials"), "{reason}");
            }
            "icmp" => assert!(reason.contains("raw socket"), "{reason}"),
            other => panic!("{other} skipped: {reason}"),
        }
    }
    let profiles: Vec<&str> = rows
        .iter()
        .map(|row| row["profile"].as_str().unwrap())
        .collect();
    for profile in ["direct", "proxy", "restricted"] {
        assert!(profiles.contains(&profile), "{profile}: {profiles:?}");
    }
    let benign = &doc["benign"];
    assert!(benign["samples"].as_u64().unwrap() >= 2, "{benign}");
    assert!(
        benign["rate"].as_f64().unwrap() <= doc["benign_limit"].as_f64().unwrap(),
        "{benign}"
    );
    assert!(doc["elapsed_secs"].as_f64().unwrap() < 600.0);
}

/// Every scored row is a different measurement: no two rows print the same
/// three scores.
#[test]
fn every_scored_row_is_distinguishable() {
    let doc = json();
    let mut seen: BTreeMap<Vec<u64>, String> = BTreeMap::new();
    for row in doc["rows"].as_array().unwrap() {
        let Some(scores) = row["scores"].as_array() else {
            continue;
        };
        let key: Vec<u64> = scores
            .iter()
            .map(|score| score["score"].as_f64().unwrap().to_bits())
            .collect();
        let name = format!(
            "{} / {}",
            row["road"].as_str().unwrap(),
            row["profile"].as_str().unwrap()
        );
        if let Some(other) = seen.insert(key, name.clone()) {
            panic!("{other} and {name} print the same scores");
        }
    }
}

/// The padding of the proxy profile hides a road: its scores are lower than
/// the scores of the same road with no fault.
#[test]
fn the_padded_road_scores_lower_than_the_unpadded_one() {
    let doc = json();
    let mut field: BTreeMap<(String, String), f64> = BTreeMap::new();
    for row in doc["rows"].as_array().unwrap() {
        let Some(scores) = row["scores"].as_array() else {
            continue;
        };
        let Some(field_score) = scores
            .iter()
            .find(|score| score["classifier"] == "field")
            .and_then(|score| score["score"].as_f64())
        else {
            continue;
        };
        field.insert(
            (
                row["road"].as_str().unwrap().to_string(),
                row["profile"].as_str().unwrap().to_string(),
            ),
            field_score,
        );
    }
    let mut compared = 0;
    for ((road, _), direct) in field.iter().filter(|((_, p), _)| p == "direct") {
        let Some(padded) = field.get(&(road.clone(), "proxy".to_string())).copied() else {
            continue;
        };
        assert!(
            *direct >= padded,
            "{road}: unpadded {direct:.3} < padded {padded:.3}"
        );
        if *direct > padded {
            compared += 1;
        }
    }
    assert!(compared > 0, "no road changed under padding");
}

#[test]
fn the_text_names_the_conditions_and_the_benign_rate() {
    let out = harness(&[]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    let today = pod_harness::date::today();
    assert!(text.contains("road"), "{text}");
    assert!(text.contains("profile"), "{text}");
    assert!(text.contains("classifier"), "{text}");
    assert!(text.contains(&today), "{text}");
    assert!(text.contains("benign cover site"), "{text}");
    assert!(text.contains("false-positive rate"), "{text}");
    assert!(text.contains("conditions: host"), "{text}");
    assert!(text.contains("podcrates 0.79.0"), "{text}");
    assert!(text.contains("elapsed"), "{text}");
}

fn expected(into: &std::path::Path, road: &str, profile: &str) -> serde_json::Value {
    let path = into.join(format!("expected/{road}-{profile}.json"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap()
}

#[test]
fn the_archive_runs_the_matrix_with_no_toolchain() {
    let dir = tempfile::tempdir().unwrap();
    let zip = dir.path().join("harness.zip");
    let out = harness(&["--archive", zip.to_str().unwrap()]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(zip.is_file(), "no archive at {}", zip.display());
    let into = dir.path().join("unpacked");
    let status = Command::new("unzip")
        .args(["-q", zip.to_str().unwrap(), "-d", into.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(status.success(), "unzip failed");
    assert!(into.join("run.sh").is_file());
    assert!(into.join("bin/podnet").is_file());
    let mode = std::fs::metadata(into.join("bin/podnet"))
        .unwrap()
        .permissions()
        .mode();
    assert!(mode & 0o111 != 0, "the binary is not executable: {mode:o}");
    assert!(into.join("expected/table.txt").is_file());
    assert!(into.join("simulator/middlebox.rs").is_file());
    // ARCH-3: the fixtures, the three models, the simulator and the
    // expected score of each row, including the skipped rows.
    for name in ["rules.json", "flow.json", "prefix.json", "cases.json"] {
        assert!(into.join(name).is_file(), "{name}");
    }
    for road in ROADS {
        assert!(
            into.join(format!("captures/{road}.cap")).is_file(),
            "{road}"
        );
    }
    assert!(into.join("captures/benign.cap").is_file());
    assert!(into.join("simulator/decisions.json").is_file());
    for profile in ["direct", "proxy", "restricted"] {
        let row = expected(&into, "s3", profile);
        assert_eq!(row["state"], "skipped", "{profile}");
        assert!(
            row["reason"].as_str().unwrap().contains("bucket"),
            "{profile}: {row}"
        );
        let row = expected(&into, "icmp", profile);
        if podnet::harness::raw_socket_available() {
            assert_eq!(row["state"], "scored", "{profile}");
        } else {
            assert_eq!(row["state"], "skipped", "{profile}");
            assert!(
                row["reason"].as_str().unwrap().contains("raw socket"),
                "{profile}: {row}"
            );
        }
    }
    let list = Command::new("sh")
        .args(["run.sh", "--list"])
        .current_dir(&into)
        .output()
        .unwrap();
    assert_eq!(
        list.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&list.stderr)
    );
    let listed = String::from_utf8_lossy(&list.stdout);
    assert!(listed.contains("rules.json"), "{listed}");
    assert!(listed.contains("run.sh"), "{listed}");
    assert!(listed.contains("bin/podnet"), "{listed}");
    // The archive holds the binary, so the run with no argument prints the
    // full table, not a skip.
    let run = Command::new("sh")
        .arg("run.sh")
        .current_dir(&into)
        .output()
        .unwrap();
    assert_eq!(
        run.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let text = String::from_utf8_lossy(&run.stdout);
    let today = pod_harness::date::today();
    assert!(text.contains("road"), "{text}");
    assert!(text.contains("profile"), "{text}");
    assert!(text.contains("classifier"), "{text}");
    assert!(text.contains(&today), "{text}");
    assert!(text.contains("benign cover site"), "{text}");
    assert!(text.contains("false-positive rate"), "{text}");
    for road in ROADS.iter().copied().chain(std::iter::once("s3")) {
        assert!(text.contains(road), "{road}: {text}");
    }
    if !podnet::harness::raw_socket_available() {
        assert!(text.contains("raw socket"), "{text}");
    }
    assert!(text.contains("bucket"), "{text}");
    let table = std::fs::read_to_string(into.join("expected/table.txt")).unwrap();
    assert!(
        text.contains(table.trim_end()),
        "the run does not print the expected table"
    );
    let json = Command::new("sh")
        .args(["run.sh", "--json"])
        .current_dir(&into)
        .output()
        .unwrap();
    assert_eq!(
        json.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&json.stderr)
    );
    let doc: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(doc["date"], today);
    assert_eq!(doc["rows"].as_array().unwrap().len(), 33);
}
