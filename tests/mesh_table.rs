//! The path-vector route table (T-286): the order of selection, the
//! withdrawal rules, the refusal of loops, the expiry with a fake clock, the
//! limits and the eviction at 90%.

use pod_identity::PublicKey;
use podnet::mesh_table::{Announcement, Learned, Limits, Refused, RouteTable, EXPIRY_MS};

fn key(n: u32) -> PublicKey {
    let mut b = [0u8; 32];
    b[..4].copy_from_slice(&n.to_be_bytes());
    PublicKey(b)
}

const ME: u32 = 1_000_000;

fn table() -> RouteTable {
    RouteTable::new(key(ME), Limits::default())
}

/// A direct announcement of `origin` with the expiry `now + EXPIRY_MS`.
fn direct(origin: u32, seq: u64, now: u64) -> Announcement {
    Announcement {
        origin: key(origin),
        seq,
        cost: 0,
        path: vec![key(origin)],
        expires_ms: now + EXPIRY_MS,
    }
}

/// A multi-hop announcement: `path` is origin first, sender last.
fn multi(path: &[u32], seq: u64, cost: u32, now: u64) -> Announcement {
    Announcement {
        origin: key(path[0]),
        seq,
        cost,
        path: path.iter().map(|n| key(*n)).collect(),
        expires_ms: now + EXPIRY_MS,
    }
}

fn learn(
    t: &mut RouteTable,
    from: u32,
    link: u32,
    a: &Announcement,
    now: u64,
) -> Result<Learned, Refused> {
    t.learn(key(from), link, a, now)
}

#[test]
fn new_then_updated() {
    let mut t = table();
    assert_eq!(learn(&mut t, 2, 5, &direct(2, 1, 0), 0), Ok(Learned::New));
    assert_eq!(
        learn(&mut t, 2, 5, &direct(2, 2, 0), 0),
        Ok(Learned::Updated)
    );
    let r = t.best(&key(2), 0).unwrap();
    assert_eq!((r.cost, r.hops, r.seq, r.next_hop), (5, 1, 2, key(2)));
    assert!(t.best(&key(9), 0).is_none());
}

#[test]
fn selection_lowest_cost_first() {
    let mut t = table();
    // Origin 9 through 2 (cost 10, 2 hops) and through 3 (cost 20, 1 hop... via path of 2).
    learn(&mut t, 2, 5, &multi(&[9, 2], 1, 5, 0), 0).unwrap();
    learn(&mut t, 3, 5, &multi(&[9, 4, 3], 1, 30, 0), 0).unwrap();
    let r = t.best(&key(9), 0).unwrap();
    assert_eq!((r.next_hop, r.cost), (key(2), 10));
}

#[test]
fn selection_fewest_hops_on_equal_cost() {
    let mut t = table();
    learn(&mut t, 3, 5, &multi(&[9, 4, 3], 1, 5, 0), 0).unwrap();
    learn(&mut t, 2, 5, &multi(&[9, 2], 1, 5, 0), 0).unwrap();
    let r = t.best(&key(9), 0).unwrap();
    assert_eq!((r.next_hop, r.hops), (key(2), 2));
}

#[test]
fn selection_newest_seq_on_equal_cost_and_hops() {
    let mut t = table();
    learn(&mut t, 2, 5, &multi(&[9, 2], 4, 5, 0), 0).unwrap();
    learn(&mut t, 3, 5, &multi(&[9, 3], 7, 5, 0), 0).unwrap();
    let r = t.best(&key(9), 0).unwrap();
    assert_eq!((r.next_hop, r.seq), (key(3), 7));
    let ranked = t.ranked(&key(9), 0);
    assert_eq!(ranked.len(), 2);
    assert_eq!(ranked[1].next_hop, key(2));
}

#[test]
fn cost_beats_hops_and_seq() {
    let mut t = table();
    learn(&mut t, 2, 1, &multi(&[9, 4, 5, 2], 9, 1, 0), 0).unwrap();
    learn(&mut t, 3, 50, &direct_via(9, 3, 9), 0).unwrap();
    assert_eq!(t.best(&key(9), 0).unwrap().next_hop, key(2));
}

fn direct_via(origin: u32, via: u32, seq: u64) -> Announcement {
    multi(&[origin, via], seq, 0, 0)
}

#[test]
fn loop_through_this_node_is_refused() {
    let mut t = table();
    let a = multi(&[9, ME, 2], 1, 1, 0);
    assert_eq!(learn(&mut t, 2, 1, &a, 0), Err(Refused::Loop));
    assert!(t.is_empty());
}

#[test]
fn repeated_node_in_path_is_refused() {
    let mut t = table();
    let a = multi(&[9, 4, 5, 4, 2], 1, 1, 0);
    assert_eq!(learn(&mut t, 2, 1, &a, 0), Err(Refused::Loop));
}

