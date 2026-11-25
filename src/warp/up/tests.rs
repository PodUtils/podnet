//! The tests of `podnet warp up` and `down` (`src/warp/up.rs`).

use super::*;

#[test]
fn the_environment_d_file() {
    let text = env_d_text(40001);
    assert!(text.starts_with(ENV_D_MARK));
    assert!(text.contains("\nALL_PROXY=socks5h://127.0.0.1:40001\n"));
    assert!(text.contains("\nHTTPS_PROXY=http://127.0.0.1:40001\n"));
    assert!(!text.contains("export"));
    assert!(text
        .lines()
        .all(|l| l.starts_with('#') || l.split_once('=').is_some_and(|(k, _)| !k.is_empty())));
    assert!(env_d_is_ours(None));
    assert!(env_d_is_ours(Some(&text)));
    assert!(!env_d_is_ours(Some("HTTPS_PROXY=http://corp:3128\n")));
}

#[test]
fn the_env_file_has_the_proxy_variables() {
    let text = env_file_text(Some(40000), Path::new("/run/u/podnet/run/warp.sock"));
    assert!(text.contains("export ALL_PROXY='socks5h://127.0.0.1:40000'\n"));
    assert!(text.contains("export HTTPS_PROXY='http://127.0.0.1:40000'\n"));
    assert!(text.contains("export http_proxy='http://127.0.0.1:40000'\n"));
    assert!(text.contains("export NO_PROXY='localhost,127.0.0.1,::1'\n"));
    assert!(text.contains("export PODNET_WARP_SOCKET='/run/u/podnet/run/warp.sock'\n"));
}

/// Issue 97: with no TCP listener, the env file has no proxy variables
/// (they would name a port that does not listen), only the socket.
#[test]
fn with_no_tcp_listener_the_env_file_has_the_socket_only() {
    let text = env_file_text(None, Path::new("/r/warp.sock"));
    assert!(text.contains("export PODNET_WARP_SOCKET='/r/warp.sock'\n"));
    // Issue 116: podnet gets a proxy URL on the unix socket, and it parses.
    assert!(text.contains("export PODNET_PROXY='socks5h+unix:/r/warp.sock'\n"));
    assert!(crate::egress::Proxy::parse("socks5h+unix:/r/warp.sock").is_ok());
    for var in ["ALL_PROXY", "HTTPS_PROXY", "http_proxy", "CARGO_HTTP_PROXY"] {
        assert!(!text.contains(var), "{var}: {text}");
    }
    assert!(text.contains("podnet warp exec"));
    let doc = serde_json::json!({"listens": ["unix:/r/warp.sock"]});
    assert_eq!(listeners_of(&doc), (None, true));
    let doc = serde_json::json!({"listens": ["tcp://127.0.0.1:40000", "unix:/r/warp.sock"]});
    assert_eq!(listeners_of(&doc), (Some(40000), true));
}

// Issue 115: zsh gets the block too; fish gets a file of its own.
#[test]
fn the_shell_files_follow_the_shell() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    assert_eq!(shell_files_in(home, "/bin/bash").len(), 2);
    assert!(shell_files_in(home, "/usr/bin/zsh").contains(&home.join(".zshrc")));
    std::fs::write(home.join(".zshrc"), "").unwrap();
    assert!(shell_files_in(home, "/bin/bash").contains(&home.join(".zshrc")));
}

