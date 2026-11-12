use super::*;

/// A TCP echo endpoint of the operator: it writes the address that it sees
/// for the peer, and closes.
async fn echo(says: &'static str) -> std::net::SocketAddr {
    use tokio::io::AsyncWriteExt as _;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let _ = stream.write_all(format!("{says}\n").as_bytes()).await;
            let _ = stream.shutdown().await;
        }
    });
    addr
}

fn file(extra: &str) -> String {
    format!(
        "networks:\n  home: direct\n  other: direct\ngroups:\n  egress: home*1, other*1\nhealth:\n  target: 192.0.2.1:443\n{extra}doors:\n  - listen: unix:/tmp/r.sock\n    via: egress\n"
    )
}

/// T-097 item 9: `health.egress` is operator input; each error names the key.
#[test]
fn route_config_health_egress_errors_name_the_key() {
    let ok = parse(&file("  egress:\n    home: 192.0.2.7:7=198.51.100.4\n")).unwrap();
    let checks = ok.health.as_ref().unwrap().egress_checks(&ok).unwrap();
    assert_eq!(
        checks["home"].1,
        "198.51.100.4".parse::<std::net::IpAddr>().unwrap()
    );
    let cases = [
        (
            "  egress:\n    nowhere: 192.0.2.7:7=198.51.100.4\n",
            "health.egress.nowhere",
        ),
        ("  egress:\n    home: 192.0.2.7:7\n", "health.egress.home"),
        (
            "  egress:\n    home: 192.0.2.7:7=not-an-ip\n",
            "health.egress.home",
        ),
        (
            "  egress:\n    home: 192.0.2.7=198.51.100.4\n",
            "health.egress.home",
        ),
        (
            "  egress:\n    home: 192.0.2.7:0=198.51.100.4\n",
            "health.egress.home",
        ),
    ];
    for (extra, key) in cases {
        let err = format!("{:#}", parse(&file(extra)).expect_err(key));
        assert!(err.contains(key), "{key}: {err}");
    }
}

/// T-097 item 9, T-098 item 3: a member whose echo names another address
/// leaves the pick in one round, and the group never falls back to direct.
#[tokio::test]
async fn route_health_wrong_egress_member_leaves_the_pick() {
    crate::test_host::need_loopback!();
    let (good, bad) = (echo("127.0.0.1").await, echo("203.0.113.5").await);
    let text = file(&format!(
        "  egress:\n    home: {good}=127.0.0.1\n    other: {bad}=127.0.0.1\n"
    ))
    .replace("192.0.2.1:443", &good.to_string());
    let router = Router::up(&parse(&text).unwrap()).await.unwrap();
    router.probe_round().await;
    let members = router.status()["groups"][0]["members"].clone();
    assert_eq!(members[0]["name"], "home");
    assert_eq!(members[0]["health"], "up");
    assert_eq!(members[1]["name"], "other");
    assert_eq!(members[1]["health"], "down", "the wrong egress");
    let target: Target = "example.com:443".parse().unwrap();
    for _ in 0..20 {
        let (member, _) = router.decide(0, &target).member.unwrap();
        assert_eq!(member, "home");
    }
}
