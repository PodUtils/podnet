use std::path::Path;

use super::*;

fn pod<'a>(backend: &'a str, rules: &'a [String]) -> Pod<'a> {
    Pod {
        socket: Path::new("/r/warp.sock"),
        backend,
        rules,
        session: None,
    }
}

#[test]
fn pod_arguments() {
    let rules = ["cidr:10.0.0.0/8=direct".to_string()];
    let argv = pod_args(&pod("auto", &rules), false, &["git".into(), "fetch".into()]);
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
    let argv = pod_args(&pod("auto", &[]), false, &["true".into()]);
    assert!(!argv.iter().any(|a| a == "--prefer-udp"), "{argv:?}");
}

/// T-110: a named pod gives the directory of its session to the pod.
#[test]
fn a_named_pod_gives_its_session() {
    let mut named = pod("auto", &[]);
    named.session = Some(Path::new("/run/p.pod"));
    let argv = pod_args(&named, false, &["true".into()]);
    let end = argv.iter().position(|a| a == "--").unwrap();
    assert!(
        argv[..end]
            .windows(2)
            .any(|w| w == ["--session", "/run/p.pod"]),
        "{argv:?}"
    );
}

// Issue 116: the program of warp exec gets PODNET_WARP_SOCKET.
#[test]
fn warp_exec_gives_the_socket() {
    let command = pod_command("/b/podnet".into(), &pod("auto", &[]), &["sh".to_string()]);
    assert!(command
        .get_envs()
        .any(|(k, v)| k == "PODNET_WARP_SOCKET" && v == Some("/r/warp.sock".as_ref())));
}

/// T-071: the backend and `-v` of `warp exec` go to the pod.
#[test]
fn warp_exec_passes_the_backend_to_the_pod() {
    let argv = pod_args(&pod("netns", &[]), true, &["true".into()]);
    let end = argv.iter().position(|a| a == "--").unwrap();
    let options = &argv[..end];
    assert!(
        options.windows(2).any(|w| w == ["--backend", "netns"]),
        "{argv:?}"
    );
    assert!(options.iter().any(|a| a == "-v"), "{argv:?}");
    assert!(options.iter().any(|a| a == "--name-backend"), "{argv:?}");
    let quiet = pod_args(&pod("seccomp", &[]), false, &["true".into()]);
    assert!(!quiet.iter().any(|a| a == "-v"), "{quiet:?}");
    assert!(quiet.windows(2).any(|w| w == ["--backend", "seccomp"]));
}
