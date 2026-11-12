use std::sync::Arc;

use super::super::fakes::{carrier, log, seen, soon, Fake};
use super::*;

fn group(log: &super::super::fakes::Log) -> Balance {
    Balance::new(
        "egress",
        vec![
            Member::new("a", Fake::up("a", log).way(), 2),
            Member::new("b", Fake::up("b", log).way(), 1),
        ],
    )
    .unwrap()
    .with_seed(7)
}

/// T-096, the Prove: the pick splits by weight, and with no healthy member
/// the flow is refused and no way is dialed (no direct way).
#[tokio::test]
async fn router_group_splits_by_weight_and_fails_closed() {
    let log = log();
    let balance = group(&log);
    let mut counts = [0u32; 2];
    for _ in 0..3000 {
        match balance.pick() {
            Selection::One(m) if m.name == "a" => counts[0] += 1,
            Selection::One(_) => counts[1] += 1,
            Selection::None => panic!("each member is up"),
        }
    }
    // 2:1 within 5 % of 3000.
    assert!((1850..=2150).contains(&counts[0]), "{counts:?}");
    let conn = balance
        .dial(&Target::new("example.test", 443), soon())
        .await
        .unwrap();
    let first = carrier(conn).await;
    assert!(first == "a" || first == "b", "{first}");

    // A member that is down leaves the pick; Cooldown and Probation stay.
    balance.set_health(0, Health::Down);
    for _ in 0..50 {
        assert!(matches!(balance.pick(), Selection::One(m) if m.name == "b"));
    }
    balance.set_health(1, Health::Cooldown);
    assert!(matches!(balance.pick(), Selection::One(m) if m.name == "b"));
    balance.set_health(1, Health::Probation);
    assert!(matches!(balance.pick(), Selection::One(m) if m.name == "b"));

    // No member takes flows: the kill switch.
    balance.set_health(1, Health::Down);
    let dialed = seen(&log).len();
    let err = balance
        .dial(&Target::new("example.test", 443), soon())
        .await
        .expect_err("the kill switch");
    assert_eq!((err.stage, err.class), (Stage::Connect, Class::Policy));
    assert!(
        err.text()
            .starts_with("the group egress has no healthy member"),
        "{err}"
    );
    let err = balance.datagrams().await.expect_err("no UDP either");
    assert_eq!(err.class, Class::Policy);
    assert_eq!(seen(&log).len(), dialed, "no way was dialed");
    assert_eq!(balance.refused(), 2);
    assert_eq!(balance.label(), "group egress(a*2, b*1)");
}

#[test]
fn router_group_parses_its_spec() {
    let (name, members) = parse_group("egress=a*2, b*1,c").unwrap();
    assert_eq!(name, "egress");
    assert_eq!(members, [("a".into(), 2), ("b".into(), 1), ("c".into(), 1)]);
    for bad in [
        "egress",
        "=a",
        "egress=",
        "egress=a*0",
        "egress=a*x",
        "egress=a,a",
        "e g=a",
        "egress=*2",
    ] {
        let err = parse_group(bad)
            .err()
            .unwrap_or_else(|| panic!("{bad} parsed"));
        assert_eq!(err.class, Class::Usage, "{bad}");
    }
}

#[test]
fn router_group_refuses_a_peer_way_and_a_zero_weight() {
    let log = log();
    let peer = Way::peer(Arc::new(Fake::up("wormhole", &log)));
    assert!(Balance::new("g", vec![Member::new("w", peer, 1)]).is_err());
    let zero = Balance::new("g", vec![Member::new("a", Fake::up("a", &log).way(), 0)]);
    assert!(zero.err().unwrap().text().contains("weight 0"));
    assert!(Balance::new("g", Vec::new()).is_err());
}

/// T-180: a pin sends each flow to one member, whatever the policy and the
/// weights say; a pin on a member that is down refuses the flow (no other
/// member, no direct way); `pin(None)` gives the policy back.
#[tokio::test]
async fn route_control_pin_holds_one_member_and_fails_closed() {
    let log = log();
    let balance = group(&log);
    assert!(balance.pinned().is_none());
    assert!(balance.pin(Some("c")).is_err());
    balance.pin(Some("b")).unwrap();
    assert_eq!(balance.pinned(), Some("b".to_string()));
    assert_eq!(balance.status()["pinned"], "b");
    for _ in 0..200 {
        match balance.pick() {
            Selection::One(m) => assert_eq!(m.name, "b"),
            Selection::None => panic!("b is up"),
        }
    }
    balance.set_health(1, Health::Down);
    assert!(matches!(balance.pick(), Selection::None));
    assert_eq!(balance.refused(), 1);
    balance.pin(None).unwrap();
    assert_eq!(balance.status()["pinned"], serde_json::Value::Null);
    for _ in 0..50 {
        match balance.pick() {
            Selection::One(m) => assert_eq!(m.name, "a"),
            Selection::None => panic!("a is up"),
        }
    }
}
