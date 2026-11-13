//! The signed route announcement (T-285): the encoding, the signature, each
//! drop rule, the skew both ways, each bound, a tampered byte and the feed of
//! a verified announcement to the route table.

use pod_identity::{Identity, PublicKey};
use podnet::mesh_announce::*;
use podnet::mesh_table::{Limits, RouteTable};

const NOW: u64 = 1_000_000;

fn id(n: u8) -> Identity {
    Identity::from_seed(&[n; 32])
}

fn key(n: u32) -> PublicKey {
    let mut b = [0u8; 32];
    b[..4].copy_from_slice(&n.to_be_bytes());
    PublicKey(b)
}

fn body(who: &Identity, seq: u64, issued: u64) -> Body {
    Body {
        origin: who.public(),
        seq,
        issued_ms: issued,
        expires_ms: issued + LIFETIME_MS,
        neighbours: vec![
            Neighbour {
                key: key(2),
                cost: 7,
            },
            Neighbour {
                key: key(1),
                cost: 3,
            },
        ],
        services: vec!["web".into(), "git".into()],
    }
}

fn wire(who: &Identity, seq: u64, issued: u64) -> Vec<u8> {
    encode(who, &body(who, seq, issued), &[who.public()]).unwrap()
}

fn me() -> PublicKey {
    id(99).public()
}

#[test]
fn round_trip_and_canonical_order() {
    let a = id(1);
    let w = wire(&a, 5, NOW);
    let v = verify(&w, &me(), NOW).unwrap();
    assert_eq!(v.body.seq, 5);
    assert_eq!(v.body.origin, a.public());
    // Sorted: key(1) before key(2); "git" before "web".
    assert_eq!(v.body.neighbours[0].key, key(1));
    assert_eq!(v.body.services, vec!["git".to_string(), "web".to_string()]);
    assert_eq!(v.path, vec![a.public()]);
    // The same body in another order gives the same bytes.
    let mut b = body(&a, 5, NOW);
    b.neighbours.reverse();
    b.services.reverse();
    assert_eq!(encode(&a, &b, &[a.public()]).unwrap(), w);
}

#[test]
fn fixed_field_order() {
    let a = id(1);
    let w = wire(&a, 5, NOW);
    assert_eq!(w[0], VERSION);
    assert_eq!(&w[1..33], &a.public().0);
    assert_eq!(&w[33..41], &5u64.to_be_bytes());
    assert_eq!(&w[41..49], &NOW.to_be_bytes());
    assert_eq!(&w[49..57], &(NOW + LIFETIME_MS).to_be_bytes());
    assert_eq!(&w[57..59], &2u16.to_be_bytes());
}

#[test]
fn every_flipped_byte_is_dropped() {
    let a = id(1);
    let w = wire(&a, 5, NOW);
    for i in 0..w.len() {
        let mut t = w.clone();
        t[i] ^= 0x01;
        // The path bytes are not signed: a change there is a path rule.
        let path_start = w.len() - 33;
        let r = verify(&t, &me(), NOW);
        if i < path_start {
            assert!(r.is_err(), "byte {i} changed and was accepted");
        }
    }
}

#[test]
fn tampered_cost_is_a_bad_signature() {
    let a = id(1);
    let mut w = wire(&a, 5, NOW);
    // The cost of the first neighbour: after the count at 57..59 and its key.
    w[59 + 32 + 3] ^= 0x10;
    assert_eq!(verify(&w, &me(), NOW), Err(Drop::BadSignature));
}

#[test]
fn signature_of_another_key_is_dropped() {
    let a = id(1);
    let mut b = body(&a, 5, NOW);
    b.origin = id(2).public();
    // `a` signs a body that names `b` as origin.
    let w = encode(&a, &b, &[b.origin]).unwrap();
    assert_eq!(verify(&w, &me(), NOW), Err(Drop::BadSignature));
}

#[test]
fn skew_ahead_boundary() {
    let a = id(1);
    let ok = wire(&a, 1, NOW + SKEW_MS);
    assert!(verify(&ok, &me(), NOW).is_ok());
    let bad = wire(&a, 1, NOW + SKEW_MS + 1);
    assert_eq!(verify(&bad, &me(), NOW), Err(Drop::FromFuture));
}

