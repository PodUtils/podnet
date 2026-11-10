//! Rules: the routes, the ambient proxy, the name map, the doctor and the refused calls.

use super::*;

#[test]
fn seccomp_backend_routes_by_rule() {
    if !have_compiler() || !seccomp_available() {
        eprintln!("skipping: no compiler or no SECCOMP_RET_USER_NOTIF");
        return;
    }
    let dir = std::env::temp_dir().join(format!("podnet-netns-route-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p1 = dir.join("p1.sock");
    let p2 = dir.join("p2.sock");
    let l1 = UnixListener::bind(&p1).unwrap();
    let l2 = UnixListener::bind(&p2).unwrap();
    let (tx1, rx1) = mpsc::channel();
    let (tx2, rx2) = mpsc::channel();
    std::thread::spawn(move || socks5_proxy(l1, tx1, false));
    std::thread::spawn(move || socks5_proxy(l2, tx2, false));

    let program = build_program(&dir, "client-route", CLIENT, false);
    let output = pod_command()
        .args(["--backend", "seccomp", "-x"])
        .arg(format!("unix:{}", p1.display()))
        .args(["-r"])
        .arg(format!("cidr:93.184.0.0/16=unix:{}", p2.display()))
        .arg("--")
        .arg(&program)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("HELLO-FROM-TARGET"));
    // The rule proxy saw it; the default did not.
    assert_eq!(rx2.recv().unwrap(), "93.184.216.34:80");
    assert!(
        rx1.try_recv().is_err(),
        "the default proxy must not be used"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn seccomp_backend_uses_an_ambient_proxy() {
    if !have_compiler() || !seccomp_available() {
        eprintln!("skipping: no compiler or no SECCOMP_RET_USER_NOTIF");
        return;
    }
    let dir = std::env::temp_dir().join(format!("podnet-netns-env-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let sock = dir.join("socks.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    let (tx, _rx) = mpsc::channel();
    std::thread::spawn(move || socks5_proxy(listener, tx, false));

    let program = build_program(&dir, "client-env", CLIENT, false);
    // No -x: the proxy comes from the environment.
    let output = pod_command()
        .args(["--backend", "seccomp", "--"])
        .arg(&program)
        .env("ALL_PROXY", format!("socks5://unix:{}", sock.display()))
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("HELLO-FROM-TARGET"),
        "{:?}",
        String::from_utf8_lossy(&output.stdout)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn seccomp_backend_pins_names_with_map() {
    if !have_compiler() || !seccomp_available() {
        eprintln!("skipping: no compiler or no SECCOMP_RET_USER_NOTIF");
        return;
    }
    let dir = std::env::temp_dir().join(format!("podnet-netns-map-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let sock = dir.join("socks.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || socks5_proxy(listener, tx, false));

    let program = build_program(&dir, "client-map", DNS_CLIENT, false);
    let output = pod_command()
        .args(["--backend", "seccomp", "--map", "example.com=1.2.3.4", "-x"])
        .arg(format!("unix:{}", sock.display()))
        .arg("--")
        .arg(&program)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("DNS:1.2.3.4:18:HELLO-FROM-TARGET"),
        "{stdout:?}"
    );
    // The pinned address reaches the proxy, not a fake IP.
    assert_eq!(rx.recv().unwrap(), "1.2.3.4:80");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn doctor_json_is_machine_readable() {
    let out = pod_command().args(["doctor", "--json"]).output().unwrap();
    let value: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("doctor --json is JSON");
    assert!(value["capabilities"].is_object());
    assert!(value["backend_seccomp"].is_boolean());
    assert!(value["backend_available"].is_boolean());
    // The exit code must agree with the report.
    let any = value["backend_available"].as_bool().unwrap();
    assert_eq!(out.status.success(), any);
}

/// A program named without a `/` is found in PATH, as `execvp` does.
#[test]
fn both_backends_find_a_program_in_path() {
    let dir = std::env::temp_dir().join(format!("podnet-netns-path-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let sock = dir.join("socks.sock");
    let _listener = UnixListener::bind(&sock).unwrap();
    if seccomp_available() {
        run_on("seccomp", &[&sock], std::path::Path::new("true"));
    }
    if netns_available() {
        run_on("netns", &[&sock], std::path::Path::new("true"));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

pub(super) const IO_URING_CLIENT: &str = r#"
#include <stdio.h>
#include <errno.h>
#include <string.h>
#include <unistd.h>
#include <sys/syscall.h>
int main(void) {
    char params[120]; memset(params, 0, sizeof params);
    long r = syscall(SYS_io_uring_setup, 4, params);
    printf("URING:%ld:%s\n", r, r < 0 && errno == EPERM ? "EPERM" : strerror(errno));
    return 0;
}
"#;

/// Issue 98: an errno of the supervisor is a failure of the system call
/// (-1 and errno), not a positive return value. io_uring is refused with
/// EPERM.
#[test]
fn a_refused_system_call_fails_with_its_errno() {
    if !have_compiler() || !seccomp_available() {
        eprintln!("skipping: no compiler or no SECCOMP_RET_USER_NOTIF");
        return;
    }
    let dir = std::env::temp_dir().join(format!("podnet-netns-uring-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let sock = dir.join("socks.sock");
    let _listener = UnixListener::bind(&sock).unwrap();
    let program = build_program(&dir, "uring", IO_URING_CLIENT, false);
    let stdout = run(&dir, &[&sock], &program);
    assert!(stdout.contains("URING:-1:EPERM"), "{stdout:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
