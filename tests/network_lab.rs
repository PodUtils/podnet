//! VM-1 to VM-3: the runtime limits and the network surface of the host.
//!
//! The lab needs `CAP_NET_ADMIN` (two network namespaces, a veth pair and a
//! mount namespace for `/run/netns`). The test runs the lab directly when
//! the process holds the capability, re-executes itself under
//! `unshare --user --map-root-user --net --mount` when it does not, and
//! prints a named skip only when neither route is available, or when the
//! host has no `ip`, `mount` or `ping`. A lab that starts and fails is a
//! failure, never a skip.
//!
//! `docs/target-environment.md` holds the limits; the podnet process is an
//! egress broker at the host, not a guest with its own network stack.

use std::process::{Command, Output};

const NEED: &str = "network-lab";
const INSIDE: &str = "PODNET_NETWORK_LAB_INSIDE";
const TEST: &str = "the_network_surface_needs_the_network_lab";

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// True when the effective capability set holds `CAP_NET_ADMIN` (bit 12).
fn has_cap_net_admin() -> bool {
    let Ok(status) = std::fs::read_to_string("/proc/self/status") else {
        return false;
    };
    let Some(line) = status.lines().find(|line| line.starts_with("CapEff:")) else {
        return false;
    };
    let Some(hex) = line.split_whitespace().nth(1) else {
        return false;
    };
    let Ok(bits) = u64::from_str_radix(hex, 16) else {
        return false;
    };
    bits & (1 << 12) != 0
}

fn run(program: &str, args: &[&str]) -> Result<Output, String> {
    Command::new(program)
        .args(args)
        .output()
        .map_err(|e| format!("{program} {}: {e}", args.join(" ")))
}

fn ok(program: &str, args: &[&str]) -> Result<Output, String> {
    let out = run(program, args)?;
    if !out.status.success() {
        return Err(format!(
            "{program} {}: {:?}: {}",
            args.join(" "),
            out.status.code(),
            text(&out.stderr).trim()
        ));
    }
    Ok(out)
}

/// The lab itself: two namespaces, a veth pair, a ping, and the podnet
/// probes under the installed policy.
fn lab() -> Result<(), String> {
    std::fs::create_dir_all("/run/netns").map_err(|e| format!("mkdir /run/netns: {e}"))?;
    ok("mount", &["-t", "tmpfs", "tmpfs", "/run/netns"])
        .map_err(|e| format!("the /run/netns mount: {e}"))?;
    ok("ip", &["netns", "add", "a"]).map_err(|e| format!("ip netns add a: {e}"))?;
    ok("ip", &["netns", "add", "b"]).map_err(|e| format!("ip netns add b: {e}"))?;
    ok(
        "ip",
        &["link", "add", "va", "type", "veth", "peer", "name", "vb"],
    )
    .map_err(|e| format!("the veth pair: {e}"))?;
    ok("ip", &["link", "set", "va", "netns", "a"]).map_err(|e| format!("va to a: {e}"))?;
    ok("ip", &["link", "set", "vb", "netns", "b"]).map_err(|e| format!("vb to b: {e}"))?;
    for (ns, dev, addr) in [("a", "va", "10.66.0.1/24"), ("b", "vb", "10.66.0.2/24")] {
        ok("ip", &["-n", ns, "addr", "add", addr, "dev", dev])
            .map_err(|e| format!("{addr} in {ns}: {e}"))?;
        ok("ip", &["-n", ns, "link", "set", dev, "up"])
            .map_err(|e| format!("{dev} up in {ns}: {e}"))?;
        ok("ip", &["-n", ns, "link", "set", "lo", "up"])
            .map_err(|e| format!("lo up in {ns}: {e}"))?;
    }
    ok(
        "ip",
        &[
            "netns",
            "exec",
            "a",
            "ping",
            "-c",
            "1",
            "-W",
            "1",
            "10.66.0.2",
        ],
    )
    .map_err(|e| format!("the veth ping: {e}"))?;

    // The host probe reads the mount, in the namespace.
    let out = ok(env!("CARGO_BIN_EXE_podnet"), &["host", "probe", "/"])
        .map_err(|e| format!("podnet host probe: {e}"))?;
    assert_eq!(out.status.code(), Some(0));

    let dir = tempfile::tempdir().map_err(|e| format!("a temporary directory: {e}"))?;
    let rules = dir.path().join("rules.json");
    std::fs::write(
        &rules,
        r#"{"default":"refuse","rules":[{"name":"lab","host":"10.66.0.1","port":"443","action":"direct"}]}"#,
    )
    .map_err(|e| format!("the rules: {e}"))?;
    let out = ok(
        env!("CARGO_BIN_EXE_podnet"),
        &["rules", "check", rules.to_str().unwrap()],
    )
    .map_err(|e| format!("podnet rules check: {e}"))?;
    assert_eq!(out.status.code(), Some(0));
    eprintln!(
        "network lab: namespaces a and b, veth 10.66.0.1/24 <-> 10.66.0.2/24, ping and policy ok"
    );
    Ok(())
}

#[test]
fn the_network_surface_needs_the_network_lab() {
    // T-541: a host without one of the tools of the lab has no lab: a skip
    // with the name of the tool.
    for tool in ["ip", "mount", "ping"] {
        pod_testkit::need_tool!(tool);
    }
    if std::env::var(INSIDE).ok().as_deref() == Some("1") {
        if let Err(why) = lab() {
            panic!("the network lab: {why}");
        }
        return;
    }
    if has_cap_net_admin() {
        if let Err(why) = lab() {
            panic!("the network lab: {why}");
        }
        return;
    }
    // No capability here: run the same test binary as root in a user, net
    // and mount namespace of its own.
    let exe = std::env::current_exe().expect("the path of this test binary");
    let out = Command::new("unshare")
        .args([
            "--user",
            "--map-root-user",
            "--net",
            "--mount",
            "--",
            exe.to_str().expect("a UTF-8 path"),
            "--exact",
            TEST,
            "--nocapture",
        ])
        .env(INSIDE, "1")
        .output()
        .map_err(|e| format!("unshare: {e}"));
    match out {
        Ok(out) if out.status.success() => {
            eprintln!("network lab: ran as root under unshare --user --net --mount");
        }
        Ok(out) => eprintln!(
            "skipping, {NEED}: no CAP_NET_ADMIN, and the unshare route failed: {:?}: {}",
            out.status.code(),
            text(&out.stderr).trim()
        ),
        Err(why) => eprintln!("skipping, {NEED}: no CAP_NET_ADMIN, and unshare: {why}"),
    }
}
