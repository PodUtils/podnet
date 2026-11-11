use super::*;

const GOOD: &str = "networks:\n  home: direct\n  other: direct\ngroups:\n  egress: home*2, other*1\nhealth:\n  target: 192.0.2.1:443\n  interval: 5\ndoors:\n  - listen: unix:/tmp/r.sock\n    via: egress\n";

#[test]
fn route_config_reads_networks_groups_doors_and_health() {
    let config = parse(GOOD).unwrap();
    assert_eq!(config.networks.len(), 2);
    assert_eq!(config.groups["egress"], "home*2, other*1");
    assert_eq!(config.doors[0].via, "egress");
    assert_eq!(config.health.as_ref().unwrap().interval, Some(5));
}

/// T-100, the Prove: each error names the key or the line.
#[test]
fn route_config_errors_name_the_key() {
    let cases = [
        ("networks:\n  home: direct\ndoors:\n  - listen: unix:/x\n    via: nowhere\n", "doors[0].via"),
        ("networks:\n  home: direct\ndoors:\n  - listen: unix:/x\n    via: home\n    port: 3\n", "port"),
        ("networks:\n  home: direct\ndoorz: []\n", "doorz"),
        ("networks:\n  home: direct\ndoors: []\n", "doors"),
        ("networks:\n  home: direct\ngroups:\n  home: home*1\ndoors:\n  - listen: unix:/x\n    via: home\n", "groups.home"),
        ("networks:\n  home: direct\nhealth:\n  target: nohost\ndoors:\n  - listen: unix:/x\n    via: home\n", "health.target"),
        ("networks:\n  home: direct\ndoors:\n  - listen: ftp://x\n    via: home\n", "doors[0].listen"),
        ("networks:\n  home: direct\nhealth:\n  target: h:1\n  down_after: 2\n  up_after: 2\ndoors:\n  - listen: unix:/x\n    via: home\n", "health"),
        ("networks:\n  home: direct\nhealth:\n  target: h:1\n  interval: 0\ndoors:\n  - listen: unix:/x\n    via: home\n", "health.interval"),
        ("networks:\n  home: direct\ndoors:\n  - listen: unix:/x\n    via: home\n    dns: 127.0.0.1:53\n", "doors[0].dns"),
        ("networks:\n  home: direct\ndoors:\n  - listen: unix:/x\n    via: home\n    dns: x\n    dns_upstream: 10.0.0.1:53\n", "doors[0].dns"),
        ("networks:\n  home: direct\ndoors:\n  - listen: unix:/x\n    via: home\n    dns_upstream: 10.0.0.1:53\n", "doors[0].dns_upstream"),
        ("networks:\n  home: direct\ncontrol: 127.0.0.1:9\ndoors:\n  - listen: unix:/x\n    via: home\n", "control"),
        ("networks:\n  home: direct\ngroups:\n  g: home\npolicies:\n  g: fastest\ndoors:\n  - listen: unix:/x\n    via: g\n", "policies.g"),
        ("networks:\n  home: direct\ngroups:\n  g: home\npolicies:\n  h: rr\ndoors:\n  - listen: unix:/x\n    via: g\n", "policies.h"),
        ("networks:\n  home: direct\nhealth:\n  target: h:1\n  window: 65\ndoors:\n  - listen: unix:/x\n    via: home\n", "window"),
        ("networks:\n\thome: direct\n", "line 2"),
    ];
    for (text, key) in cases {
        let err = format!("{:#}", parse(text).expect_err(key));
        assert!(err.contains(key), "{key}: {err}");
    }
}

#[tokio::test]
async fn route_router_up_builds_groups_and_doors() {
    let config = parse(GOOD).unwrap();
    let router = Router::up(&config).await.unwrap();
    assert_eq!(router.groups.len(), 1);
    assert_eq!(router.doors.len(), 1);
    let status = router.status();
    assert_eq!(status["groups"][0]["group"], "egress");
    assert_eq!(status["groups"][0]["members"].as_array().unwrap().len(), 2);
    assert_eq!(status["doors"][0]["listen"], "unix:/tmp/r.sock");
    let bad = parse("networks:\n  home: gopher://x\ndoors:\n  - listen: unix:/x\n    via: home\n")
        .unwrap();
    let err = format!("{:#}", Router::up(&bad).await.err().unwrap());
    assert!(err.starts_with("networks.home"), "{err}");
}

/// A SOCKS5 proxy that only records each connection: the test sees which
/// network the router dialed.
async fn recorder() -> (String, tokio::sync::mpsc::UnboundedReceiver<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let _ = tx.send(());
            drop(stream);
        }
    });
    (format!("socks5h://{addr}"), rx)
}

