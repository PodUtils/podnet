//! The build features of the link (T-311, GitHub #476, row E10-6). The
//! default build holds the carriers of today. An optional carrier sits
//! behind its feature. A carrier whose feature is off is listed with the
//! reason "not built", and a command that names it exits 69 with the
//! feature name. The tests pass in both builds (default and `--features
//! full`); each one checks the case that the build has. They bind no port.

use std::process::Command;

use podnet::build_features as features;
use podnet::carriers::{self, Options};
use podnet::exit::{self, code_of};
use serde_json::Value;

/// The carriers that sit behind a feature, with the feature of each.
const GATED: [(&str, &str); 15] = [
    ("tcp", "tcp"),
    ("ws", "ws"),
    ("h2", "h2"),
    ("quic", "quic"),
    ("udp", "udp"),
    ("poll", "poll"),
    ("sse", "poll"),
    ("chunked", "poll"),
    ("httppoll", "poll"),
    ("dns", "dns"),
    ("s3", "s3"),
    ("icmp", "icmp"),
    ("ntp", "ntp"),
    ("smtp", "smtp"),
    ("imap", "imap"),
];

fn podnet(args: &[&str]) -> std::process::Output {
    let dir = tempfile::tempdir().unwrap();
    Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(args)
        .env("PODNET_STATE_DIR", dir.path().join("state"))
        .env("PODNET_RUN_DIR", dir.path().join("run"))
        .env_remove("RUST_LOG")
        .output()
        .unwrap()
}

#[test]
fn the_optional_features_are_the_ones_of_the_issue() {
    assert_eq!(
        features::OPTIONAL,
        [
            "tcp", "ws", "h2", "quic", "udp", "poll", "dns", "s3", "icmp", "ntp", "smtp", "imap",
            "ss", "trojan", "sidecar", "c-abi"
        ]
    );
}

#[test]
fn full_holds_each_optional_feature() {
    let manifest =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml")).unwrap();
    let table = manifest
        .split("\n[features]")
        .nth(1)
        .expect("a [features] table");
    let table = table.split("\n[").next().unwrap();
    for name in features::OPTIONAL {
        assert!(
            table
                .lines()
                .any(|l| l.starts_with(&format!("{name:?} = "))
                    || l.starts_with(&format!("{name} = "))),
            "the feature {name} is not declared:\n{table}"
        );
    }
    let full = table
        .lines()
        .find(|l| l.starts_with("full"))
        .expect("the feature full");
    for name in features::FULL {
        assert!(full.contains(&format!("{name:?}")), "{full}");
    }
    assert_eq!(features::FULL, features::OPTIONAL);
}

#[test]
fn the_default_build_holds_no_optional_feature() {
    if cfg!(feature = "full") {
        assert_eq!(features::enabled(), features::FULL);
    } else if !cfg!(any(
        feature = "tcp",
        feature = "ntp",
        feature = "smtp",
        feature = "imap",
        feature = "ss",
        feature = "trojan",
        feature = "sidecar",
        feature = "c-abi"
    )) {
        // Another feature may be on in a build with `--features` (each
        // other optional feature holds `tcp`); the plain default build
        // has none.
        assert!(features::enabled().is_empty(), "{:?}", features::enabled());
    }
}

#[test]
fn a_gated_command_in_a_default_build_exits_69_with_the_feature_name() {
    for name in features::OPTIONAL {
        let result = features::require(name);
        if features::built(name) {
            assert!(result.is_ok(), "{name}");
        } else {
            let e = result.unwrap_err();
            assert_eq!(code_of(&e), exit::UNAVAILABLE, "{name}");
            let text = format!("{e:#}");
            assert!(text.contains(name) && text.contains("not built"), "{text}");
        }
    }
}

#[test]
fn an_unknown_feature_is_a_usage_error() {
    let e = features::require("warp-drive").unwrap_err();
    assert_eq!(code_of(&e), exit::USAGE);
}

#[test]
fn the_registry_has_a_row_for_each_gated_carrier_with_its_feature() {
    for (name, feature) in GATED {
        let row = carriers::ROWS
            .iter()
            .find(|r| r.name == name)
            .unwrap_or_else(|| panic!("no row {name}"));
        assert_eq!(row.feature, Some(feature), "{name}");
    }
    for name in ["unix", "memory"] {
        let row = carriers::ROWS.iter().find(|r| r.name == name).unwrap();
        assert_eq!(row.feature, None, "{name} is in the default build");
    }
}

