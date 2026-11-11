use std::time::Duration;

use super::super::balance::{Balance, Member, Selection};
use super::super::fakes::{log, seen, Fake};
use super::*;

const OK: ProbeResult = ProbeResult::Ok(Duration::from_millis(12));
const MISS: ProbeResult = ProbeResult::ConnectFailed { clean: true };

fn t() -> Thresholds {
    Thresholds {
        cooldown: Duration::from_secs(30),
        ..Thresholds::default()
    }
}

#[test]
fn router_health_thresholds_keep_k_above_r_above_half_k() {
    assert!(Thresholds::default().check().is_ok());
    for (k, r) in [(3, 3), (4, 1), (6, 2), (2, 2)] {
        let bad = Thresholds {
            down_after: k,
            up_after: r,
            ..Thresholds::default()
        };
        assert_eq!(bad.check().unwrap_err().class, Class::Config, "K {k} R {r}");
    }
}

#[test]
fn router_health_member_with_flapping_probe_stays_in_cooldown() {
    let (mut tracker, now) = (Tracker::default(), Instant::now());
    for i in 0..20u64 {
        let result = if i % 2 == 0 { MISS } else { OK };
        let step = tracker.observe(result, true, now + Duration::from_secs(i * 10), &t());
        assert_eq!(step.health, Health::Cooldown, "probe {i}");
    }
}

#[test]
fn router_health_goes_down_after_k_clean_misses_and_back_after_r() {
    let (mut tracker, now) = (Tracker::default(), Instant::now());
    let at = |s: u64| now + Duration::from_secs(s);
    let steps: Vec<Health> = (0..4)
        .map(|i| tracker.observe(MISS, true, at(i), &t()).health)
        .collect();
    assert_eq!(
        steps,
        [
            Health::Cooldown,
            Health::Cooldown,
            Health::Cooldown,
            Health::Down
        ]
    );
    // One success is probation, not up; R successes in a row are up.
    assert_eq!(
        tracker.observe(OK, true, at(10), &t()).health,
        Health::Probation
    );
    assert_eq!(
        tracker.observe(OK, true, at(20), &t()).health,
        Health::Probation
    );
    assert_eq!(tracker.observe(OK, true, at(30), &t()).health, Health::Up);
    assert_eq!(tracker.rtt(), Some(Duration::from_millis(12)));
}

#[test]
fn router_health_a_failed_control_or_a_time_out_keeps_the_member() {
    let (mut tracker, now) = (Tracker::default(), Instant::now());
    for i in 0..10 {
        // The control failed: each member is out, the host is the cause.
        let step = tracker.observe(MISS, false, now + Duration::from_secs(i), &t());
        assert_ne!(step.health, Health::Down);
    }
    let mut slow = Tracker::default();
    for i in 0..10 {
        let step = slow.observe(
            ProbeResult::ConnectFailed { clean: false },
            true,
            now + Duration::from_secs(i),
            &t(),
        );
        assert_ne!(
            step.health,
            Health::Down,
            "a time out is not a clean failure"
        );
    }
}

#[test]
fn router_health_cooldown_waits_its_time_before_up() {
    let (mut tracker, now) = (Tracker::default(), Instant::now());
    tracker.observe(MISS, true, now, &t());
    for s in [1, 2, 3, 4] {
        assert_eq!(
            tracker
                .observe(OK, true, now + Duration::from_secs(s), &t())
                .health,
            Health::Cooldown
        );
    }
    assert_eq!(
        tracker
            .observe(OK, true, now + Duration::from_secs(31), &t())
            .health,
        Health::Up
    );
}

#[test]
fn router_health_the_step_down_and_an_expired_credential_renew() {
    let (mut tracker, now) = (Tracker::default(), Instant::now());
    let mut renews = Vec::new();
    for i in 0..5 {
        renews.push(
            tracker
                .observe(MISS, true, now + Duration::from_secs(i), &t())
                .renew,
        );
    }
    assert_eq!(
        renews,
        [false, false, false, true, false],
        "one renew at the step down"
    );
    let mut warp = Tracker::default();
    assert!(
        warp.observe(ProbeResult::AuthExpired, true, now, &t())
            .renew
    );
}

#[tokio::test]
async fn router_health_moves_flows_off_a_dead_member() {
    let log = log();
    let group = Balance::new(
        "egress",
        vec![
            Member::new("a", Fake::up("a", &log).way(), 1),
            Member::new("b", Fake::up("b", &log).way(), 1),
        ],
    )
    .unwrap()
    .with_seed(3);
    let now = Instant::now();
    for i in 0..4 {
        group.observe(0, MISS, true, now + Duration::from_secs(i), &t());
    }
    assert_eq!(group.members()[0].health(), Health::Down);
    for _ in 0..100 {
        assert!(matches!(group.pick(), Selection::One(m) if m.name == "b"));
    }
}

#[tokio::test]
async fn router_probe_dials_through_its_own_member() {
    let log = log();
    let a = Fake::up("member-a", &log).way();
    let result = probe(
        &a,
        &Target::new("probe.example", 443),
        Duration::from_secs(5),
    )
    .await;
    assert!(matches!(result, ProbeResult::Ok(_)), "{result:?}");
    assert_eq!(seen(&log), ["member-a"]);
    let down = Fake::down("member-b", &log).way();
    let result = probe(
        &down,
        &Target::new("probe.example", 443),
        Duration::from_secs(5),
    )
    .await;
    assert_eq!(result, ProbeResult::ConnectFailed { clean: true });
}

#[test]
fn router_status_json_keys() {
    let log = log();
    let group = Balance::new(
        "egress",
        vec![Member::new("a", Fake::up("a", &log).way(), 2)],
    )
    .unwrap();
    let _ = group.pick();
    group.observe(0, OK, true, Instant::now(), &t());
    let status = group.status();
    let keys: Vec<&String> = status.as_object().unwrap().keys().collect();
    assert_eq!(keys, ["group", "members", "refused"]);
    let member = &status["members"][0];
    let keys: Vec<&String> = member.as_object().unwrap().keys().collect();
    assert_eq!(keys, ["flows", "health", "name", "rtt_ms", "way", "weight"]);
    assert_eq!(member["health"], "up");
    assert_eq!(member["flows"], 1);
    assert_eq!(member["rtt_ms"], 12.0);
}