/// T-177, the Prove: for each target, the dry run (`Router::plan`) names
/// the way that the router (`Router::up`) dials, and a refusal of the
/// rules is a refusal of both.
#[tokio::test]
async fn route_check_agrees_with_the_router() {
    crate::test_host::need_loopback!();
    let dir = tempfile::tempdir().unwrap();
    let rules = dir.path().join("r.rules");
    std::fs::write(
        &rules,
        "domain:.a.example=a\ndomain:.blocked.example=reject\ncidr:10.0.0.0/8=a\n",
    )
    .unwrap();
    let (a, mut seen_a) = recorder().await;
    let (b, mut seen_b) = recorder().await;
    let text = format!(
        "networks:\n  a: {a}\n  b: {b}\ndoors:\n  - listen: unix:/tmp/x.sock\n    via: b\n    rules: {}\n",
        rules.display()
    );
    let config = parse(&text).unwrap();
    let plan = Router::plan(&config).unwrap();
    let router = Router::up(&config).await.unwrap();
    let cases = [
        ("www.a.example:443", Some("a"), Some("domain:.a.example")),
        ("10.1.2.3:80", Some("a"), Some("cidr:10.0.0.0/8")),
        ("other.example:443", Some("b"), None),
        (
            "x.blocked.example:443",
            None,
            Some("domain:.blocked.example"),
        ),
    ];
    for (to, way, rule) in cases {
        let target: Target = to.parse().unwrap();
        let decision = plan.decide(0, &target);
        assert_eq!(decision.way.as_deref(), way, "{to}");
        assert_eq!(decision.rule.as_deref(), rule, "{to}");
        assert_eq!(router.decide(0, &target), decision, "{to}");
        let rules = router.doors[0].rules.clone().unwrap();
        let dialed = rules
            .dial(&target, Instant::now() + Duration::from_secs(5))
            .await;
        let got = match (seen_a.try_recv(), seen_b.try_recv()) {
            (Ok(()), Err(_)) => Some("a"),
            (Err(_), Ok(())) => Some("b"),
            (Err(_), Err(_)) => None,
            _ => panic!("{to}: both networks were dialed"),
        };
        assert_eq!(got, way, "{to}: {:?}", dialed.err());
        if way.is_none() {
            let err = dialed.err().unwrap().text().to_string();
            assert!(err.contains("refuses"), "{to}: {err}");
        }
    }
}

/// The dry run of a door over a group: the pick names a member and its
/// health; a group with no member up refuses.
#[test]
fn route_check_picks_a_member_of_a_group() {
    let config = parse(GOOD).unwrap();
    let plan = Router::plan(&config).unwrap();
    let target: Target = "example.com:443".parse().unwrap();
    let decision = plan.decide(0, &target);
    assert_eq!(decision.way.as_deref(), Some("egress"));
    let (member, health) = decision.member.clone().unwrap();
    assert!(member == "home" || member == "other", "{member}");
    assert_eq!(health, "up");
    assert!(decision.passes(&plan.groups));
    let (_, group) = &plan.groups[0];
    for i in 0..group.members().len() {
        group.set_health(i, crate::network::Health::Down);
    }
    let decision = plan.decide(0, &target);
    assert_eq!(decision.member, None);
    assert!(!decision.passes(&plan.groups));
}

/// T-097 item 11: the thresholds of the file go to the probe of the router.
#[test]
fn route_config_thresholds_reach_the_router() {
    let text = GOOD.replace(
        "  interval: 5\n",
        "  interval: 5\n  down_after: 3\n  up_after: 2\n  cooldown: 7\n  timeout: 2\n",
    );
    let plan = Router::plan(&parse(&text).unwrap()).unwrap();
    let (_, t) = plan.health.unwrap();
    assert_eq!((t.down_after, t.up_after), (3, 2));
    assert_eq!(t.interval, Duration::from_secs(5));
    assert_eq!(t.cooldown, Duration::from_secs(7));
    assert_eq!(t.timeout, Duration::from_secs(2));
    let plan = Router::plan(&parse(GOOD).unwrap()).unwrap();
    let (_, t) = plan.health.unwrap();
    assert_eq!(
        t,
        crate::network::Thresholds {
            interval: Duration::from_secs(5),
            ..Default::default()
        }
    );
}

