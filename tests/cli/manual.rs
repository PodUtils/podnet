//! The manual, the completion scripts and the JSON documents.

use super::*;

/// Each `.B` line of the man page has text, and each long flag of
/// `podnet man --json` is in the page.
#[test]
fn the_man_page_names_each_flag() {
    let roff = podnet(&["man", "--roff"]);
    assert_eq!(roff.status.code(), Some(0));
    let roff = String::from_utf8(roff.stdout).unwrap();
    for line in roff.lines() {
        if let Some(rest) = line.strip_prefix(".B") {
            if rest.is_empty() || rest.starts_with(' ') {
                assert!(!rest.trim().is_empty(), "an empty .B line");
            }
        }
    }

    let json = podnet(&["man", "--json"]);
    assert_eq!(json.status.code(), Some(0));
    let json: serde_json::Value = serde_json::from_slice(&json.stdout).expect("valid JSON");
    let mut flags = 0;
    for command in json["commands"].as_array().unwrap() {
        for arg in command["args"].as_array().unwrap() {
            if arg["positional"] == false {
                let long = format!("\\-\\-{}", arg["id"].as_str().unwrap().replace('-', "\\-"));
                assert!(roff.contains(&long), "the man page has no {long}");
                flags += 1;
            }
        }
    }
    assert!(flags > 50, "only {flags} flags");

    let table = &json["exit_status"];
    for (code, name) in [
        (0, "success"),
        (64, "usage"),
        (69, "unavailable"),
        (78, "config"),
    ] {
        assert!(
            table
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["code"] == code && row["name"] == name),
            "no row {code} ({name})"
        );
    }
}

/// T-321: the help of `forward` has the TURN forms, and the manual has the
/// variables of the account (a spec holds none).
#[test]
fn the_manual_names_the_turn_network_and_its_variables() {
    let help = podnet(&["forward", "--help"]);
    let help = String::from_utf8(help.stdout).unwrap();
    assert!(help.contains("turn://HOST[:PORT]") && help.contains("turns://HOST[:PORT]"));
    assert!(help.contains("PODNET_TURN_PASSWORD_FILE"));
    let out = podnet(&["man", "environment", "--json"]);
    assert_eq!(out.status.code(), Some(0));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).expect("valid JSON");
    let row = json
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["names"].to_string().contains("PODNET_TURN_USER"))
        .expect("a row for PODNET_TURN_USER");
    let names = row["names"].to_string();
    for name in [
        "PODNET_TURN_PASSWORD_FILE",
        "PODNET_TURN_PASSWORD",
        "PODNET_TURN_REALM",
    ] {
        assert!(names.contains(name), "{names}");
    }
}

/// T-320: the help of `forward` has the Dev Tunnels form, and the manual has
/// the variables of the token (a spec holds none).
#[test]
fn the_manual_names_the_devtunnel_network_and_its_variables() {
    let help = podnet(&["forward", "--help"]);
    let help = String::from_utf8(help.stdout).unwrap();
    assert!(help.contains("devtunnel://TUNNEL"));
    assert!(help.contains("PODNET_DEVTUNNEL_TOKEN_FILE"));
    let out = podnet(&["man", "environment", "--json"]);
    assert_eq!(out.status.code(), Some(0));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).expect("valid JSON");
    let row = json
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["names"].to_string().contains("PODNET_DEVTUNNEL_TOKEN_FILE"))
        .expect("a row for PODNET_DEVTUNNEL_TOKEN_FILE");
    let names = row["names"].to_string();
    for name in [
        "PODNET_DEVTUNNEL_TOKEN",
        "PODNET_DEVTUNNEL_SCHEME",
        "PODNET_DEVTUNNEL_MANAGEMENT",
    ] {
        assert!(names.contains(name), "{names}");
    }
}

#[test]
fn ports_json_is_one_object() {
    let out = podnet(&["ports", "--json"]);
    assert_eq!(out.status.code(), Some(0));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).expect("valid JSON");
    assert!(json["ports"].is_array(), "{json}");
}

/// Each default address in the table "What listens" of SECURITY.md is the
/// default of the --listen flag of that command, and each command with a
/// --listen default has a row.
#[test]
fn the_listen_defaults_agree_with_the_manual() {
    let security =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/SECURITY.md")).unwrap();
    let mut table = std::collections::BTreeMap::new();
    for line in security.lines() {
        let cells: Vec<&str> = line.split('|').map(str::trim).collect();
        if cells.len() < 4 {
            continue;
        }
        let (command, address) = (cells[1], cells[2]);
        let (Some(command), Some(address)) = (
            command
                .strip_prefix("`podnet ")
                .and_then(|c| c.strip_suffix('`')),
            address.strip_prefix('`').and_then(|a| a.strip_suffix('`')),
        ) else {
            continue;
        };
        if address.starts_with("tcp://") || address.starts_with("unix:") {
            table.insert(command.to_string(), address.to_string());
        }
    }
    let json = podnet(&["man", "--json"]);
    let json: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    let mut manual = std::collections::BTreeMap::new();
    for command in json["commands"].as_array().unwrap() {
        for arg in command["args"].as_array().unwrap() {
            if arg["id"] == "listen" {
                if let Some(default) = arg["default"].as_str() {
                    let path: Vec<&str> = command["path"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|w| w.as_str().unwrap())
                        .collect();
                    manual.insert(path.join(" "), default.to_string());
                }
            }
        }
    }
    assert!(manual.len() >= 4, "{manual:?}");
    assert_eq!(table, manual, "SECURITY.md and podnet man --json differ");
}

/// Each completion script names the commands and the flags; the bash script
/// is valid bash.
#[test]
fn completion_scripts() {
    for shell in ["bash", "zsh", "fish"] {
        let out = podnet(&["completions", shell]);
        assert_eq!(out.status.code(), Some(0), "{shell}");
        let text = String::from_utf8(out.stdout).unwrap();
        for word in ["tunnel", "warp", "update", "leg-failover"] {
            assert!(text.contains(word), "{shell}: no {word}");
        }
        if shell == "bash" {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("podnet.bash");
            std::fs::write(&path, &text).unwrap();
            let check = Command::new("bash").arg("-n").arg(&path).status().unwrap();
            assert!(check.success(), "bash -n failed");
        }
    }
    assert_eq!(podnet(&["completions", "tcsh"]).status.code(), Some(64));
}

/// E3: the JSON documents have schema and podnet; ps --json names its dir.
#[test]
fn json_documents_have_the_schema_and_the_version() {
    let dir = tempfile::tempdir().unwrap();
    for args in [
        &["ps", "--json"][..],
        &["ports", "--json"][..],
        &["status"][..],
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_podnet"))
            .args(args)
            .env("PODNET_RUN_DIR", dir.path())
            .output()
            .unwrap();
        let doc: serde_json::Value = serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|e| panic!("{args:?}: {e}: {}", stderr(&out)));
        assert_eq!(doc["schema"], 1, "{args:?}: {doc}");
        assert!(
            doc["podnet"]
                .as_str()
                .unwrap()
                .starts_with(env!("CARGO_PKG_VERSION")),
            "{args:?}: {doc}"
        );
        if args[0] == "ps" {
            assert!(doc["dir"].is_string(), "{doc}");
        }
    }
}
