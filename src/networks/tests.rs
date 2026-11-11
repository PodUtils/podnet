use super::*;

fn scheme_of(spec: &str) -> &'static str {
    parse(spec)
        .unwrap_or_else(|e| panic!("{spec}: {e}"))
        .scheme()
}

#[test]
fn network_spec_each_form_of_the_two_old_parsers_is_taken() {
    // The forms of the proxy of the process (egress::Proxy::parse_trusted).
    for spec in [
        "http://proxy.example:3128",
        "http://user:pw@proxy.example",
        "socks5://proxy.example:1080",
        "socks5h://proxy.example",
        "socks5h+unix:/run/proxy.sock",
        "http://unix:/run/proxy.sock",
        "http+exec:ssh -W %h:%p jump",
        "socks5+unix:///run/proxy.sock",
    ] {
        let network = parse(spec).unwrap();
        assert!(
            matches!(network.kind, Kind::Proxy(_)),
            "{spec}: {network:?}"
        );
        assert_eq!(network.scheme(), "proxy");
    }
    // The forms that only the hop of the pod takes (`pod -x`).
    for spec in [
        "unix:/run/proxy.sock",
        "proxy.example:1080",
        "socks://proxy.example:1080",
        "https://proxy.example:3128",
    ] {
        let network = parse(spec).unwrap();
        assert!(
            matches!(network.kind, Kind::Hop { tailscale: false }),
            "{spec}: {network:?}"
        );
        assert_eq!(network.scheme(), "proxy");
    }
    assert_eq!(scheme_of("direct"), "direct");
    assert_eq!(
        scheme_of("tailscale:/run/tailscale/tailscaled.sock"),
        "tailscale"
    );
    assert_eq!(scheme_of("relay"), "relay");
    assert_eq!(scheme_of("relay:relay.example"), "relay");
    assert_eq!(scheme_of("relay:http://127.0.0.1:8787"), "relay");
}

#[test]
fn network_spec_a_bad_spec_is_one_usage_line_with_no_password() {
    for spec in [
        "",
        "   ",
        "tailscale:",
        "unix:",
        "relay:ftp://relay.example",
        "relay:https://relay.example/path",
        "gopher://user:secret@proxy.example:70",
    ] {
        let err = parse(spec)
            .err()
            .unwrap_or_else(|| panic!("{spec:?} parsed"));
        assert_eq!(err.class, Class::Usage, "{spec:?}: {err}");
        assert!(!err.text().contains('\n'), "{err}");
        assert!(!err.text().contains("secret"), "{err}");
    }
    let err = parse("gopher://user:secret@proxy.example:70")
        .err()
        .unwrap();
    assert!(
        err.text().starts_with("gopher://***@proxy.example:70: "),
        "{err}"
    );
}

#[test]
fn network_spec_warp_is_named_and_not_in_the_registry_yet() {
    for spec in ["warp", "warp:identity=/tmp/id.json"] {
        let err = parse(spec).expect_err("no warp yet");
        assert_eq!(err.class, Class::Unsupported);
        assert!(err.text().contains("podnet warp proxy"), "{err}");
    }
    assert!(networks().iter().all(|r| r.scheme != "warp"));
}

#[test]
fn network_spec_the_label_has_no_credentials() {
    assert_eq!(
        parse("socks5h://u:pw@proxy.example:1080").unwrap().label(),
        "socks5h://proxy.example:1080"
    );
    assert_eq!(
        parse("u:pw@proxy.example:1080").unwrap().label(),
        "***@proxy.example:1080"
    );
    assert_eq!(
        parse("https://u:pw@proxy.example:3128").unwrap().label(),
        "https://***@proxy.example:3128"
    );
}

#[test]
fn network_spec_the_table_has_one_row_for_each_scheme() {
    let schemes: Vec<&str> = networks().iter().map(|r| r.scheme).collect();
    assert_eq!(schemes, ["direct", "proxy", "relay", "tailscale"]);
    for spec in ["direct", "http://p.example:3128", "relay", "tailscale:/s"] {
        let network = parse(spec).unwrap();
        assert_eq!(network.row().scheme, network.scheme());
        assert_eq!(network.caps(), network.row().caps);
    }
    assert!(parse("direct").unwrap().caps().contains(Caps::EGRESS_UDP));
    assert!(!parse("relay").unwrap().caps().contains(Caps::EGRESS_UDP));
}

#[test]
fn network_spec_the_checks_name_the_proxy_and_the_relay() {
    let checks = parse("http://u:pw@proxy.example:3128").unwrap().checks();
    assert_eq!(
        checks,
        [Check {
            name: "proxy".into(),
            target: Target::new("proxy.example", 3128),
            tls: false,
        }]
    );
    let checks = parse("relay:relay.example").unwrap().checks();
    assert_eq!(
        checks,
        [Check {
            name: "relay".into(),
            target: Target::new("relay.example", 443),
            tls: true,
        }]
    );
    let checks = parse("relay:http://127.0.0.1:8787").unwrap().checks();
    assert_eq!(checks[0].target, Target::new("127.0.0.1", 8787));
    assert!(!checks[0].tls);
    assert!(parse("socks5h+unix:/run/p.sock")
        .unwrap()
        .checks()
        .is_empty());
    assert!(parse("direct").unwrap().checks().is_empty());
}

#[tokio::test]
async fn network_spec_up_gives_a_way_and_refuses_a_way_below() {
    let direct = parse("direct").unwrap();
    let link = direct.up(&Ctx::default()).await.unwrap();
    assert_eq!(link.way.as_ref().unwrap().label(), "direct");
    assert_eq!(
        link.status,
        serde_json::json!({"scheme": "direct", "spec": "direct"})
    );
    let hop = parse("unix:/run/proxy.sock").unwrap();
    let below = Ctx {
        below: Some(link.way.as_ref().unwrap().dial.clone()),
    };
    let err = hop.up(&below).await.expect_err("no way below yet");
    assert_eq!(
        format!("{err:#}"),
        "unix:/run/proxy.sock cannot go over another way (direct) yet"
    );
    let link = hop.up(&Ctx::default()).await.unwrap();
    assert!(link.way.is_some());
}