#[test]
fn a_carrier_whose_feature_is_off_is_listed_as_not_built_and_exits_69_when_named() {
    for (name, feature) in GATED {
        let report = carriers::report(&Options::default());
        let r = report.iter().find(|r| r.name == name).unwrap();
        let opened = carriers::open(name, &Options::default());
        if features::built(feature) {
            // The feature is on: the row is no longer "not built".
            assert!(r.built, "{name}");
            if let Err(reason) = &r.available {
                assert!(!reason.contains("not built"), "{name}: {reason}");
            }
            continue;
        }
        let reason = r.available.as_ref().expect_err("unavailable");
        assert!(reason.contains("not built"), "{reason}");
        assert!(reason.contains(feature), "{reason}");
        assert!(!r.built);
        assert!(r.line().contains("unavailable: "), "{}", r.line());
        let e = opened.err().expect("exit 69");
        assert_eq!(code_of(&e), exit::UNAVAILABLE);
        let text = format!("{e:#}");
        assert!(
            text.contains("not built") && text.contains(feature),
            "{text}"
        );
    }
}

#[test]
fn the_default_carriers_are_built_and_available() {
    let report = carriers::report(&Options::default());
    for name in ["unix", "memory"] {
        let r = report.iter().find(|r| r.name == name).unwrap();
        assert!(r.built && r.available.is_ok(), "{name}");
    }
}

#[test]
fn link_carriers_lists_a_gated_carrier_with_not_built_and_the_json_has_the_feature() {
    let out = podnet(&["link", "carriers"]);
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let out = podnet(&["link", "carriers", "--json"]);
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    let rows = doc["carriers"].as_array().unwrap();
    for (name, feature) in GATED {
        let row = rows.iter().find(|r| r["name"] == name).unwrap();
        assert_eq!(row["feature"], feature);
        assert_eq!(row["built"], features::built(feature), "{row}");
        if !features::built(feature) {
            assert_eq!(row["available"], false);
            assert!(
                row["reason"].as_str().unwrap().contains("not built"),
                "{row}"
            );
            let line = text.lines().find(|l| l.starts_with(name)).unwrap();
            assert!(line.contains("not built"), "{line}");
        }
    }
}

#[test]
fn a_command_that_names_a_carrier_that_is_not_built_exits_69() {
    for (name, feature) in GATED {
        if features::built(feature) {
            continue;
        }
        let out = podnet(&["link", "dial", "x", "--carrier", name, "--peer", "y"]);
        assert_eq!(out.status.code(), Some(69), "{name}");
        let err = String::from_utf8_lossy(&out.stderr).into_owned();
        assert!(err.contains("not built") && err.contains(feature), "{err}");
    }
}

/// The packet sidecar is optional (GitHub #495, LIM-2): a dependency that
/// is optional, and a command that names the feature when it is off.
#[test]
fn the_packet_sidecar_is_optional_and_its_command_names_the_feature() {
    let manifest =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml")).unwrap();
    let line = manifest
        .lines()
        .find(|line| line.starts_with("pod-sidecar ="))
        .expect("a pod-sidecar dependency");
    assert!(line.contains("optional = true"), "{line}");
    if cfg!(feature = "sidecar") {
        let out = podnet(&["sidecar", "probe"]);
        assert_eq!(
            out.status.code(),
            Some(0),
            "stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.contains("verdict"), "{text}");
    } else {
        let out = podnet(&["sidecar", "probe"]);
        assert_eq!(out.status.code(), Some(69));
        let err = String::from_utf8_lossy(&out.stderr).into_owned();
        assert!(
            err.contains("not built") && err.contains("sidecar"),
            "{err}"
        );
    }
}

/// `link listen --service ss` (T-294): a build with no feature `ss` exits
/// 69 and names the feature; a build with it reads the key first, and a
/// missing key exits 64 and names its variable. No listener starts.
#[test]
fn the_service_ss_needs_its_feature_and_then_its_key() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("l.sock");
    let allow = dir.path().join("allow");
    std::fs::write(&allow, "").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(["link", "listen", sock.to_str().unwrap(), "--allow"])
        .arg(&allow)
        .args(["--service", "ss"])
        .env("PODNET_STATE_DIR", dir.path().join("state"))
        .env("PODNET_RUN_DIR", dir.path().join("run"))
        .env_remove("RUST_LOG")
        .env_remove("PODNET_SHADOWSOCKS_SERVICE_KEY")
        .env_remove("PODNET_SHADOWSOCKS_SERVICE_KEY_FILE")
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    let (code, words) = match features::built("ss") {
        true => (exit::USAGE, "PODNET_SHADOWSOCKS_SERVICE_KEY_FILE"),
        false => (exit::UNAVAILABLE, "not built: the feature ss is off"),
    };
    assert_eq!(out.status.code(), Some(code), "{err}");
    assert!(err.contains(words), "{err}");
    assert!(!sock.exists(), "no listener: {err}");
}