/// T-097 item 4, the Prove `router_status_json_keys`: the control socket
/// gives the status document of the router, with the keys of each member;
/// another request gets 404.
#[tokio::test]
async fn router_status_json_keys_over_the_control_socket() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("r.ctl");
    let router = Arc::new(Router::plan(&parse(GOOD).unwrap()).unwrap());
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    tokio::spawn(router.clone().serve_control(listener));
    let status = read_status(&path).await.unwrap();
    assert_eq!(status["doors"][0]["listen"], "unix:/tmp/r.sock");
    let group = &status["groups"][0];
    assert_eq!(group["group"], "egress");
    assert_eq!(group["refused"], 0);
    assert_eq!(group["policy"], "weighted");
    assert_eq!(group["head"], serde_json::Value::Null);
    for member in group["members"].as_array().unwrap() {
        let mut keys: Vec<&str> = member
            .as_object()
            .unwrap()
            .keys()
            .map(|k| k.as_str())
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "bytes",
                "flows",
                "health",
                "name",
                "opens",
                "rtt_floor_ms",
                "rtt_ms",
                "way",
                "weight"
            ]
        );
    }
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let mut other = tokio::net::UnixStream::connect(&path).await.unwrap();
    other
        .write_all(b"POST /route/select HTTP/1.0\r\n\r\n")
        .await
        .unwrap();
    let mut answer = String::new();
    other.read_to_string(&mut answer).await.unwrap();
    assert!(answer.starts_with("HTTP/1.0 404"), "{answer}");
    let err = read_status(&dir.path().join("none.ctl")).await.unwrap_err();
    assert!(format!("{err:#}").contains("none.ctl"), "{err:#}");
}

/// T-098 item 1: a door can have a DNS door over its own dial.
#[test]
fn route_door_dns_reaches_the_router() {
    let text = GOOD.replace(
        "    via: egress\n",
        "    via: egress\n    dns: 127.0.0.1:5353\n    dns_upstream: 10.0.0.53:53\n",
    );
    let plan = Router::plan(&parse(&text).unwrap()).unwrap();
    let (addr, upstream) = plan.doors[0].dns.clone().unwrap();
    assert_eq!(addr.to_string(), "127.0.0.1:5353");
    assert_eq!(upstream.to_string(), "10.0.0.53:53");
    assert_eq!(plan.doors[0].dial.label(), plan.doors[0].label);
    assert!(Router::plan(&parse(GOOD).unwrap()).unwrap().doors[0]
        .dns
        .is_none());
}

/// T-178: the key `policies` names the policy of a group; a file with no
/// key (the old form) keeps the weighted pick. The status shows it.
#[test]
fn route_config_policy_reaches_the_group_and_the_status() {
    let old = Router::plan(&parse(GOOD).unwrap()).unwrap();
    assert_eq!(old.groups[0].1.policy(), Policy::Weighted);
    let text = GOOD.replace("health:", "policies:\n  egress: rr\nhealth:");
    let config = parse(&text).unwrap();
    assert_eq!(config.groups["egress"], "home*2, other*1");
    let plan = Router::plan(&config).unwrap();
    assert_eq!(plan.groups[0].1.policy(), Policy::Rr);
    let status = plan.status();
    assert_eq!(status["groups"][0]["policy"], "rr");
    let names: Vec<String> = (0..4)
        .map(|_| match plan.groups[0].1.pick() {
            Selection::One(m) => m.name.clone(),
            Selection::None => panic!("up"),
        })
        .collect();
    assert_eq!(names, ["home", "other", "home", "other"]);
    let window = GOOD.replace("interval: 5", "interval: 5\n  window: 3");
    let t = parse(&window)
        .unwrap()
        .health
        .unwrap()
        .thresholds()
        .unwrap();
    assert_eq!(t.rtt_window, 3);
}

/// T-242: a rules file of a door with a shadowed `reject` does not load,
/// unless the door has `allow_shadowed_rules: true`.
#[test]
fn route_door_shadowed_reject_needs_the_key() {
    let dir = tempfile::tempdir().unwrap();
    let rules = dir.path().join("r.rules");
    std::fs::write(
        &rules,
        "domain:.example.com=home\ndomain:a.example.com=reject\n",
    )
    .unwrap();
    let text = format!(
        "networks:\n  home: direct\ndoors:\n  - listen: unix:/tmp/x.sock\n    via: home\n    rules: {}\n",
        rules.display()
    );
    let err = Router::plan(&parse(&text).unwrap()).err().unwrap();
    assert!(format!("{err:#}").contains("doors[0].rules"), "{err:#}");
    let allowed = text + "    allow_shadowed_rules: true\n";
    Router::plan(&parse(&allowed).unwrap()).unwrap();
}
