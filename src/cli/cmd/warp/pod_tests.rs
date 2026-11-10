use std::path::Path;

use super::*;

#[test]
fn pod_arguments() {
    let argv = pod_args(
        Path::new("/r/warp.sock"),
        "auto",
        false,
        &["cidr:10.0.0.0/8=direct".into()],
        &["git".into(), "fetch".into()],
    );
    assert_eq!(
        argv,
        [
            "pod",
            "--backend",
            "auto",
            "--name-backend",
            "-x",
            "socks5://unix:/r/warp.sock",
            "-r",
            "cidr:10.0.0.0/8=direct",
            "--",
            "git",
            "fetch"
        ]
    );
}

/// T-113: the pod of warp exec gets no --prefer-udp, so auto takes the
/// netns backend first.
#[test]
fn warp_exec_auto_has_no_prefer_udp() {
    let argv = pod_args(Path::new("/s"), "auto", false, &[], &["true".into()]);
    assert!(!argv.iter().any(|a| a == "--prefer-udp"), "{argv:?}");
}

// Issue 116: the program of warp exec gets PODNET_WARP_SOCKET.
#[test]
fn warp_exec_gives_the_socket() {
    let command = pod_command(
        "/b/podnet".into(),
        Path::new("/r/warp.sock"),
        "auto",
        &[],
        &["sh".to_string()],
    );
    assert!(command
        .get_envs()
        .any(|(k, v)| k == "PODNET_WARP_SOCKET" && v == Some("/r/warp.sock".as_ref())));
}

/// T-071: the backend and `-v` of `warp exec` go to the pod.
#[test]
fn warp_exec_passes_the_backend_to_the_pod() {
    let argv = pod_args(Path::new("/s"), "netns", true, &[], &["true".into()]);
    let end = argv.iter().position(|a| a == "--").unwrap();
    let options = &argv[..end];
    assert!(
        options.windows(2).any(|w| w == ["--backend", "netns"]),
        "{argv:?}"
    );
    assert!(options.iter().any(|a| a == "-v"), "{argv:?}");
    assert!(options.iter().any(|a| a == "--name-backend"), "{argv:?}");
    let quiet = pod_args(Path::new("/s"), "seccomp", false, &[], &["true".into()]);
    assert!(!quiet.iter().any(|a| a == "-v"), "{quiet:?}");
    assert!(quiet.windows(2).any(|w| w == ["--backend", "seccomp"]));
}