#[test]
fn skew_behind_boundary() {
    let a = id(1);
    // Issued SKEW_MS ago with a life of 30 s is expired: use a receiver
    // whose clock is just inside the window of an unexpired announcement.
    let mut b = body(&a, 1, NOW);
    b.expires_ms = NOW + LIFETIME_MS;
    let w = encode(&a, &b, &[a.public()]).unwrap();
    assert!(verify(&w, &me(), NOW + LIFETIME_MS - 1).is_ok());
    // Past the skew window the drop is TooOld, before the expiry check.
    assert_eq!(verify(&w, &me(), NOW + SKEW_MS + 1), Err(Drop::TooOld));
    // Inside the skew window but past the expiry: Expired.
    assert_eq!(verify(&w, &me(), NOW + LIFETIME_MS), Err(Drop::Expired));
    assert_eq!(verify(&w, &me(), NOW + SKEW_MS), Err(Drop::Expired));
}

#[test]
fn lifetime_rules() {
    let a = id(1);
    let mut b = body(&a, 1, NOW);
    b.expires_ms = NOW;
    let w = encode(&a, &b, &[a.public()]).unwrap();
    assert_eq!(verify(&w, &me(), NOW), Err(Drop::BadLifetime));
    b.expires_ms = NOW + LIFETIME_MS + 1;
    let w = encode(&a, &b, &[a.public()]).unwrap();
    assert_eq!(verify(&w, &me(), NOW), Err(Drop::BadLifetime));
}

#[test]
fn path_with_self_is_dropped() {
    let a = id(1);
    let m = me();
    let w = encode(&a, &body(&a, 1, NOW), &[a.public(), m]).unwrap();
    assert_eq!(verify(&w, &m, NOW), Err(Drop::PathHasSelf));
}

#[test]
fn path_must_start_at_origin_and_not_repeat() {
    let a = id(1);
    // The encoder refuses a bad path, so build the wire by hand.
    let good = wire(&a, 1, NOW);
    let mut w = good[..good.len() - 33].to_vec();
    w.push(1);
    w.extend_from_slice(&key(5).0);
    assert_eq!(verify(&w, &me(), NOW), Err(Drop::PathNotFromOrigin));
    let mut w = good[..good.len() - 33].to_vec();
    w.push(3);
    for k in [a.public(), key(5), key(5)] {
        w.extend_from_slice(&k.0);
    }
    assert_eq!(verify(&w, &me(), NOW), Err(Drop::PathRepeats));
    assert_eq!(
        encode(&a, &body(&a, 1, NOW), &[key(5)]),
        Err(EncodeError::BadPath)
    );
    assert_eq!(
        encode(&a, &body(&a, 1, NOW), &[]),
        Err(EncodeError::BadPath)
    );
}

#[test]
fn own_origin_is_dropped() {
    let a = id(1);
    let w = wire(&a, 1, NOW);
    assert_eq!(verify(&w, &a.public(), NOW), Err(Drop::PathHasSelf));
}

#[test]
fn path_bounds() {
    let a = id(1);
    let mut path = vec![a.public()];
    path.extend((10..25).map(key));
    assert_eq!(path.len(), 16);
    let w = encode(&a, &body(&a, 1, NOW), &path).unwrap();
    assert!(verify(&w, &me(), NOW).is_ok());
    path.push(key(100));
    assert_eq!(
        encode(&a, &body(&a, 1, NOW), &path),
        Err(EncodeError::BadPath)
    );
    // A wire with 17 nodes is dropped.
    let good = wire(&a, 1, NOW);
    let mut w = good[..good.len() - 33].to_vec();
    w.push(17);
    w.extend_from_slice(&a.public().0);
    for n in 0..16 {
        w.extend_from_slice(&key(10 + n).0);
    }
    assert_eq!(verify(&w, &me(), NOW), Err(Drop::BadPathLength));
}

#[test]
fn neighbour_bound() {
    let a = id(1);
    let mut b = body(&a, 1, NOW);
    b.neighbours = (0..64)
        .map(|n| Neighbour {
            key: key(n),
            cost: n,
        })
        .collect();
    let w = encode(&a, &b, &[a.public()]).unwrap();
    assert_eq!(verify(&w, &me(), NOW).unwrap().body.neighbours.len(), 64);
    b.neighbours.push(Neighbour {
        key: key(64),
        cost: 0,
    });
    assert_eq!(
        encode(&a, &b, &[a.public()]),
        Err(EncodeError::TooManyNeighbours)
    );
    // A wire with a count of 65 is dropped before it reads the entries.
    let mut w = wire(&a, 1, NOW);
    w[57..59].copy_from_slice(&65u16.to_be_bytes());
    assert_eq!(verify(&w, &me(), NOW), Err(Drop::TooManyNeighbours));
}

