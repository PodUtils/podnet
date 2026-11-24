//! T-535: a live script that waits for the ready line of podnet prints the
//! masked lines of podnet when podnet ends before it is ready. A stand-in
//! `podnet` writes one error line with a tunnel host and exits 69; each
//! script must print that line with the host masked, name the exit code,
//! and stop at once (not after its wait of 90 s).

mod test_host;

use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use std::time::{Duration, Instant};

const LINE: &str = "podnet: the edge refused the registration: abc-def.trycloudflare.com: Unauthorized (retry: no)";

fn python_has(module: &str) -> bool {
    Command::new("python3")
        .args(["-I", "-c", &format!("import {module}")])
        .output()
        .is_ok_and(|o| o.status.success())
}

#[test]
fn a_live_script_prints_the_lines_of_podnet_when_podnet_ends_before_ready() {
    test_host::need_loopback!();
    let dir = tempfile::tempdir().unwrap();
    let fake = dir.path().join("podnet");
    std::fs::write(&fake, format!("#!/bin/sh\necho '{LINE}' >&2\nexit 69\n")).unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    // (the script, the Python module that it needs before podnet starts,
    // the command of podnet)
    let cases = [
        ("scripts/live-ws-url.sh", Some("websockets"), "tunnel"),
        ("scripts/live-throughput.sh", None, "tunnel"),
        ("scripts/live-share.sh", None, "share"),
    ];
    for (script, module, command) in cases {
        if let Some(module) = module.filter(|m| !python_has(m)) {
            test_host::skip_line(&format!("{script}: python3 has no module {module}"));
            continue;
        }
        let start = Instant::now();
        let out = Command::new("sh")
            .arg(script)
            .arg(&fake)
            .arg("quic")
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .unwrap();
        let took = start.elapsed();
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(1), "{script}: {stdout}{stderr}");
        assert!(
            stdout.contains("<tunnel>.trycloudflare.com: Unauthorized (retry: no)"),
            "{script}: no masked line of podnet: {stdout}{stderr}"
        );
        assert!(
            !stdout.contains("abc-def"),
            "{script}: the host is not masked: {stdout}"
        );
        assert!(
            stdout.contains(&format!(
                "podnet {command} ended before it was ready (exit 69)"
            )),
            "{script}: {stdout}{stderr}"
        );
        assert!(took < Duration::from_secs(30), "{script}: took {took:?}");
    }
}
