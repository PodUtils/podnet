//! CI-1: a pipeline declares its egress needs, and `podnet ci check`
//! refuses before the first step with the offending host named and exit 69.

use std::path::Path;
use std::process::{Command, Output};

fn run(policy: Option<&Path>, declaration: &Path, args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_podnet"));
    command.args(["ci", "check", declaration.to_str().unwrap()]);
    command.args(args);
    match policy {
        Some(policy) => command.env("PODNET_POLICY", policy),
        None => command.env_remove("PODNET_POLICY"),
    };
    command.env_remove("PODNET_NODE_CONFIG").output().unwrap()
}

fn write(dir: &Path, name: &str, text: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    path
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

const POLICY: &str = r#"{"default":"refuse","rules":[
    {"name":"api","host":"api.example","port":"443","action":"direct"},
    {"name":"proxied","host":"udp.example","port":"443","action":"proxy"}
]}"#;

#[test]
fn a_permitted_declaration_exits_zero() {
    let dir = tempfile::tempdir().unwrap();
    let policy = write(dir.path(), "policy.json", POLICY);
    let declaration = write(
        dir.path(),
        "need.json",
        r#"{"schema":"podnet.ci-egress/1","hosts":[{"host":"api.example","port":443}]}"#,
    );
    let out = run(Some(&policy), &declaration, &[]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let all = format!("{}{}", text(&out.stdout), text(&out.stderr));
    assert!(all.contains("1 need(s) covered"), "{all}");
}

#[test]
fn a_forbidden_host_is_named_and_the_step_never_runs() {
    let dir = tempfile::tempdir().unwrap();
    let policy = write(dir.path(), "policy.json", POLICY);
    let declaration = write(
        dir.path(),
        "need.json",
        r#"{"schema":"podnet.ci-egress/1","hosts":[
            {"host":"api.example","port":443},
            {"host":"blocked.example","port":443}
        ]}"#,
    );
    let out = run(Some(&policy), &declaration, &[]);
    assert_eq!(out.status.code(), Some(69), "{}", text(&out.stderr));
    let all = format!("{}{}", text(&out.stdout), text(&out.stderr));
    assert!(all.contains("blocked.example:443"), "{all}");
    assert!(all.contains("no rule matches"), "{all}");
    assert!(all.contains("before its first step"), "{all}");
}

#[test]
fn a_udp_need_is_not_covered_by_a_proxy_rule() {
    let dir = tempfile::tempdir().unwrap();
    let policy = write(dir.path(), "policy.json", POLICY);
    let declaration = write(
        dir.path(),
        "need.json",
        r#"{"schema":"podnet.ci-egress/1","hosts":[
            {"host":"udp.example","port":443,"udp":true}
        ]}"#,
    );
    let out = run(Some(&policy), &declaration, &[]);
    assert_eq!(out.status.code(), Some(69), "{}", text(&out.stderr));
    let all = format!("{}{}", text(&out.stdout), text(&out.stderr));
    assert!(all.contains("udp.example:443 (UDP)"), "{all}");
    assert!(all.contains("carries TCP"), "{all}");
}

#[test]
fn the_json_document_names_each_refusal() {
    let dir = tempfile::tempdir().unwrap();
    let policy = write(dir.path(), "policy.json", POLICY);
    let declaration = write(
        dir.path(),
        "need.json",
        r#"{"schema":"podnet.ci-egress/1","hosts":[{"host":"blocked.example","port":443}]}"#,
    );
    let out = run(Some(&policy), &declaration, &["--json"]);
    assert_eq!(out.status.code(), Some(69), "{}", text(&out.stderr));
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["schema"], 1);
    assert_eq!(doc["kind"], "podnet.ci-check/1");
    assert_eq!(doc["allowed"], 0);
    assert_eq!(doc["refused"][0]["host"], "blocked.example");
    assert_eq!(doc["refused"][0]["port"], 443);
}

#[test]
fn no_installed_policy_is_a_configuration_fault() {
    let dir = tempfile::tempdir().unwrap();
    let declaration = write(
        dir.path(),
        "need.json",
        r#"{"schema":"podnet.ci-egress/1","hosts":[{"host":"api.example","port":443}]}"#,
    );
    let out = run(None, &declaration, &[]);
    assert_eq!(out.status.code(), Some(78), "{}", text(&out.stderr));
    assert!(
        text(&out.stderr).contains("no egress policy is installed"),
        "{}",
        text(&out.stderr)
    );
}

#[test]
fn a_declaration_that_is_not_json_is_config() {
    let dir = tempfile::tempdir().unwrap();
    let policy = write(dir.path(), "policy.json", POLICY);
    let declaration = write(dir.path(), "need.json", "not json");
    let out = run(Some(&policy), &declaration, &[]);
    assert_eq!(out.status.code(), Some(78), "{}", text(&out.stderr));
    assert!(
        text(&out.stderr).contains("declaration is not JSON"),
        "{}",
        text(&out.stderr)
    );
}
