//! T-330: `podnet pod --report FILE` writes one JSON object when the pod
//! starts and again when it ends. The exit status of `podnet pod` is the
//! exit status of the program, and the report holds it.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output};

use serde_json::Value;

/// The schema file that a release publishes.
const SCHEMA: &str = include_str!("../docs/schema/pod-report.schema.json");

fn pod() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_podnet"));
    command.arg("pod");
    for var in [
        "ALL_PROXY",
        "all_proxy",
        "HTTPS_PROXY",
        "https_proxy",
        "HTTP_PROXY",
        "http_proxy",
        "PODNET_NETNS_QUIET",
        "PODNET_NETNS_TEST_NO_NETNS",
    ] {
        command.env_remove(var);
    }
    command
}

/// The backends that `podnet pod doctor --json` finds, and its `hold`.
fn doctor() -> Value {
    let out = pod().args(["doctor", "--json"]).output().unwrap();
    serde_json::from_slice(&out.stdout).unwrap()
}

fn available(doctor: &Value, backend: &str) -> bool {
    doctor[format!("backend_{backend}")] == Value::Bool(true)
}

/// Whether `value` has the JSON type `name`.
fn has_type(value: &Value, name: &str) -> bool {
    match name {
        "string" => value.is_string(),
        "integer" => value.is_i64() || value.is_u64(),
        "array" => value.is_array(),
        "null" => value.is_null(),
        "object" => value.is_object(),
        other => panic!("the test does not know the type {other}"),
    }
}

/// Check `value` against the keywords of the schema that the file uses:
/// `type`, `const`, `enum`, `items` and `minimum`.
fn check(value: &Value, schema: &Value, place: &str) {
    if let Some(kind) = schema.get("type") {
        let kinds: Vec<&str> = match kind {
            Value::String(s) => vec![s.as_str()],
            Value::Array(a) => a.iter().map(|k| k.as_str().unwrap()).collect(),
            other => panic!("{place}: bad type {other}"),
        };
        assert!(
            kinds.iter().any(|k| has_type(value, k)),
            "{place}: {value} is not of type {kinds:?}"
        );
    }
    if let Some(want) = schema.get("const") {
        assert_eq!(value, want, "{place}: const");
    }
    if let Some(Value::Array(words)) = schema.get("enum") {
        assert!(words.contains(value), "{place}: {value} not in {words:?}");
    }
    if let (Some(items), Some(array)) = (schema.get("items"), value.as_array()) {
        for (n, item) in array.iter().enumerate() {
            check(item, items, &format!("{place}[{n}]"));
        }
    }
    if let (Some(min), Some(n)) = (schema.get("minimum"), value.as_i64()) {
        assert!(n >= min.as_i64().unwrap(), "{place}: {n} below {min}");
    }
}

/// Validate a report against `docs/schema/pod-report.schema.json`.
fn validate(report: &Value) {
    let schema: Value = serde_json::from_str(SCHEMA).unwrap();
    let object = report.as_object().expect("the report is an object");
    for key in schema["required"].as_array().unwrap() {
        assert!(
            object.contains_key(key.as_str().unwrap()),
            "key {key} missing in {report}"
        );
    }
    for (key, rule) in schema["properties"].as_object().unwrap() {
        if let Some(value) = object.get(key) {
            check(value, rule, key);
        }
    }
}

fn read(path: &Path) -> Value {
    let text = std::fs::read_to_string(path).unwrap();
    let report: Value = serde_json::from_str(&text).unwrap();
    validate(&report);
    report
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// No file but the report stays in its directory (no temporary file).
fn only_the_report(dir: &Path) {
    let names: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".tmp"))
        .collect();
    assert!(names.is_empty(), "temporary files: {names:?}");
}

#[test]
fn schema_file_is_json_with_the_keys_of_the_report() {
    let schema: Value = serde_json::from_str(SCHEMA).unwrap();
    let required: Vec<&str> = schema["required"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| k.as_str().unwrap())
        .collect();
    for key in required {
        assert!(schema["properties"][key].is_object(), "no property {key}");
    }
}

