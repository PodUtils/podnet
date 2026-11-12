//! T-060: each `--egress` gives the same parse as its old flag.

use clap::FromArgMatches as _;

use crate::cli::help::top_command;
use crate::cli::way::rewrite;
use crate::cli::Cli;

fn words(a: &[&str]) -> Vec<String> {
    a.iter().map(|s| s.to_string()).collect()
}

fn parse(a: &[String]) -> Cli {
    let matches = top_command().try_get_matches_from(a).expect("a parse");
    Cli::from_arg_matches(&matches).expect("a Cli")
}

/// The network fields of a parse, and of the tunnel options when the
/// command has them.
fn way(cli: &Cli) -> String {
    format!("{:?} {} {:?}", cli.proxy, cli.no_proxy, cli.egress)
}

fn same_as(via: &[&str], old: &[&str]) {
    let rewritten = rewrite(&words(via)).unwrap_or_else(|e| panic!("{via:?}: {e}"));
    assert_eq!(rewritten, words(old), "{via:?}");
    let (a, b) = (parse(&rewritten), parse(&words(old)));
    assert_eq!(way(&a), way(&b));
    assert_eq!(format!("{:?}", a.command), format!("{:?}", b.command));
}

#[test]
fn egress_alias_direct_is_no_proxy() {
    same_as(
        &["podnet", "--egress", "direct", "status"],
        &["podnet", "--no-proxy", "status"],
    );
    same_as(
        &["podnet", "status", "--egress=direct"],
        &["podnet", "status", "--no-proxy"],
    );
}

#[test]
fn egress_alias_a_proxy_url_is_proxy() {
    same_as(
        &["podnet", "--egress", "socks5h://127.0.0.1:1080", "status"],
        &["podnet", "--proxy", "socks5h://127.0.0.1:1080", "status"],
    );
    same_as(
        &["podnet", "status", "--egress", "http+unix:/run/p.sock"],
        &["podnet", "status", "--proxy", "http+unix:/run/p.sock"],
    );
}

#[test]
fn egress_alias_relay_is_transport_relay() {
    let url = "http://localhost:8080";
    same_as(
        &["podnet", "tunnel", "--url", url, "--egress", "relay"],
        &["podnet", "tunnel", "--url", url, "--transport", "relay"],
    );
    same_as(
        &["podnet", "--egress", "relay", "tunnel", "--url", url],
        &["podnet", "tunnel", "--transport", "relay", "--url", url],
    );
    same_as(
        &[
            "podnet",
            "tunnel",
            "--url",
            url,
            "--egress",
            "relay:relay.example",
        ],
        &[
            "podnet",
            "tunnel",
            "--url",
            url,
            "--transport",
            "relay",
            "--relay",
            "https://relay.example",
        ],
    );
}

#[test]
fn egress_alias_pod_takes_netbird_as_a_socks5_hop_on_the_loopback() {
    let rewritten = rewrite(&words(&[
        "podnet",
        "--egress",
        "netbird",
        "--egress",
        "netbird:1081",
        "pod",
        "--",
        "true",
    ]))
    .unwrap();
    assert_eq!(
        rewritten,
        words(&[
            "podnet",
            "pod",
            "-x",
            "127.0.0.1:1080",
            "-x",
            "127.0.0.1:1081",
            "--",
            "true"
        ])
    );
}

/// T-115: the tailnet is a hop of the pod, and a process command names the
/// hop in its refusal.
#[test]
fn network_spec_tailscale_is_a_hop_of_the_pod_and_refused_by_a_process_command() {
    let spec = "tailscale:/run/tailscale/tailscaled.sock";
    let rewritten = rewrite(&words(&["podnet", "--egress", spec, "pod", "--", "true"])).unwrap();
    assert_eq!(
        rewritten,
        words(&["podnet", "pod", "-x", spec, "--", "true"])
    );
    let refused = rewrite(&words(&["podnet", "--egress", spec, "status"])).unwrap_err();
    assert!(refused.contains("a hop of podnet pod"), "{refused}");
    assert!(refused.contains(spec), "{refused}");
}

