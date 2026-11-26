//! The tests of `egress.rs`.

use super::*;

/// podnet's words of a refusal: the status and the reason, and a SOCKS5
/// refusal with the status 0.
#[test]
fn a_refusal_has_podnet_s_words() {
    let refused = Refusal::of(403, "Forbidden: not on the egress allowlist");
    assert_eq!(refused.status, 403);
    assert_eq!(
        refused.text,
        "the proxy refused CONNECT with HTTP 403: Forbidden: not on the egress allowlist"
    );
    assert_eq!(
        Refusal::of(502, " ").text,
        "the proxy refused CONNECT with HTTP 502"
    );
    let socks = Refusal::of(407, "SOCKS5: the proxy refused the user and the password");
    assert_eq!(
        (socks.status, socks.text.as_str()),
        (0, "SOCKS5: the proxy refused the user and the password")
    );
}

/// A dial that no address took keeps the kind of its last error and
/// the words of each address, as podnet's dial gave them.
#[test]
fn a_failed_dial_keeps_the_kind_of_its_error() {
    let e = DialError::Connect {
        target: "h:1".into(),
        detail: "192.0.2.1:1: No route to host".into(),
        cut: None,
        kind: Some(std::io::ErrorKind::HostUnreachable),
    };
    let err = failed(e, "h", 1, None);
    assert_eq!(
        format!("{err:#}"),
        "connecting to h:1: 192.0.2.1:1: No route to host"
    );
    let io = err.chain().find_map(|e| e.downcast_ref::<std::io::Error>());
    assert_eq!(
        io.map(|e| e.kind()),
        Some(std::io::ErrorKind::HostUnreachable)
    );
}

/// T-452: a dial that timed out names its step once, not `connecting to
/// h:1: connecting to h:1 timed out ...`.
#[test]
fn a_timed_out_dial_names_its_step_once() {
    let e = DialError::Timeout {
        step: "connecting to h:1".into(),
        after: std::time::Duration::from_secs(29),
        stage: Stage::Network,
    };
    let err = failed(e, "h", 1, None);
    assert_eq!(format!("{err:#}"), "connecting to h:1 timed out after 29s");
}

/// #762: a proxy that answered 502 or 504 reached out, and the target
/// failed: the hop is the target. A refusal of the proxy itself (407,
/// 403) stays at the hop of the proxy. #761: a dial that timed out keeps
/// its DialError as the cause of its failure, with its time limit.
#[test]
fn a_refusal_of_the_far_side_is_the_target_and_a_timeout_keeps_its_limit() {
    let proxy = Proxy::parse("http://p.example:3128").unwrap();
    let cases = [
        (502, Hop::Connect),
        (504, Hop::Connect),
        (407, Hop::Proxy),
        (403, Hop::Proxy),
    ];
    for (status, want) in cases {
        let e = DialError::ProxyRefused {
            proxy: "http://p.example:3128".into(),
            target: "t.example:443".into(),
            status,
            reason: "x".into(),
        };
        let err = failed(e, "t.example", 443, Some(&proxy));
        assert_eq!(hop::of(&err), Some(want), "{status}: {err:#}");
    }
    let e = DialError::Timeout {
        step: "connecting to h:1".into(),
        after: std::time::Duration::from_secs(29),
        stage: Stage::Connect,
    };
    let failure = dial_failure(failed(e, "h", 1, None), Stage::Connect, Class::Timeout);
    let cause = std::error::Error::source(&failure).and_then(|c| c.downcast_ref::<DialError>());
    assert!(
        matches!(cause, Some(DialError::Timeout { after, .. }) if after.as_secs() == 29),
        "{cause:?}"
    );
}

