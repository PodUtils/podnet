//! `podnet components` (T-310): the inventory of the carriers and services.

use super::*;

fn json() -> serde_json::Value {
    let out = podnet(&["components", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    serde_json::from_slice(&out.stdout).expect("valid JSON")
}

/// One row for each carrier of `link carriers` and for each service, with
/// the stable fields, at most 64 rows and 4 KiB for each row.
#[test]
fn components_json_has_one_row_for_each_carrier_and_service() {
    let doc = json();
    let rows = doc["components"].as_array().unwrap();
    assert!(!rows.is_empty() && rows.len() <= 64, "{} rows", rows.len());
    let fields = [
        "name",
        "kind",
        "framing",
        "duplex",
        "needs_privilege",
        "available",
        "reason",
        "cost",
        "feature",
        "built",
        "about",
        "address",
    ];
    for row in rows {
        let mut keys: Vec<&str> = row
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        let mut want = fields.to_vec();
        want.sort_unstable();
        assert_eq!(keys, want, "{row}");
        assert!(row.to_string().len() <= 4096, "{row}");
        assert!(["carrier", "service"].contains(&row["kind"].as_str().unwrap()));
    }
    let carriers = podnet(&["link", "carriers", "--json"]);
    let carriers: serde_json::Value = serde_json::from_slice(&carriers.stdout).unwrap();
    for c in carriers["carriers"].as_array().unwrap() {
        let row = rows
            .iter()
            .find(|r| r["kind"] == "carrier" && r["name"] == c["name"])
            .unwrap_or_else(|| panic!("no row for the carrier {}", c["name"]));
        assert_eq!(row["available"], c["available"]);
        assert_eq!(row["built"], c["built"]);
    }
    for service in ["ss", "trojan"] {
        assert!(rows
            .iter()
            .any(|r| r["kind"] == "service" && r["name"] == service));
    }
}

/// A row of a feature that this build leaves out is unavailable, names the
/// feature in its reason, and `excluded` lists it.
#[test]
fn a_row_of_a_feature_that_is_off_says_why() {
    let doc = json();
    let excluded = doc["excluded"].as_array().unwrap();
    for row in doc["components"].as_array().unwrap() {
        if row["built"] == false {
            let feature = row["feature"].as_str().expect("a feature");
            assert_eq!(row["available"], false, "{row}");
            let reason = row["reason"].as_str().unwrap();
            assert!(
                reason.contains("not built") && reason.contains(feature),
                "{row}"
            );
            assert!(row["framing"].is_null() && row["duplex"].is_null(), "{row}");
            assert!(excluded.iter().any(|e| e["name"] == feature), "{feature}");
        }
    }
    assert_eq!(doc["schema"], 1);
}

/// The unix and memory carriers are in each build and are available.
#[test]
fn the_text_lists_each_row() {
    let out = podnet(&["components"]);
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8(out.stdout).unwrap();
    let doc = json();
    for row in doc["components"].as_array().unwrap() {
        let prefix = format!(
            "{}: {}",
            row["name"].as_str().unwrap(),
            row["kind"].as_str().unwrap()
        );
        assert!(
            text.lines().any(|l| l.starts_with(&prefix)),
            "{prefix}\n{text}"
        );
    }
    assert!(text.contains("unix: carrier, stream, duplex"), "{text}");
}
