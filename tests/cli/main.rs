//! The command line of the podnet binary: exit codes, usage messages and the
//! generated manual. No test here uses the network.

#[path = "../test_host/mod.rs"]
mod test_host;

use std::process::{Command, Output};

fn podnet(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(args)
        .env_remove("PODNET_ERROR_PROGRAM")
        .env_remove("PODNET_EDGE")
        .env("PAGER", "cat")
        .output()
        .expect("podnet runs")
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// Run podnet with the log at its highest level, and give stderr and stdout.
fn trace(args: &[&str], env: &[(&str, &str)]) -> (Option<i32>, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_podnet"));
    cmd.args(args)
        .env_remove("PODNET_ERROR_PROGRAM")
        .env_remove("PODNET_EDGE")
        .env("RUST_LOG", "trace")
        .env("NO_PROXY", "");
    for key in [
        "HTTPS_PROXY",
        "https_proxy",
        "HTTP_PROXY",
        "http_proxy",
        "ALL_PROXY",
        "all_proxy",
    ] {
        cmd.env_remove(key);
    }
    for (key, value) in env {
        cmd.env(key, value);
    }
    let out = cmd.output().unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.code(), text)
}

mod autostart;
mod components;
mod exit_codes;
mod git;
mod link;
mod llms;
mod log;
mod manual;
mod net;
mod profile;
mod query;
mod serve;
mod startup;
mod warp;
