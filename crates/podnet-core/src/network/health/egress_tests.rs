use std::net::IpAddr;
use std::time::Duration;

use super::super::balance::{Balance, Member, Selection};
use super::super::fakes::{log, seen, soon, Fake};
use super::super::{Class, Dial, Target};
use super::*;

const OK: ProbeResult = ProbeResult::Ok(Duration::from_millis(12));
const T5: Duration = Duration::from_secs(5);

fn ip(text: &str) -> IpAddr {
    text.parse().unwrap()
}

fn echo() -> Target {
    Target::new("echo.example", 7)
}

/// T-097 item 9: the echo endpoint of the operator names the address that
/// the member leaves by. The check passes when it is the expected address.
#[tokio::test]
async fn router_probe_egress_passes_when_the_echo_names_the_expected_address() {
    let log = log();
    let way = Fake::up("198.51.100.4", &log).way();
    let result = probe_egress(&way, &echo(), ip("198.51.100.4"), T5).await;
    assert!(matches!(result, ProbeResult::Ok(_)), "{result:?}");
    assert_eq!(seen(&log), ["198.51.100.4"], "through its own member");
}

/// T-097 item 9: another address is `WrongEgress`; an answer that is no
/// address is a miss that is not clean (as a time out); a refusal is clean.
#[tokio::test]
async fn router_probe_egress_fails_on_another_address_or_no_address() {
    let log = log();
    let other = Fake::up("203.0.113.8", &log).way();
    assert_eq!(
        probe_egress(&other, &echo(), ip("198.51.100.4"), T5).await,
        ProbeResult::WrongEgress
    );
    let v6 = Fake::up("2001:db8::1", &log).way();
    let result = probe_egress(&v6, &echo(), ip("2001:db8::1"), T5).await;
    assert!(matches!(result, ProbeResult::Ok(_)), "{result:?}");
    let junk = Fake::up("not-an-address", &log).way();
    assert_eq!(
        probe_egress(&junk, &echo(), ip("198.51.100.4"), T5).await,
        ProbeResult::ConnectFailed { clean: false }
    );
    let down = Fake::down("198.51.100.4", &log).way();
    assert_eq!(
        probe_egress(&down, &echo(), ip("198.51.100.4"), T5).await,
        ProbeResult::ConnectFailed { clean: true }
    );
}

/// T-097 and T-098 item 3: a member that leaves by the wrong egress leaves
/// the pick at once, with no K misses and whatever the control says. The
/// group has no direct way, so its flows are refused. A pass puts it on
/// probation.
#[tokio::test]
async fn router_health_wrong_egress_leaves_the_pick_at_once() {
    let now = Instant::now();
    for control in [true, false] {
        let mut tracker = Tracker::default();
        let step = tracker.observe(
            ProbeResult::WrongEgress,
            control,
            now,
            &Thresholds::default(),
        );
        assert_eq!(step.health, Health::Down, "control {control}");
    }
    let log = log();
    let t = Thresholds::default();
    let group = Balance::new(
        "g",
        vec![
            Member::new("a", Fake::up("a", &log).way(), 1),
            Member::new("b", Fake::up("b", &log).way(), 1),
        ],
    )
    .unwrap();
    group.observe(0, ProbeResult::WrongEgress, false, now, &t);
    for _ in 0..20 {
        let Selection::One(m) = group.pick() else {
            panic!("the other member takes the flows")
        };
        assert_eq!(m.name, "b");
    }
    group.observe(1, ProbeResult::WrongEgress, true, now, &t);
    assert!(matches!(group.pick(), Selection::None));
    let err = group
        .dial(&Target::new("svc.example", 443), soon())
        .await
        .map(|_| ())
        .unwrap_err();
    assert_eq!(err.class, Class::Policy, "the kill switch, no direct way");
    assert_eq!(seen(&log), Vec::<String>::new(), "no member dialed");
    let step = group.observe(0, OK, true, now, &t).unwrap();
    assert_eq!(step.health, Health::Probation);
}

/// T-097 item 10: a refused or revoked credential is `AuthExpired` (a WARP
/// member renews its link); a time out and a temporary failure are misses
/// that are not clean; each other class is a clean miss.
#[test]
fn router_probe_result_of_a_failure_names_the_credential() {
    let of = |class| failed(&Failure::new(Stage::Connect, class, "x"));
    assert_eq!(of(Class::Refused), ProbeResult::AuthExpired);
    assert_eq!(of(Class::Revoked), ProbeResult::AuthExpired);
    for class in [Class::Timeout, Class::TempFail] {
        assert_eq!(of(class), ProbeResult::ConnectFailed { clean: false });
    }
    for class in [Class::Unavailable, Class::Protocol, Class::NotFound] {
        assert_eq!(of(class), ProbeResult::ConnectFailed { clean: true });
    }
}