// Issue 116: warp up does not send its own session through the proxy
// variables of an earlier warp up.
#[test]
fn the_own_proxy_variables_are_not_given_to_the_proxy() {
    let argv: Vec<String> = [
        "warp",
        "proxy",
        "--listen",
        "tcp://127.0.0.1:40000",
        "--listen",
        "unix:/r/warp.sock",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let vars = [
        ("ALL_PROXY", "socks5h://127.0.0.1:40000"),
        ("HTTPS_PROXY", "http://127.0.0.1:40000"),
        ("PODNET_PROXY", "socks5h+unix:/r/warp.sock"),
        ("PODNET_WARP_SOCKET", "/r/warp.sock"),
        ("http_proxy", "http://corp.example:3128"),
    ]
    .map(|(k, v)| (k.to_string(), v.to_string()));
    let mut own = own_proxy_variables(&argv, vars.into_iter());
    own.sort();
    assert_eq!(own, ["ALL_PROXY", "HTTPS_PROXY", "PODNET_PROXY"]);
}

#[test]
fn the_block_is_added_once_and_removed_with_the_rest_kept() {
    let block = hook_block(
        Path::new("/r/warp.env"),
        Path::new("/r/warp.sock"),
        Path::new("/b/podnet"),
    );
    assert!(block.starts_with(BEGIN) && block.trim_end().ends_with(END));
    assert!(block.contains("exec '/b/podnet' warp shell"));
    assert!(block.contains("PODNET_WARP_SHELL"));
    let user = "alias ll='ls -l'\nexport EDITOR=vi\n";
    let once = format!("{user}{block}");
    let twice = format!("{}{block}", without_block(&once));
    assert_eq!(once, twice, "a second --persist adds nothing");
    assert_eq!(without_block(&once), user, "down keeps the rest");
}

#[test]
fn the_block_is_valid_sh() {
    let block = hook_block(
        Path::new("/nonexistent/warp.env"),
        Path::new("/nonexistent/warp.sock"),
        Path::new("/nonexistent/podnet"),
    );
    let out = std::process::Command::new("sh")
        .arg("-n")
        .arg("-c")
        .arg(&block)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    // An interactive sh runs the block: nothing exists, so no exec.
    let out = std::process::Command::new("sh")
        .arg("-i")
        .arg("-c")
        .arg(format!("{block}\necho still-here"))
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).contains("still-here"));
}

const CERT: &str = "-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----\n";

#[test]
fn a_small_bundle_is_merged_with_the_system_bundle() {
    let dir = tempfile::tempdir().unwrap();
    let proxy = dir.path().join("proxy.pem");
    std::fs::write(&proxy, CERT).unwrap();
    let big = dir.path().join("big.pem");
    std::fs::write(&big, CERT.repeat(40)).unwrap();
    let system = dir.path().join("system.pem");
    std::fs::write(&system, CERT.repeat(120)).unwrap();
    let (p, b) = (proxy.display().to_string(), big.display().to_string());
    let env = |k: &str| match k {
        "SSL_CERT_FILE" => Some(p.clone()),
        "REQUESTS_CA_BUNDLE" => Some(p.clone()),
        "CURL_CA_BUNDLE" => Some(b.clone()),
        _ => None,
    };
    let (merged, vars) = ca_fix(&env, Some(&system)).unwrap();
    assert_eq!(vars, ["SSL_CERT_FILE", "REQUESTS_CA_BUNDLE"]);
    // The proxy root one time, and the 120 roots of the system.
    assert_eq!(certificates_in(&merged), 121);
    // A big bundle (a public bundle) needs no fix.
    let only_big = |k: &str| (k == "SSL_CERT_FILE").then(|| b.clone());
    assert!(ca_fix(&only_big, Some(&system)).is_none());
    // No system bundle: no fix (a warning).
    assert!(ca_fix(&env, None).is_none());
    assert!(ca_fix(&|_| None, Some(&system)).is_none());
}

const TUN_LABEL: &str = "podnet warp tun --default-route --detach";
const NOISE: &str =
    "2026-11-02T10:00:00Z  INFO podnet: the routes and the resolver are as before tun=podnet0\n";

/// Issue 106, case A: the last line names the recovery, and the first
/// line names the real command (`warp tun`, not `warp proxy`).
#[test]
fn a_tun_refusal_names_the_command_and_the_recovery() {
    let stderr =
        format!("{NOISE}podnet: TUNSETIFF (needs CAP_NET_ADMIN): Operation not permitted\n");
    let text = failure_text(TUN_LABEL, 69, &stderr, false);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines[0],
        "podnet warp tun --default-route --detach exited 69: TUNSETIFF (needs CAP_NET_ADMIN): Operation not permitted"
    );
    assert!(!text.contains("warp proxy"), "{text}");
    assert!(text.contains("sudo podnet warp up --tun"), "{text}");
    assert!(text.contains("podnet warp up (proxy mode)"), "{text}");
    assert_eq!(
        *lines.last().unwrap(),
        "podnet: one command with no root: podnet warp run -- CMD"
    );
}