#[test]
fn service_bounds() {
    let a = id(1);
    let mut b = body(&a, 1, NOW);
    b.services = (0..16).map(|n| format!("s{n}")).collect();
    let w = encode(&a, &b, &[a.public()]).unwrap();
    assert_eq!(verify(&w, &me(), NOW).unwrap().body.services.len(), 16);
    b.services.push("s16".into());
    assert_eq!(
        encode(&a, &b, &[a.public()]),
        Err(EncodeError::TooManyServices)
    );
    b.services = vec!["x".repeat(64)];
    assert!(encode(&a, &b, &[a.public()]).is_ok());
    b.services = vec!["x".repeat(65)];
    assert_eq!(
        encode(&a, &b, &[a.public()]),
        Err(EncodeError::BadServiceName)
    );
    b.services = vec![String::new()];
    assert_eq!(
        encode(&a, &b, &[a.public()]),
        Err(EncodeError::BadServiceName)
    );
    b.services = vec!["a\nb".into()];
    assert_eq!(
        encode(&a, &b, &[a.public()]),
        Err(EncodeError::BadServiceName)
    );
    b.services = vec!["a".into(), "a".into()];
    assert_eq!(encode(&a, &b, &[a.public()]), Err(EncodeError::Repeat));
}

#[test]
fn service_count_on_the_wire_is_bounded() {
    let a = id(1);
    let mut b = body(&a, 1, NOW);
    b.neighbours.clear();
    b.services.clear();
    let mut w = encode(&a, &b, &[a.public()]).unwrap();
    // The service count follows the neighbour count at 57..59.
    w[59..61].copy_from_slice(&17u16.to_be_bytes());
    assert_eq!(verify(&w, &me(), NOW), Err(Drop::TooManyServices));
}

#[test]
fn body_limit_is_8_kib() {
    // The largest body that the other bounds allow is below 8 KiB:
    // 59 + 64 * 36 + 2 + 16 * 65 = 3405 bytes. The limit stays as a guard.
    let a = id(1);
    let mut b = body(&a, 1, NOW);
    b.neighbours = (0..64)
        .map(|n| Neighbour {
            key: key(n),
            cost: 0,
        })
        .collect();
    b.services = (0..16)
        .map(|n| format!("{n:02}{}", "x".repeat(62)))
        .collect();
    let bytes = encode_body(&b).unwrap();
    assert!(bytes.len() <= MAX_BODY);
    // A wire that is far above the limit is dropped without parsing.
    let big = vec![0u8; MAX_BODY + 64 + 1 + 32 * MAX_PATH + 1];
    assert_eq!(verify(&big, &me(), NOW), Err(Drop::BodyTooLarge));
}

#[test]
fn truncated_and_trailing_bytes() {
    let a = id(1);
    let w = wire(&a, 1, NOW);
    for n in 0..w.len() {
        assert!(verify(&w[..n], &me(), NOW).is_err(), "cut at {n}");
    }
    let mut t = w.clone();
    t.push(0);
    assert_eq!(verify(&t, &me(), NOW), Err(Drop::Trailing));
}

#[test]
fn unsorted_neighbours_are_not_canonical() {
    let a = id(1);
    let mut w = wire(&a, 1, NOW);
    // Swap the two neighbour entries (36 bytes each, from offset 59).
    let first: Vec<u8> = w[59..95].to_vec();
    let second: Vec<u8> = w[95..131].to_vec();
    w[59..95].copy_from_slice(&second);
    w[95..131].copy_from_slice(&first);
    assert_eq!(verify(&w, &me(), NOW), Err(Drop::NotCanonical));
}

#[test]
fn bad_version() {
    let a = id(1);
    let mut w = wire(&a, 1, NOW);
    w[0] = 2;
    assert_eq!(verify(&w, &me(), NOW), Err(Drop::BadVersion));
}

