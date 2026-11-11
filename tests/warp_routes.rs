//! `warp tun --default-route` in a network namespace (T-240). It needs
//! CAP_NET_ADMIN and `ip`. Run it on a disposable host:
//!
//! ```sh
//! unshare --user --map-root-user --net --mount \
//!   cargo test -p podnet --test warp_routes -- --ignored routes_lifecycle
//! ```
//!
//! The test skips when the host does not allow it.

use std::process::Command;

fn sh(script: &str) -> Option<String> {
    let out = Command::new("sh").arg("-c").arg(script).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

#[test]
#[ignore = "needs CAP_NET_ADMIN, ip and a mount namespace (unshare --user --map-root-user --net --mount)"]
fn routes_lifecycle() {
    let ready = sh("command -v ip >/dev/null \
        && ip link add t240a type dummy \
        && ip link add t240b type dummy \
        && ip link set t240a up && ip link set t240b up \
        && ip addr add 192.0.2.2/24 dev t240a \
        && ip route add default via 192.0.2.1 dev t240a");
    if ready.is_none() {
        eprintln!("skip: no ip, no CAP_NET_ADMIN or no network namespace");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let original = dir.path().join("original.conf");
    std::fs::write(&original, "nameserver 10.0.0.2\n").unwrap();
    let bound = sh(&format!(
        "touch /etc/resolv.conf && mount --bind {} /etc/resolv.conf",
        original.display()
    ));
    if bound.is_none() {
        eprintln!("skip: no mount namespace");
        return;
    }
    std::env::set_var("PODNET_STATE_DIR", dir.path());
    let probe = || sh("ip -4 route get 203.0.113.5").unwrap();
    let before = probe();
    {
        let mut routes = podnet::warp::route::Routes::setup(
            "t240b",
            vec!["198.51.100.7".parse().unwrap()],
            Vec::new(),
            false,
        )
        .unwrap();
        routes.up().unwrap();
        // The positive control: the snapshot sees the /1 routes.
        assert_ne!(probe(), before);
        let text = std::fs::read_to_string("/etc/resolv.conf").unwrap();
        assert!(
            text.starts_with(podnet::warp::route::resolv::MARKER),
            "{text}"
        );
    }
    assert_eq!(probe(), before);
    assert_eq!(
        std::fs::read_to_string("/etc/resolv.conf").unwrap(),
        "nameserver 10.0.0.2\n"
    );
}