/// As root, sudo gives nothing: it is not advised.
#[test]
fn root_is_not_told_to_use_sudo() {
    let stderr = "podnet: TUNSETIFF (needs CAP_NET_ADMIN): Operation not permitted\n";
    let text = failure_text(TUN_LABEL, 69, stderr, true);
    assert!(!text.contains("sudo"), "{text}");
    assert!(text.contains("CAP_NET_ADMIN"), "{text}");
    assert!(text.contains("podnet warp run -- CMD"), "{text}");
}

/// Issue 106, case B: the route guard. sudo does not help.
#[test]
fn no_default_route_is_not_a_privilege_problem() {
    let stderr = format!("{NOISE}podnet: no IPv4 default route: podnet cannot keep the MASQUE connection outside the tunnel\n");
    let text = failure_text(TUN_LABEL, 69, &stderr, false);
    assert!(text.contains("exited 69: no IPv4 default route"), "{text}");
    assert!(text.contains("needs an IPv4 default route"), "{text}");
    assert!(!text.contains("sudo podnet"), "{text}");
    assert!(text.ends_with("podnet warp run -- CMD"), "{text}");
}

#[test]
fn a_missing_tun_device_is_named() {
    let stderr = "podnet: opening /dev/net/tun: No such file or directory (os error 2)\n";
    let text = failure_text(TUN_LABEL, 69, stderr, false);
    assert!(text.contains("the host has no /dev/net/tun"), "{text}");
    assert!(!text.contains("sudo podnet"), "{text}");
}

/// A failure of the proxy, or a `warp tun` failure that is not a TUN
/// refusal, gets the command and the cause only.
#[test]
fn other_failures_get_no_recovery_lines() {
    let proxy = failure_text(
        "podnet warp proxy --detach",
        69,
        "podnet: the MASQUE endpoint did not answer\n",
        false,
    );
    assert_eq!(
        proxy,
        "podnet warp proxy --detach exited 69: the MASQUE endpoint did not answer"
    );
    let tun = failure_text(
        TUN_LABEL,
        69,
        "podnet: the device is not registered\n",
        false,
    );
    assert_eq!(tun.lines().count(), 1, "{tun}");
    assert_eq!(
        failure_text("podnet warp proxy --detach", 1, "", false),
        "podnet warp proxy --detach exited 1"
    );
}

// T-475: `warp up` reads each system bundle that the manual names, in the
// order of the manual.
#[test]
fn the_system_bundles_are_the_bundles_of_the_manual() {
    let (first, text) = crate::manual::facts::FILES
        .iter()
        .find(|(name, _)| *name == "/etc/ssl/certs/ca-certificates.crt")
        .expect("the manual names the system bundle");
    let mut named = vec![first.to_string()];
    named.extend(
        text.split_whitespace()
            .filter(|w| w.starts_with('/'))
            .map(|w| w.trim_end_matches('.').to_string()),
    );
    assert_eq!(named, SYSTEM_BUNDLES);
}

/// GitHub #738: the error of the child is said once. Its lines wait in the
/// tee, and the error of this process carries the cause and the step of
/// the child; a log line comes at once.
#[test]
fn the_error_of_the_child_is_said_once() {
    const LABEL: &str = "podnet warp proxy --detach";
    let cause = "TLS handshake with the MASQUE front door 162.159.198.2:443: no answer";
    let step = "the MASQUE door failed; try again later";
    let log = "2026-11-02T10:00:00Z  WARN podnet: the front door 162.159.198.1:443 failed";
    let child = [
        log.to_string(),
        "podnet: --detach: the instance warp started".to_string(),
        log.to_string(),
        format!("podnet: {cause}"),
        format!("podnet: {step}"),
    ];
    for success in [false, true] {
        let mut tee = Tee::default();
        let mut shown: Vec<String> = Vec::new();
        for line in &child {
            shown.extend(tee.line(line));
        }
        let stderr = tee.kept.clone();
        shown.extend(tee.end(success));
        if success {
            assert_eq!(shown, child, "a child that worked shows each line");
            continue;
        }
        let text = failure_text(LABEL, 69, &stderr, false);
        assert_eq!(text, format!("{LABEL} exited 69: {cause}\npodnet: {step}"));
        shown.push(format!("podnet: {text}"));
        let all = shown.join("\n");
        assert_eq!(all.matches(cause).count(), 1, "{all}");
        assert_eq!(all.matches(step).count(), 1, "{all}");
        assert_eq!(all.matches(log).count(), 2, "{all}");
        assert!(all.contains("podnet: --detach: the instance"), "{all}");
    }
}
