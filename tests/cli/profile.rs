//! T-218: the connector profile (`docs/protocol.md`, section 10).

use super::*;

/// The text of the profile in `docs/protocol.md`.
fn profile() -> String {
    let doc =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/docs/protocol.md")).unwrap();
    let start = doc
        .find("## 10. The connector profile")
        .expect("section 10");
    let rest = &doc[start..];
    let end = rest.find("\n## Failure causes").expect("section end");
    rest[..end].to_string()
}

/// The long flags of `podnet tunnel --help`, sorted.
fn tunnel_flags() -> Vec<String> {
    let out = podnet(&["tunnel", "--help"]);
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let mut flags: Vec<String> = text
        .lines()
        .filter_map(|line| {
            let line = line.trim_start();
            let at = line.find("--")?;
            // A flag starts a line, or follows `-x, `.
            if at != 0 && !line[..at].ends_with(", ") {
                return None;
            }
            Some(
                line[at..]
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
                    .collect(),
            )
        })
        .collect();
    flags.sort();
    flags.dedup();
    flags
}

fn sorted_keys(doc: &serde_json::Value) -> Vec<String> {
    let mut keys: Vec<String> = doc.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    keys
}

#[test]
fn connector_profile_is_pinned() {
    let profile = profile();

    // The flags of `tunnel`: a flag is added, never removed.
    let flags = tunnel_flags();
    let pinned = [
        "--auth-password-file",
        "--auth-user",
        "--ca-file",
        "--check",
        "--color",
        "--config",
        "--credentials-file",
        "--detach",
        "--egress",
        "--forward",
        "--ha-connections",
        "--hello-world",
        "--help",
        "--http-host-header",
        "--json",
        "--leg-failover",
        "--log-format",
        "--masque-endpoint",
        "--name",
        "--no-proxy",
        "--no-restart",
        "--no-tls-verify",
        "--no-verify",
        "--port",
        "--protocol",
        "--proxy",
        "--qr",
        "--quiet",
        "--ready-timeout",
        "--relay",
        "--replace",
        "--resolve",
        "--run-for",
        "--service-url",
        "--strip-accept-encoding",
        "--tcp",
        "--token",
        "--token-file",
        "--transport",
        "--unix",
        "--url",
        "--user-agent",
        "--verbose",
        "--warp-identity",
    ];
    for flag in pinned {
        assert!(flags.iter().any(|f| f == flag), "tunnel lost {flag}");
        assert!(
            profile.contains(&format!("`{flag}`")),
            "profile lacks {flag}"
        );
    }
    assert_eq!(
        flags.len(),
        pinned.len(),
        "a new flag: add it to the profile: {flags:?}"
    );

    // The exit codes: a code is added, never changed.
    let codes: Vec<(i32, &str)> = podnet::exit::TABLE
        .iter()
        .map(|r| (r.code, r.name))
        .collect();
    assert_eq!(
        codes,
        [
            (0, "success"),
            (1, "failure"),
            (3, "no-backend"),
            (64, "usage"),
            (66, "no-input"),
            (68, "no-host"),
            (69, "unavailable"),
            (71, "os-error"),
            (75, "tempfail"),
            (77, "no-permission"),
            (78, "config"),
            (124, "timeout"),
        ]
    );
    for (code, name) in codes {
        assert!(
            profile.contains(&format!("| {code} | `{name}` |")),
            "profile lacks the code {code}"
        );
    }
    assert_eq!(podnet(&["tunel"]).status.code(), Some(64));
    assert_eq!(
        podnet(&["tunnel", "--protocl", "quic"]).status.code(),
        Some(64)
    );

    // The keys of `ps --json`, with one live instance.
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("run");
    std::fs::create_dir(&dir).unwrap();
    let paths = podnet::instances::paths(&dir, "profile").unwrap();
    let _held = podnet::instances::try_lock(&paths.lock).unwrap().unwrap();
    podnet::instances::write(
        &dir,
        &podnet::instances::Record {
            name: "profile".into(),
            pid: std::process::id(),
            started: 1,
            command: "tunnel".into(),
            state: "ready".into(),
            ready: Some(serde_json::json!({"url": "https://x/"})),
            ..Default::default()
        },
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(["ps", "--json"])
        .env("PODNET_RUN_DIR", &dir)
        .output()
        .unwrap();
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let top = sorted_keys(&doc);
    assert_eq!(top, ["dir", "instances", "podnet", "schema"]);
    let item_keys = sorted_keys(&doc["instances"][0]);
    assert_eq!(
        item_keys,
        [
            "args",
            "command",
            "cwd",
            "log",
            "name",
            "pid",
            "ready",
            "restarts",
            "started",
            "state",
            "supervisor"
        ]
    );
    for key in top.iter().chain(&item_keys) {
        assert!(profile.contains(&format!("`{key}`")), "profile lacks {key}");
    }
}
