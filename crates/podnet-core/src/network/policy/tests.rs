use std::time::Duration;

use super::super::balance::{Balance, Member, Selection};
use super::super::fakes::{log, Fake};
use super::super::health::{ProbeResult, Thresholds};
use super::*;

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn group(policy: Policy, names: &[&'static str]) -> Balance {
    let log = log();
    let members = names
        .iter()
        .map(|n| Member::new(n, Fake::up(n, &log).way(), 1))
        .collect();
    Balance::new("g", members)
        .unwrap()
        .with_seed(5)
        .with_policy(policy)
}

fn name(b: &Balance, now: Instant) -> String {
    match b.pick_at(now) {
        Selection::One(m) => m.name.clone(),
        Selection::None => "none".into(),
    }
}

/// Feed a passing probe time to the member `i`.
fn feed(b: &Balance, i: usize, rtt: Duration) {
    b.observe(
        i,
        ProbeResult::Ok(rtt),
        true,
        Instant::now(),
        &Thresholds::default(),
    );
}

/// T-178, the Prove: each policy picks by its name, with a seed and an
/// injected clock.
#[test]
fn group_policy_picks_by_name() {
    let now = Instant::now();

    // rr: the members in turn; a member that is down leaves the turn.
    let b = group(Policy::Rr, &["a", "b", "c"]);
    let got: Vec<String> = (0..6).map(|_| name(&b, now)).collect();
    assert_eq!(got, ["a", "b", "c", "a", "b", "c"]);
    b.set_health(1, Health::Down);
    let got: Vec<String> = (0..4).map(|_| name(&b, now)).collect();
    assert!(got.iter().all(|n| n != "b"), "{got:?}");
    assert!(got.contains(&"a".to_string()) && got.contains(&"c".to_string()));

    // protect: a member in cooldown gets no flow while a member is up.
    let b = group(Policy::Protect, &["a", "b"]);
    b.set_health(0, Health::Cooldown);
    assert!((0..100).all(|_| name(&b, now) == "b"));
    b.set_health(0, Health::Probation);
    assert!((0..100).all(|_| name(&b, now) == "b"));
    // No member is up: the degraded member takes the flows (no refusal).
    b.set_health(1, Health::Cooldown);
    assert!((0..20).any(|_| name(&b, now) == "a"));
    // Only a member that is down is refused.
    b.set_health(0, Health::Down);
    b.set_health(1, Health::Down);
    assert_eq!(name(&b, now), "none");

    // minrtt: the head is the member with the least floor.
    let b = group(Policy::MinRtt, &["a", "b"]);
    feed(&b, 0, ms(40));
    feed(&b, 1, ms(10));
    b.end_round();
    assert!((0..20).all(|_| name(&b, now) == "b"));
    b.set_health(1, Health::Down);
    assert!((0..20).all(|_| name(&b, now) == "a"), "a head that is down");
    assert_eq!(b.status()["policy"], "minrtt");

    // leastload: see the other tests; here a busy member loses.
    let b = group(Policy::LeastLoad, &["a", "b"]);
    b.members()[0].add_bytes(200_000, now);
    assert!((0..20).all(|_| name(&b, now) == "b"));
}

/// 35 idle flows against 2 busy bulk flows: a flow-count policy picks the
/// member with the bulk flows, `leastload` does not.
#[test]
fn group_policy_leastload_reads_bytes_not_flow_counts() {
    let now = Instant::now();
    let b = group(Policy::LeastLoad, &["a", "b"]);
    // "a" has 2 bulk flows: much traffic. "b" has 35 flows with no traffic.
    b.members()[0].add_bytes(400_000, now);
    let mut guards = Vec::new();
    for _ in 0..35 {
        let m = &b.members()[1];
        guards.push(m.open());
    }
    assert!((0..10).all(|_| name(&b, now) == "b"));
    // The same state under a count of flows would pick "a".
    assert!(b.members()[0].opens() < 35);
}

#[test]
fn group_policy_leastload_spreads_opens_and_returns_them() {
    let now = Instant::now();
    let b = group(Policy::LeastLoad, &["a", "b", "c"]);
    // Three opens in one tick never land on one member.
    let mut held = Vec::new();
    let mut names = Vec::new();
    for _ in 0..3 {
        let (sel, guard) = b.pick_open(now);
        names.push(match sel {
            Selection::One(m) => m.name.clone(),
            Selection::None => panic!("members are up"),
        });
        held.push(guard);
    }
    names.sort();
    assert_eq!(names, ["a", "b", "c"]);
    // A flow dropped before its first byte returns its count.
    let opens = |i: usize| b.members()[i].opens();
    assert_eq!((opens(0), opens(1), opens(2)), (1, 1, 1));
    held.clear();
    assert_eq!((opens(0), opens(1), opens(2)), (0, 0, 0));
    // The tie-break rotates: equal members share the picks.
    let got: Vec<String> = (0..6).map(|_| name(&b, now)).collect();
    for n in ["a", "b", "c"] {
        assert_eq!(got.iter().filter(|g| *g == n).count(), 2, "{got:?}");
    }
}

#[test]
fn group_policy_leastload_bucket_falls_back_with_the_clock() {
    let t0 = Instant::now();
    let b = group(Policy::LeastLoad, &["a", "b"]);
    b.members()[0].add_bytes(100_000, t0);
    b.members()[1].add_bytes(10_000, t0);
    // "a" is in the top bucket, "b" in the middle one.
    assert_eq!(name(&b, t0), "b");
    // Ten seconds later both loads are under 4096: the tie-break rules.
    let later = t0 + Duration::from_secs(10);
    let got: Vec<String> = (0..4).map(|_| name(&b, later)).collect();
    assert!(
        got.contains(&"a".to_string()) && got.contains(&"b".to_string()),
        "{got:?}"
    );
}

fn snap(floor: Option<u64>) -> Snapshot {
    Snapshot {
        weight: 1,
        health: Health::Up,
        rtt_floor: floor.map(ms),
        opens: 0,
        load: 0.0,
    }
}

/// The head of `minrtt` moves only for a margin of max(5 ms, 10%) that
/// holds for the dwell of probe rounds.
#[test]
fn group_policy_minrtt_head_needs_a_margin_and_a_dwell() {
    let mut head = next_head(Head::default(), &[snap(Some(100)), snap(Some(120))]);
    assert_eq!(head.index, Some(0));
    // 92 ms is not better by 10 ms (10% of 100): the head stays.
    for _ in 0..10 {
        head = next_head(head, &[snap(Some(100)), snap(Some(92))]);
    }
    assert_eq!(head.index, Some(0));
    // 85 ms is better by the margin: the head moves at the third round.
    let better = [snap(Some(100)), snap(Some(85))];
    head = next_head(head, &better);
    head = next_head(head, &better);
    assert_eq!(head.index, Some(0));
    head = next_head(head, &better);
    assert_eq!(head.index, Some(1));
    // A round with no margin resets the count.
    let mut head = Head {
        index: Some(0),
        ..Head::default()
    };
    head = next_head(head, &better);
    head = next_head(head, &better);
    head = next_head(head, &[snap(Some(100)), snap(Some(99))]);
    head = next_head(head, &better);
    head = next_head(head, &better);
    assert_eq!(head.index, Some(0));
    // The least margin is 5 ms: a floor of 20 ms needs 15 ms.
    let mut head = next_head(Head::default(), &[snap(Some(20)), snap(Some(30))]);
    for _ in 0..5 {
        head = next_head(head, &[snap(Some(20)), snap(Some(16))]);
    }
    assert_eq!(head.index, Some(0));
    // No floor at all: no head, and the pick falls back to the weights.
    assert_eq!(
        next_head(Head::default(), &[snap(None), snap(None)]).index,
        None
    );
}