/// T-117: a WireGuard network is not a hop of the pod and not a flag of a
/// process command. The refusal names the commands that start it.
#[test]
fn network_spec_wg_is_refused_by_the_pod_and_by_a_process_command_with_one_line() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wg0.conf");
    let key = "BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc=";
    std::fs::write(
        &path,
        format!("[Interface]\nPrivateKey = {key}\nAddress = 10.9.0.2/32\n[Peer]\nPublicKey = {key}\nEndpoint = 127.0.0.1:51820\n"),
    )
    .unwrap();
    let spec = format!("wg:{}", path.display());
    for args in [
        words(&["podnet", "--egress", &spec, "pod", "--", "true"]),
        words(&["podnet", "--egress", &spec, "status"]),
    ] {
        let refused = rewrite(&args).unwrap_err();
        assert!(refused.contains("forward --network"), "{refused}");
        assert!(refused.contains("podnet route"), "{refused}");
        assert!(!refused.contains('\n'), "{refused}");
    }
}

#[test]
fn egress_alias_pod_takes_each_via_as_an_x_hop_in_order() {
    let rewritten = rewrite(&words(&[
        "podnet",
        "--egress",
        "socks5://127.0.0.1:1080",
        "--egress=unix:/run/p.sock",
        "pod",
        "--",
        "curl",
        "--egress",
        "x",
    ]))
    .unwrap();
    assert_eq!(
        rewritten,
        words(&[
            "podnet",
            "pod",
            "-x",
            "socks5://127.0.0.1:1080",
            "-x",
            "unix:/run/p.sock",
            "--",
            "curl",
            "--egress",
            "x",
        ])
    );
    // A --egress after the word pod is the pod's own option.
    let own = words(&["podnet", "pod", "--egress", "direct", "--", "true"]);
    assert_eq!(rewrite(&own).unwrap(), own);
    // The program's own --egress after -- stays.
    let tail = words(&["podnet", "status", "--", "--egress", "x"]);
    assert_eq!(rewrite(&tail).unwrap(), tail);
}

#[test]
fn egress_alias_a_spec_that_the_command_cannot_take_is_one_usage_line() {
    let err = |a: &[&str]| rewrite(&words(a)).expect_err("refused");
    assert!(
        err(&["podnet", "--egress", "direct", "--egress", "direct", "status"]).contains("a chain")
    );
    assert_eq!(
        err(&[
            "podnet",
            "--egress",
            "direct",
            "--proxy",
            "http://p:1",
            "status"
        ]),
        "--egress and --proxy name one way: give one of them"
    );
    assert!(
        err(&["podnet", "--egress", "unix:/run/p.sock", "status"]).contains("a hop of podnet pod")
    );
    assert!(err(&["podnet", "--egress", "relay", "status"]).contains("status has no relay way"));
    assert!(err(&["podnet", "--egress", "relay", "pod", "--", "true"])
        .contains("not a hop of podnet pod"));
    assert!(err(&[
        "podnet",
        "--egress",
        "http+exec:nc %h %p",
        "pod",
        "--",
        "true"
    ])
    .contains("podnet pod cannot take it"));
    assert!(err(&["podnet", "--egress", "warp", "status"])
        .contains("not a network of the registry yet"));
    assert!(err(&["podnet", "status", "--egress"]).contains("--egress needs a value"));
    let secret = err(&["podnet", "--egress", "gopher://u:secret@p:70", "status"]);
    assert!(!secret.contains("secret"), "{secret}");
}

#[test]
fn egress_alias_the_old_via_of_serve_names_ingress() {
    let err = top_command()
        .try_get_matches_from(["podnet", "serve", "/tmp", "--via", "none"])
        .expect_err("--via is gone");
    let hints = crate::cli::help::usage_hints(&err);
    assert_eq!(hints.len(), 1, "{hints:?}");
    assert!(
        hints[0].contains("--ingress tunnel|relay|none"),
        "{hints:?}"
    );
}
