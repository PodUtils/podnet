use std::sync::Arc;
use std::time::Duration;

use crate::router::{parse, Router};

const FILE: &str = "networks:\n  home: direct\n  other: direct\ngroups:\n  egress: home*2, other*1\nhealth:\n  target: 192.0.2.1:443\n  interval: 5\ndoors:\n  - listen: unix:/tmp/r.sock\n    via: egress\n";

fn router() -> Arc<Router> {
    Arc::new(Router::plan(&parse(FILE).unwrap()).unwrap())
}

/// The value of the row `name{labels}` of a text, if it has one.
fn row<'a>(text: &'a str, name: &str, labels: &str) -> Option<&'a str> {
    let head = format!("{name}{{{labels}}} ");
    text.lines().find_map(|l| l.strip_prefix(head.as_str()))
}

/// T-097 item 4: the metrics have one row per member and metric: the health,
/// the round-trip time, the flows, the bytes, and the refusals of the group.
#[test]
fn router_metrics_rows_follow_the_status() {
    let router = router();
    router.groups[0].1.pin(Some("other")).unwrap();
    for _ in 0..3 {
        let _ = router.groups[0].1.pick();
    }
    router.groups[0].1.members()[1].add_bytes(500, tokio::time::Instant::now());
    router.groups[0]
        .1
        .set_health(0, crate::network::Health::Down);
    let text = router.metrics();
    let (g, h, o) = (
        "group=\"egress\"",
        "group=\"egress\",member=\"home\"",
        "group=\"egress\",member=\"other\"",
    );
    assert_eq!(row(&text, "podnet_route_member_up", h), Some("0"));
    assert_eq!(row(&text, "podnet_route_member_takes_flows", h), Some("0"));
    assert_eq!(row(&text, "podnet_route_member_up", o), Some("1"));
    assert_eq!(row(&text, "podnet_route_member_flows_total", o), Some("3"));
    assert_eq!(
        row(&text, "podnet_route_member_bytes_total", o),
        Some("500")
    );
    assert_eq!(row(&text, "podnet_route_member_weight", h), Some("2"));
    assert_eq!(row(&text, "podnet_route_group_refused_total", g), Some("0"));
    assert_eq!(
        row(&text, "podnet_route_member_rtt_seconds", h),
        None,
        "no passing probe yet: no row"
    );
    assert!(text.contains("# TYPE podnet_route_member_bytes_total counter"));
    assert!(text.contains("# TYPE podnet_route_member_up gauge"));
}

/// T-097 item 4: a passing probe gives the round-trip rows.
#[test]
fn router_metrics_rows_hold_the_round_trip_time() {
    let router = router();
    let t = crate::network::Thresholds::default();
    router.groups[0].1.observe(
        0,
        crate::network::ProbeResult::Ok(Duration::from_millis(40)),
        true,
        tokio::time::Instant::now(),
        &t,
    );
    let text = router.metrics();
    let h = "group=\"egress\",member=\"home\"";
    assert_eq!(
        row(&text, "podnet_route_member_rtt_seconds", h),
        Some("0.04")
    );
    assert_eq!(
        row(&text, "podnet_route_member_rtt_floor_seconds", h),
        Some("0.04")
    );
}

/// T-097 item 4: the control socket serves the rows at `GET /route/metrics`
/// in the text format, and `read_metrics` reads them.
#[tokio::test]
async fn router_metrics_over_the_control_socket() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("r.ctl");
    let router = router();
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    tokio::spawn(router.clone().serve_control(listener));
    let text = crate::router::control::read_metrics(&path).await.unwrap();
    assert_eq!(text, router.metrics());
    assert!(text.contains("podnet_route_member_up{"), "{text}");
    let err = crate::router::control::request(&path, "POST", "/route/metrics", None)
        .await
        .unwrap_err();
    assert!(format!("{err:#}").contains("405"), "{err:#}");
}
