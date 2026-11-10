//! `--detach` (issues 70, 71, 72): an instance in the background, with no
//! network. `podnet net switch` is ready when its listener is bound.

#[path = "../test_host/mod.rs"]
mod test_host;

use std::io::Read as _;
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

struct Env {
    _tmp: tempfile::TempDir,
    run: PathBuf,
    sock: PathBuf,
}

impl Env {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let run = tmp.path().join("run");
        let sock = tmp.path().join("sw.sock");
        Env {
            _tmp: tmp,
            run,
            sock,
        }
    }

    fn cmd(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_podnet"));
        cmd.args(args)
            .env("PODNET_RUN_DIR", &self.run)
            .env_remove("RUST_LOG")
            .env_remove("PODNET_ERROR_PROGRAM");
        cmd
    }

    fn run(&self, args: &[&str]) -> Output {
        self.cmd(args).output().unwrap()
    }

    fn switch(&self) -> String {
        format!("unix:{}", self.sock.display())
    }

    fn ps(&self) -> serde_json::Value {
        let out = self.run(&["ps", "--json"]);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap()
    }

    fn pid_of(&self, name: &str) -> Option<i32> {
        self.ps()["instances"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["name"] == name)
            .map(|i| i["pid"].as_i64().unwrap() as i32)
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = self.run(&["stop", "--all", "--timeout", "2"]);
    }
}

/// The last line of stdout: the ready document.
fn ready_doc(out: &Output) -> serde_json::Value {
    let text = String::from_utf8_lossy(&out.stdout);
    let last = text.lines().last().unwrap_or("");
    serde_json::from_str(last).unwrap_or_else(|e| panic!("{e}: {text:?}"))
}

fn alive(pid: i32) -> bool {
    // SAFETY: signal 0 tests for the process only.
    unsafe { libc::kill(pid, 0) == 0 }
}

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(10);
    while !f() {
        assert!(Instant::now() < until, "{what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn connects(sock: &Path) -> bool {
    std::os::unix::net::UnixStream::connect(sock).is_ok()
}

fn instance(env: &Env, name: &str) -> serde_json::Value {
    env.ps()["instances"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["name"] == name)
        .cloned()
        .unwrap_or(serde_json::Value::Null)
}

/// The names of the files in `dir`, sorted.
fn listing(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|r| {
            r.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

mod logs;
mod names;
mod supervisor;
mod warp_exec;