#[test]
fn bad_path_and_own_origin_are_refused() {
    let mut t = table();
    let mut a = multi(&[9, 2], 1, 1, 0);
    a.path.clear();
    assert_eq!(learn(&mut t, 2, 1, &a, 0), Err(Refused::BadPath));
    // The sender is not the last node.
    assert_eq!(
        learn(&mut t, 3, 1, &multi(&[9, 2], 1, 1, 0), 0),
        Err(Refused::BadPath)
    );
    // The path does not start at the origin.
    let mut b = multi(&[9, 2], 1, 1, 0);
    b.origin = key(8);
    assert_eq!(learn(&mut t, 2, 1, &b, 0), Err(Refused::BadPath));
    assert_eq!(
        learn(&mut t, 2, 1, &direct(ME, 1, 0), 0),
        Err(Refused::OwnOrigin)
    );
}

#[test]
fn hop_limit_is_16() {
    let mut t = table();
    let ok: Vec<u32> = (1..=16).collect();
    let a = multi(&ok, 1, 1, 0);
    assert_eq!(learn(&mut t, 16, 1, &a, 0), Ok(Learned::New));
    let long: Vec<u32> = (1..=17).collect();
    let b = multi(&long, 1, 1, 0);
    assert_eq!(learn(&mut t, 17, 1, &b, 0), Err(Refused::TooManyHops));
}

#[test]
fn expired_announcement_is_refused() {
    let mut t = table();
    let a = direct(2, 1, 0);
    assert_eq!(learn(&mut t, 2, 1, &a, a.expires_ms), Err(Refused::Expired));
    assert_eq!(
        learn(&mut t, 2, 1, &a, a.expires_ms + 1),
        Err(Refused::Expired)
    );
}

#[test]
fn seq_must_rise() {
    let mut t = table();
    learn(&mut t, 2, 1, &multi(&[9, 2], 5, 1, 0), 0).unwrap();
    // The same seq and an older seq from the same hop.
    assert_eq!(
        learn(&mut t, 2, 1, &multi(&[9, 2], 5, 1, 0), 0),
        Err(Refused::StaleSeq)
    );
    assert_eq!(
        learn(&mut t, 2, 1, &multi(&[9, 2], 4, 1, 0), 0),
        Err(Refused::StaleSeq)
    );
    // An older seq from another hop: the origin showed a newer one.
    assert_eq!(
        learn(&mut t, 3, 1, &multi(&[9, 3], 4, 1, 0), 0),
        Err(Refused::StaleSeq)
    );
    // The same seq from another hop is a second route.
    assert_eq!(
        learn(&mut t, 3, 1, &multi(&[9, 3], 5, 1, 0), 0),
        Ok(Learned::Updated)
    );
}

#[test]
fn direct_route_ends_at_expiry() {
    let mut t = table();
    let a = direct(2, 1, 0);
    learn(&mut t, 2, 1, &a, 0).unwrap();
    assert!(t.best(&key(2), a.expires_ms - 1).is_some());
    assert!(t.best(&key(2), a.expires_ms).is_none());
    assert_eq!(t.purge(a.expires_ms), 1);
    assert!(t.is_empty());
}

#[test]
fn multi_hop_route_is_held_one_interval_after_expiry() {
    let mut t = table();
    let a = multi(&[9, 2], 1, 1, 0);
    learn(&mut t, 2, 1, &a, 0).unwrap();
    let e = a.expires_ms;
    assert!(!t.best(&key(9), e - 1).unwrap().held(e - 1));
    // Held: still used, marked held.
    assert!(t.best(&key(9), e).unwrap().held(e));
    assert!(t.best(&key(9), e + EXPIRY_MS - 1).is_some());
    // Ended.
    assert!(t.best(&key(9), e + EXPIRY_MS).is_none());
    assert_eq!(t.purge(e + EXPIRY_MS), 1);
    assert!(t.is_empty());
}

#[test]
fn fresh_route_beats_held_route() {
    let mut t = table();
    // The cheap route expires first; the dear route is announced later.
    learn(&mut t, 2, 1, &multi(&[9, 2], 1, 1, 0), 0).unwrap();
    let later = 20_000;
    learn(&mut t, 3, 1, &multi(&[9, 4, 3], 2, 50, later), later).unwrap();
    let now = EXPIRY_MS + 1; // the first is held, the second is fresh
    assert_eq!(t.best(&key(9), now).unwrap().next_hop, key(3));
}

#[test]
fn session_close_ends_direct_route_at_once() {
    let mut t = table();
    learn(&mut t, 2, 1, &direct(2, 1, 0), 0).unwrap();
    assert_eq!(t.session_closed(&key(2), 10), 1);
    assert!(t.best(&key(2), 10).is_none());
    assert!(t.is_empty());
}

#[test]
fn session_close_downs_multi_hop_routes_for_one_interval() {
    let mut t = table();
    learn(&mut t, 2, 1, &multi(&[9, 2], 1, 1, 0), 0).unwrap();
    learn(&mut t, 3, 1, &multi(&[9, 4, 3], 1, 40, 0), 0).unwrap();
    assert_eq!(t.session_closed(&key(2), 100), 0);
    // The route through 2 cannot be used; the route through 3 takes over.
    assert_eq!(t.best(&key(9), 100).unwrap().next_hop, key(3));
    assert_eq!(t.ranked(&key(9), 100).len(), 1);
    // The down route is still in the table until one interval has passed.
    assert_eq!(t.purge(100 + EXPIRY_MS - 1), 0);
    assert_eq!(t.purge(100 + EXPIRY_MS), 1);
    assert_eq!(t.best(&key(9), 100 + EXPIRY_MS).unwrap().next_hop, key(3));
}

