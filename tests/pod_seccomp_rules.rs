//! The refusals of `podnet pod` (T-156, T-157): a `reject` or `block` rule,
//! `--default-action reject` and the flow record (`--flow-log`).
//!
//! The proxy listens on a unix socket, so these tests need no loopback.

#[path = "pod_harness/mod.rs"]
mod pod_harness;

use pod_harness::*;
use std::os::unix::net::UnixListener;
use std::process::Output;
use std::sync::mpsc;

/// Send one datagram to 93.184.216.34:9999 and print the result of `sendto`.
const DATAGRAM: &str = r#"
#include <stdio.h>
#include <string.h>
#include <errno.h>
#include <unistd.h>
#include <arpa/inet.h>
#include <sys/socket.h>
int main(void) {
    alarm(10);
    int s = socket(AF_INET, SOCK_DGRAM, 0);
    struct sockaddr_in d; memset(&d, 0, sizeof d);
    d.sin_family = AF_INET; d.sin_port = htons(9999);
    inet_pton(AF_INET, "93.184.216.34", &d.sin_addr);
    int r = sendto(s, "PING", 4, 0, (struct sockaddr *)&d, sizeof d);
    printf("SENDTO:%d:%s\n", r, r < 0 ? strerror(errno) : "ok");
    return 0;
}
"#;

/// A proxy that greets and echoes; each target goes to the receiver.
struct Proxy {
    dir: std::path::PathBuf,
    sock: std::path::PathBuf,
    targets: mpsc::Receiver<String>,
}

impl Proxy {
    fn start(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("podnet-rules-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("socks.sock");
        let listener = UnixListener::bind(&sock).unwrap();
        let (tx, targets) = mpsc::channel();
        std::thread::spawn(move || socks5_proxy(listener, tx, false));
        Proxy { dir, sock, targets }
    }

    /// Run `source` (C) in a pod of `backend` with `options`.
    fn run(&self, backend: &str, source: &str, options: &[&str]) -> Output {
        let program = build_program(&self.dir, "program", source, false);
        pod_command()
            .args(["--backend", backend, "-x"])
            .arg(format!("unix:{}", self.sock.display()))
            .args(options)
            .arg("--")
            .arg(&program)
            .output()
            .unwrap()
    }
}

impl Drop for Proxy {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn seccomp_ready() -> bool {
    if !have_compiler() || !seccomp_available() {
        eprintln!("skipping: no compiler or no SECCOMP_RET_USER_NOTIF");
        return false;
    }
    true
}

#[test]
fn seccomp_block_rule_refuses_connect() {
    if !seccomp_ready() {
        return;
    }
    let proxy = Proxy::start("block");
    let output = proxy.run("seccomp", CLIENT, &["-v", "-r", "cidr:93.184.0.0/16=block"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("connect: Connection refused"), "{stderr}");
    assert!(
        stderr.contains("refused by the rule cidr:93.184.0.0/16=block"),
        "-v names the rule: {stderr}"
    );
    assert!(proxy.targets.try_recv().is_err(), "the proxy is not used");
}

#[test]
fn seccomp_reject_rule_refuses_a_datagram() {
    if !seccomp_ready() {
        return;
    }
    let proxy = Proxy::start("reject-udp");
    let output = proxy.run("seccomp", DATAGRAM, &["-r", "cidr:93.184.0.0/16=reject"]);
    assert!(output.status.success());
    assert!(
        stdout(&output).contains("SENDTO:-1:Permission denied"),
        "{}",
        stdout(&output)
    );
}

#[test]
fn netns_block_rule_refuses_connect() {
    if !have_compiler() || !netns_available() {
        eprintln!("skipping: no compiler or no netns backend");
        return;
    }
    let proxy = Proxy::start("netns-block");
    let output = proxy.run("netns", CLIENT, &["-r", "cidr:93.184.0.0/16=block"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("connect: Connection refused"), "{stderr}");
    assert!(proxy.targets.try_recv().is_err(), "the proxy is not used");
}

#[test]
fn a_rule_with_an_unknown_action_is_a_usage_error() {
    let output = pod_command()
        .args(["-x", "socks5://127.0.0.1:1", "-r", "cidr:10.0.0.0/8=drop"])
        .args(["--", "true"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(64));
}