#[test]
fn seen_seq_is_dropped() {
    let a = id(1);
    let mut r = Receiver::new(me());
    r.accept(&wire(&a, 5, NOW), NOW).unwrap();
    assert_eq!(r.accept(&wire(&a, 5, NOW + 1), NOW + 1), Err(Drop::SeenSeq));
    assert_eq!(r.accept(&wire(&a, 4, NOW + 1), NOW + 1), Err(Drop::SeenSeq));
    assert!(r.accept(&wire(&a, 6, NOW + 2), NOW + 2).is_ok());
    assert_eq!(r.seq_of(&a.public()), Some(6));
}

#[test]
fn dropped_announcement_does_not_move_seq() {
    let a = id(1);
    let mut r = Receiver::new(me());
    r.accept(&wire(&a, 5, NOW), NOW).unwrap();
    let mut w = wire(&a, 9, NOW);
    w[60] ^= 1;
    assert!(r.accept(&w, NOW).is_err());
    assert_eq!(r.seq_of(&a.public()), Some(5));
}

#[test]
fn seen_table_is_bounded_and_purges_old_entries() {
    const CAP: usize = 16;
    let mut r = Receiver::with_capacity(me(), CAP);
    // CAP origins with distinct times of issue, then one more.
    for n in 0..CAP as u32 {
        let who = Identity::from_seed(&{
            let mut s = [7u8; 32];
            s[..4].copy_from_slice(&n.to_be_bytes());
            s
        });
        r.accept(&wire(&who, 1, NOW + n as u64), NOW + n as u64)
            .unwrap();
    }
    assert_eq!(r.len(), CAP);
    let extra = id(200);
    let t = NOW + CAP as u64;
    r.accept(&wire(&extra, 1, t), t).unwrap();
    assert_eq!(r.len(), CAP);
    assert_eq!(r.seq_of(&extra.public()), Some(1));
    // The oldest entry (n = 0) went out.
    assert_eq!(Receiver::new(me()).len(), 0);
    assert_eq!(MAX_SEEN, 4096);
    // A late announcement purges the entries whose skew window ended.
    let late = t + SKEW_MS + CAP as u64 + 10;
    r.accept(&wire(&id(201), 1, late), late).unwrap();
    assert_eq!(r.len(), 1);
}

#[test]
fn direct_announcement_is_a_neighbour_claim_multi_hop_is_not() {
    let a = id(1);
    let direct = verify(&wire(&a, 1, NOW), &me(), NOW).unwrap();
    assert_eq!(direct.neighbour_claims().unwrap().len(), 2);
    let w = encode(&a, &body(&a, 1, NOW), &[a.public(), key(5)]).unwrap();
    let multi = verify(&w, &me(), NOW).unwrap();
    assert!(multi.neighbour_claims().is_none());
}

#[test]
fn append_hop_keeps_the_signature() {
    let a = id(1);
    let w = wire(&a, 1, NOW);
    let w2 = append_hop(&w, &key(5)).unwrap();
    let v = verify(&w2, &me(), NOW).unwrap();
    assert_eq!(v.path, vec![a.public(), key(5)]);
    // The sixteenth node is the last that fits.
    let mut cur = w;
    for n in 0..15 {
        cur = append_hop(&cur, &key(10 + n)).unwrap();
    }
    assert_eq!(append_hop(&cur, &key(99)), Err(Drop::BadPathLength));
}

#[test]
fn verified_announcement_feeds_the_route_table() {
    let a = id(1);
    let m = me();
    let mut rx = Receiver::new(m);
    let mut table = RouteTable::new(m, Limits::default());
    let v = rx.accept(&wire(&a, 3, NOW), NOW).unwrap();
    table.learn(a.public(), 5, &v.route(0), NOW).unwrap();
    let best = table.best(&a.public(), NOW).unwrap();
    assert_eq!(best.next_hop, a.public());
    assert_eq!(best.cost, 5);
    assert_eq!(best.seq, 3);
    // A replay is dropped by the receiver before it reaches the table.
    assert_eq!(rx.accept(&wire(&a, 3, NOW), NOW), Err(Drop::SeenSeq));
    // A multi-hop announcement through the neighbour b.
    let b = id(2);
    let w = encode(&a, &body(&a, 4, NOW), &[a.public(), b.public()]).unwrap();
    let v = rx.accept(&w, NOW).unwrap();
    table.learn(b.public(), 4, &v.route(2), NOW).unwrap();
    assert_eq!(table.ranked(&a.public(), NOW).len(), 2);
}