#[test]
fn announcement_after_close_brings_the_route_back() {
    let mut t = table();
    learn(&mut t, 2, 1, &multi(&[9, 2], 1, 1, 0), 0).unwrap();
    t.session_closed(&key(2), 100);
    assert!(t.best(&key(9), 100).is_none());
    learn(&mut t, 2, 1, &multi(&[9, 2], 2, 1, 200), 200).unwrap();
    assert_eq!(t.best(&key(9), 200).unwrap().next_hop, key(2));
}

#[test]
fn origin_route_count_is_bounded() {
    let limits = Limits {
        routes_per_origin: 2,
        ..Limits::default()
    };
    let mut t = RouteTable::new(key(ME), limits);
    learn(&mut t, 2, 10, &multi(&[9, 2], 1, 0, 0), 0).unwrap();
    learn(&mut t, 3, 20, &multi(&[9, 3], 1, 0, 0), 0).unwrap();
    // Worse than both: refused.
    assert_eq!(
        learn(&mut t, 4, 30, &multi(&[9, 4], 1, 0, 0), 0),
        Err(Refused::Full)
    );
    // Better than the worst: replaces it.
    assert_eq!(
        learn(&mut t, 5, 1, &multi(&[9, 5], 1, 0, 0), 0),
        Ok(Learned::Updated)
    );
    let ranked = t.ranked(&key(9), 0);
    assert_eq!(ranked.len(), 2);
    assert_eq!((ranked[0].next_hop, ranked[1].next_hop), (key(5), key(2)));
}

#[test]
fn eviction_starts_at_90_percent_and_counts() {
    let limits = Limits {
        origins: 100,
        ..Limits::default()
    };
    assert_eq!(limits.high_water(), 90);
    let mut t = RouteTable::new(key(ME), limits);
    for n in 0..90u32 {
        // Origin n+10 through a next hop; cost grows with n.
        let a = multi(&[n + 10, 5], 1, n, 0);
        assert_eq!(learn(&mut t, 5, 0, &a, 0), Ok(Learned::New));
    }
    assert_eq!(t.len(), 90);
    assert_eq!(t.evictions(), 0);
    // The 91st origin evicts the worst one (cost 89, origin 99).
    let a = multi(&[500, 5], 1, 0, 0);
    assert_eq!(learn(&mut t, 5, 0, &a, 0), Ok(Learned::New));
    assert_eq!(t.len(), 90);
    assert_eq!(t.evictions(), 1);
    assert!(t.best(&key(99), 0).is_none());
    assert!(t.best(&key(500), 0).is_some());
    assert!(t.best(&key(10), 0).is_some());
    // The table never passes the limit, and each new origin evicts one.
    for n in 0..50u32 {
        learn(&mut t, 5, 0, &multi(&[1000 + n, 5], 1, 0, 0), 0).unwrap();
    }
    assert!(t.len() <= 90);
    assert_eq!(t.evictions(), 51);
}

#[test]
fn direct_routes_are_not_evicted() {
    let limits = Limits {
        origins: 10,
        ..Limits::default()
    };
    let mut t = RouteTable::new(key(ME), limits);
    // 9 direct neighbours fill the table to the high water mark.
    for n in 0..9u32 {
        assert_eq!(
            learn(&mut t, 20 + n, 0, &direct(20 + n, 1, 0), 0),
            Ok(Learned::New)
        );
    }
    assert_eq!(
        learn(&mut t, 5, 0, &multi(&[500, 5], 1, 0, 0), 0),
        Err(Refused::Full)
    );
    assert_eq!(t.evictions(), 0);
    assert_eq!(t.len(), 9);
}

#[test]
fn ended_routes_are_dropped_before_eviction() {
    let limits = Limits {
        origins: 10,
        ..Limits::default()
    };
    let mut t = RouteTable::new(key(ME), limits);
    for n in 0..9u32 {
        learn(&mut t, 20 + n, 0, &direct(20 + n, 1, 0), 0).unwrap();
    }
    // All nine direct routes ended; a new origin fits with no eviction.
    let now = EXPIRY_MS;
    assert_eq!(
        learn(&mut t, 5, 0, &multi(&[500, 5], 1, 0, now), now),
        Ok(Learned::New)
    );
    assert_eq!(t.evictions(), 0);
    assert_eq!(t.len(), 1);
}

#[test]
fn default_limits_are_the_rules_of_the_issue() {
    let l = Limits::default();
    assert_eq!((l.origins, l.max_hops, l.expiry_ms), (4096, 16, 30_000));
    assert_eq!(l.high_water(), 3686);
}