/// podnet's rules: the upper case first, port 8080 for `http://`, the
/// port of a `NO_PROXY` entry, and the loopback through the proxy.
#[test]
fn the_rules_are_podnet_s() {
    let set = |name: &str| match name {
        "HTTPS_PROXY" => Some("proxy.example".into()),
        "https_proxy" => Some("http://lower.example:3128".into()),
        "NO_PROXY" => Some("internal.example:8443".into()),
        _ => None,
    };
    let env = Environment::read_with(&set, rules());
    let proxy = env.proxy_at("relay.example", 443, &[]).unwrap().unwrap();
    assert_eq!((proxy.host.as_str(), proxy.port), ("proxy.example", 80));
    assert!(env
        .proxy_at("internal.example", 8443, &[])
        .unwrap()
        .is_none());
    assert!(env
        .proxy_at("internal.example", 443, &[])
        .unwrap()
        .is_some());
    assert!(
        env.proxy_at("127.0.0.1", 80, &[]).unwrap().is_some(),
        "the loopback takes the proxy"
    );
    assert_eq!(parse("http://127.0.0.1").unwrap().port, 80);
    assert_eq!(parse("socks5://127.0.0.1").unwrap().port, 1080);
}

/// T-349: a proxy on a Unix socket with a user, a password or a host in
/// its path is a usage error (exit 64), and no text of the refusal
/// shows the path. A normal path still works.
#[test]
fn a_unix_proxy_with_user_info_is_refused() {
    for spec in [
        "socks5h+unix://user:hunter2@x",
        "socks5+unix://ss@proxy:2]:/",
        "http+unix:/run/hunter2@x.sock",
        "socks5h://unix://user:hunter2@x",
        "unix://user:hunter2@x",
        " socks5h+unix://hunter2",
    ] {
        let err = parse(spec).unwrap_err();
        assert_eq!(crate::exit::code_of(&err), crate::exit::USAGE, "{spec}");
        let err = set_proxy(spec).unwrap_err();
        assert_eq!(crate::exit::code_of(&err), crate::exit::USAGE, "{spec}");
        let network = crate::networks::parse(spec).unwrap_err();
        for text in [format!("{err:#}"), network.to_string()] {
            for secret in ["hunter2", "ss@", "@x", "proxy:2"] {
                assert!(!text.contains(secret), "{spec}: {text}");
            }
        }
    }
    for spec in [
        "socks5h+unix:/run/x.sock",
        "http://unix:/run/x.sock",
        "unix:/run/x.sock",
        "socks5h+exec:ssh -W %h:%p user@host",
        "http://user:hunter2@proxy:3128",
        "socks5+unix:///run/x.sock",
    ] {
        check_unix_path(spec).unwrap();
    }
    let proxy = parse("socks5h+unix:/run/x.sock").unwrap();
    assert_eq!(proxy.label(), "socks5h+unix:/run/x.sock");
    assert!(crate::networks::parse("socks5h+unix:/run/x.sock").is_ok());
}

/// T-349 with podcrates v0.45.0 (their entry 612): `pod-proxy` itself
/// refuses a Unix path with `//HOST` or `@`, and its text holds no
/// path; `///PATH` stays valid. A tool's file with such a proxy gives no
/// proxy, and the skipped entry holds no password.
#[test]
fn pod_proxy_refuses_a_unix_path_with_user_info() {
    for spec in [
        "socks5h+unix://user:hunter2@x",
        "socks5+unix://ss@proxy:2]:/",
        "http+unix:/run/hunter2@x.sock",
        "socks5h://unix://user:hunter2@x",
    ] {
        let why = Proxy::parse(spec).unwrap_err();
        for secret in ["hunter2", "ss@", "@x", "proxy:2"] {
            assert!(!why.contains(secret), "{spec}: {why}");
        }
    }
    let proxy = Proxy::parse("socks5+unix:///run/x.sock").unwrap();
    assert!(matches!(proxy.carrier(), Carrier::Unix(_)));
    let home = |name: &str| (name == "HOME").then(|| "/home/podnet-test".into());
    let read = |path: &std::path::Path| {
        (path == std::path::Path::new("/home/podnet-test/.gitconfig"))
            .then(|| "[http]\n\tproxy = socks5h+unix://user:hunter2@x\n".to_string())
    };
    let env = Environment::read_with(&home, rules()).or_detected(&home, &read);
    assert!(env.proxy.is_none());
    assert_eq!(env.skipped.len(), 1);
    assert!(
        !env.skipped[0].value.contains("hunter2"),
        "{}",
        env.skipped[0].value
    );
}
