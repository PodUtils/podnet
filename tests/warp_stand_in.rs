//! T-012: the account commands of `podnet warp` against the API stand-in of
//! `pod-testkit` (`pod_testkit::stand::warp::Api`) on the loopback. The
//! binary reads the base of the API from `PODNET_WARP_API`. No test
//! registers a device at the WARP service.

mod test_host;

use std::path::Path;
use std::process::{Command, Output};

use pod_testkit::stand;

/// The stand-in API, served over plain HTTP on the loopback.
struct World {
    api: stand::warp::Api,
    served: stand::Stand,
    dir: tempfile::TempDir,
}

impl World {
    fn new() -> World {
        let api = stand::warp::Api::new("", "192.0.2.1", "2001:db8::1");
        let served = stand::http(api.handler()).unwrap();
        World {
            api,
            served,
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn identity(&self) -> std::path::PathBuf {
        self.dir.path().join("warp.json")
    }

    /// `podnet warp --identity FILE ARGS` with the API of the stand-in.
    fn warp(&self, args: &[&str]) -> Output {
        self.warp_with(args, None)
    }

    fn warp_with(&self, args: &[&str], stdin: Option<&str>) -> Output {
        use std::io::Write as _;
        let identity = self.identity();
        let mut command = Command::new(env!("CARGO_BIN_EXE_podnet"));
        command
            .args([
                "warp",
                "--no-proxy",
                "--identity",
                identity.to_str().unwrap(),
            ])
            .args(args)
            .env("PODNET_WARP_API", format!("http://{}", self.served.addr()))
            .env("PODNET_STATE_DIR", self.dir.path().join("state"))
            .env_remove("PODNET_ERROR_PROGRAM")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let mut child = command.spawn().unwrap();
        if let Some(text) = stdin {
            child
                .stdin
                .take()
                .unwrap()
                .write_all(text.as_bytes())
                .unwrap();
        } else {
            drop(child.stdin.take());
        }
        child.wait_with_output().unwrap()
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn stored(path: &Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// The text of the stored file, with each string value.
fn file_text(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap()
}

fn ok(out: &Output, what: &str) {
    assert_eq!(
        out.status.code(),
        Some(0),
        "{what}: {}{}",
        text(&out.stdout),
        text(&out.stderr)
    );
}

#[test]
fn warp_stand_in_register_saves_one_device_and_a_second_run_loads_it() {
    test_host::need_loopback!();
    let world = World::new();
    let out = world.warp(&["register"]);
    ok(&out, "register");
    assert_eq!(world.api.devices(), 1);
    assert_eq!(world.api.asked("POST"), 1);
    let path = world.identity();
    assert!(path.exists());
    assert!(file_text(&path).contains("stand-in-device-1"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "the identity file holds a key");
    }
    // The second run loads the device: no new POST.
    let out = world.warp(&["register"]);
    ok(&out, "register again");
    assert_eq!(world.api.asked("POST"), 1);
    assert_eq!(world.api.devices(), 1);
}

#[test]
fn warp_stand_in_status_reads_the_account_of_the_saved_device() {
    test_host::need_loopback!();
    let world = World::new();
    ok(&world.warp(&["register"]), "register");
    let out = world.warp(&["status", "--json"]);
    ok(&out, "status");
    let stdout = text(&out.stdout);
    let doc: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("status --json: {e}: {stdout}"));
    assert!(
        doc.to_string().contains("free"),
        "the account type of the stand-in: {doc}"
    );
    assert_eq!(world.api.asked("POST"), 1, "status registers no device");
}

#[test]
fn warp_stand_in_license_from_stdin_reaches_the_api() {
    test_host::need_loopback!();
    let world = World::new();
    ok(&world.warp(&["register"]), "register");
    let key = "abcd1234-efgh5678-ijkl9012";
    let out = world.warp_with(&["license", "-"], Some(key));
    ok(&out, "license");
    let puts = world.api.bodies("PUT");
    assert_eq!(puts.len(), 1, "{puts:?}");
    assert!(puts[0].contains(key), "{puts:?}");
    // A key that the API refuses (400) is an error.
    let out = world.warp_with(&["license", "-"], Some("short"));
    assert_ne!(out.status.code(), Some(0), "{}", text(&out.stderr));
}

#[test]
fn warp_stand_in_rotate_enrols_a_new_key_for_the_same_device() {
    test_host::need_loopback!();
    let world = World::new();
    ok(&world.warp(&["register"]), "register");
    let path = world.identity();
    let before = stored(&path);
    let patches = world.api.asked("PATCH");
    let out = world.warp(&["rotate"]);
    ok(&out, "rotate");
    assert_eq!(world.api.asked("POST"), 1, "rotate registers no device");
    assert_eq!(world.api.devices(), 1);
    assert_eq!(world.api.asked("PATCH"), patches + 1, "one enrolment");
    let after = stored(&path);
    assert_ne!(before, after, "the stored key changes");
    assert!(file_text(&path).contains("stand-in-device-1"));
}

#[test]
fn warp_stand_in_delete_removes_the_device_and_the_file() {
    test_host::need_loopback!();
    let world = World::new();
    ok(&world.warp(&["register"]), "register");
    let out = world.warp(&["delete"]);
    ok(&out, "delete");
    assert_eq!(world.api.asked("DELETE"), 1);
    assert_eq!(world.api.devices(), 0);
    assert!(!world.identity().exists(), "delete removes the file");
}

#[test]
fn warp_stand_in_register_names_the_limit_of_new_devices() {
    test_host::need_loopback!();
    let world = World::new();
    world.api.fail_post(Some(429));
    let out = world.warp(&["register"]);
    assert_ne!(out.status.code(), Some(0));
    let err = text(&out.stderr);
    assert!(err.contains("429"), "{err}");
    assert!(!world.identity().exists(), "no file for a refused device");
}
