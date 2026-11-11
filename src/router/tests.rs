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