/// A program that exits 7 makes `podnet pod` exit 7, and the report holds
/// `"exit": 7`, the backend and the hold of `pod doctor`.
#[test]
fn exit_status_of_the_program_is_in_the_report() {
    let doctor = doctor();
    let mut ran = 0;
    for backend in ["netns", "seccomp"] {
        if !available(&doctor, backend) {
            eprintln!("skipping, the {backend} backend cannot run on this host");
            continue;
        }
        ran += 1;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("report.json");
        let out = pod()
            .args(["--backend", backend, "--report"])
            .arg(&file)
            .args(["-x", "socks5://127.0.0.1:9", "--", "sh", "-c", "exit 7"])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(7), "{}", stderr(&out));
        let report = read(&file);
        assert_eq!(report["exit"], 7);
        assert_eq!(report["schema"], 1);
        assert_eq!(report["backend"], backend);
        assert_eq!(report["hold"], doctor["hold"][backend]["level"]);
        assert_eq!(report["not_held"], doctor["hold"][backend]["not_held"]);
        assert_eq!(report["reason"], Value::Null);
        assert_eq!(report["version"], env!("CARGO_PKG_VERSION"));
        let udp = if backend == "netns" {
            Value::from(1500)
        } else {
            Value::Null
        };
        assert_eq!(report["udp_limit"], udp);
        let mode = std::fs::metadata(&file).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        only_the_report(dir.path());
    }
    if ran == 0 {
        eprintln!("skipping, no backend can run on this host");
    }
}

/// The first report exists while the program runs, with `"exit": null`.
#[test]
fn report_exists_at_the_start_with_a_null_exit() {
    let doctor = doctor();
    let backend = ["netns", "seccomp"]
        .into_iter()
        .find(|b| available(&doctor, b));
    let Some(backend) = backend else {
        eprintln!("skipping, no backend can run on this host");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("report.json");
    let copy = dir.path().join("copy.json");
    let script = format!("cat '{}' > '{}'", file.display(), copy.display());
    let out = pod()
        .args(["--backend", backend, "--report"])
        .arg(&file)
        .args(["-x", "socks5://127.0.0.1:9", "--", "sh", "-c", &script])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let start = read(&copy);
    assert_eq!(start["exit"], Value::Null);
    assert_eq!(start["backend"], backend);
    assert_eq!(read(&file)["exit"], 0);
}

/// A pod that cannot start writes a report with its reason.
#[test]
fn a_pod_that_cannot_start_writes_a_report_with_its_reason() {
    let dir = tempfile::tempdir().unwrap();
    // No proxy: a usage mistake, exit 64.
    let file = dir.path().join("usage.json");
    let out = pod()
        .arg("--report")
        .arg(&file)
        .args(["--", "true"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(64), "{}", stderr(&out));
    let report = read(&file);
    assert_eq!(report["exit"], 64);
    assert_eq!(report["backend"], Value::Null);
    assert_eq!(report["hold"], "none");
    assert!(report["reason"].as_str().unwrap().contains("no proxy"));

    // The pod cannot start the program: exit 3 with a reason.
    if !available(&doctor(), "netns") {
        eprintln!("skipping the exit 3 case, the netns backend cannot run on this host");
        return;
    }
    let file = dir.path().join("start.json");
    let out = pod()
        .args(["--backend", "netns", "--report"])
        .arg(&file)
        .args(["-x", "socks5://127.0.0.1:9", "--", "/nonexistent/program"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(3), "{}", stderr(&out));
    let report = read(&file);
    assert_eq!(report["exit"], 3);
    assert!(report["reason"].as_str().is_some_and(|r| !r.is_empty()));
}

#[test]
fn report_is_refused_with_serve() {
    let dir = tempfile::tempdir().unwrap();
    let out = pod()
        .args(["--serve", "tcp://127.0.0.1:0", "-x", "socks5://127.0.0.1:9"])
        .arg("--report")
        .arg(dir.path().join("r.json"))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(64), "{}", stderr(&out));
}
