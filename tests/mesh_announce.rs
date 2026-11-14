//! The signed route announcement (T-285): the encoding, the signature, each
//! drop rule, the skew both ways, each bound, a tampered byte and the feed of
//! a verified announcement to the route table. Version 2 (T-346): each hop
//! names its next node, so a node cannot cut the hops before its own.

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

fn me() -> PublicKey {
    id(99).public()
}

/// The announcement of `who` for this node.
fn wire(who: &Identity, seq: u64, issued: u64) -> Vec<u8> {
    wire_to(who, seq, issued, &me())
}

/// The announcement of `who` for the neighbour `to`.
fn wire_to(who: &Identity, seq: u64, issued: u64, to: &PublicKey) -> Vec<u8> {
    encode(who, &body(who, seq, issued), to).unwrap()
}

/// The bytes of one hop: the key, the cost, the next key and the signature.
const HOP: usize = 32 + 4 + 32 + 64;

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
    assert_eq!(encode(&a, &b, &me()).unwrap(), w);
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
    // Version 2 signs each byte of the path too: the hop of the origin
    // covers its key, its cost and its next key.
    let a = id(1);
    for w in [wire(&a, 5, NOW), chain3()] {
        for i in 0..w.len() {
            let mut t = w.clone();
            t[i] ^= 0x01;
            assert!(
                verify(&t, &me(), NOW).is_err(),
                "byte {i} changed and was accepted"
            );
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
    // `a` signs a body that names `b` as origin. The hop of the origin has
    // the key of `a`.
    let w = encode(&a, &b, &me()).unwrap();
    assert_eq!(verify(&w, &me(), NOW), Err(Drop::PathNotFromOrigin));
    // With the hop key of `b`, the signature of the body fails.
    let mut t = w.clone();
    let at = t.len() - HOP;
    t[at..at + 32].copy_from_slice(&b.origin.0);
    assert_eq!(verify(&t, &me(), NOW), Err(Drop::BadSignature));
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
    let w = encode(&a, &b, &me()).unwrap();
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
    let w = encode(&a, &b, &me()).unwrap();
    assert_eq!(verify(&w, &me(), NOW), Err(Drop::BadLifetime));
    b.expires_ms = NOW + LIFETIME_MS + 1;
    let w = encode(&a, &b, &me()).unwrap();
    assert_eq!(verify(&w, &me(), NOW), Err(Drop::BadLifetime));
}

#[test]
fn path_with_self_is_dropped() {
    let a = id(1);
    let m = id(99);
    let w = append_hop(&wire(&a, 1, NOW), &m, 4, &key(7)).unwrap();
    assert_eq!(verify(&w, &m.public(), NOW), Err(Drop::PathHasSelf));
}

#[test]
fn path_must_start_at_origin_and_not_repeat() {
    let a = id(1);
    // The encoder refuses a bad path, so build the wire by hand.
    let good = wire(&a, 1, NOW);
    let body_end = good.len() - 1 - HOP;
    let mut w = good[..body_end].to_vec();
    w.push(1);
    w.extend_from_slice(&key(5).0);
    w.extend_from_slice(&[0u8; HOP - 32]);
    assert_eq!(verify(&w, &me(), NOW), Err(Drop::PathNotFromOrigin));
    let mut w = good[..body_end].to_vec();
    w.push(3);
    for k in [a.public(), key(5), key(5)] {
        w.extend_from_slice(&k.0);
        w.extend_from_slice(&[0u8; HOP - 32]);
    }
    assert_eq!(verify(&w, &me(), NOW), Err(Drop::PathRepeats));
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
    let mut w = wire_to(&a, 1, NOW, &id(10).public());
    for n in 10..25 {
        let to = if n < 24 { id(n + 1).public() } else { me() };
        w = append_hop(&w, &id(n), 1, &to).unwrap();
    }
    // 16 nodes: the origin and 15 forwarders.
    assert!(verify(&w, &me(), NOW).is_ok());
    assert!(w.len() <= MAX_WIRE);
    assert_eq!(append_hop(&w, &id(100), 1, &me()), Err(Drop::BadPathLength));
    // A wire with 17 nodes is dropped.
    let good = wire(&a, 1, NOW);
    let mut w = good[..good.len() - 1 - HOP].to_vec();
    w.push(17);
    w.extend_from_slice(&a.public().0);
    w.extend_from_slice(&[0u8; 17 * HOP - 32]);
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
    let w = encode(&a, &b, &me()).unwrap();
    assert_eq!(verify(&w, &me(), NOW).unwrap().body.neighbours.len(), 64);
    b.neighbours.push(Neighbour {
        key: key(64),
        cost: 0,
    });
    assert_eq!(encode(&a, &b, &me()), Err(EncodeError::TooManyNeighbours));
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
    let w = encode(&a, &b, &me()).unwrap();
    assert_eq!(verify(&w, &me(), NOW).unwrap().body.services.len(), 16);
    b.services.push("s16".into());
    assert_eq!(encode(&a, &b, &me()), Err(EncodeError::TooManyServices));
    b.services = vec!["x".repeat(64)];
    assert!(encode(&a, &b, &me()).is_ok());
    b.services = vec!["x".repeat(65)];
    assert_eq!(encode(&a, &b, &me()), Err(EncodeError::BadServiceName));
    b.services = vec![String::new()];
    assert_eq!(encode(&a, &b, &me()), Err(EncodeError::BadServiceName));
    b.services = vec!["a\nb".into()];
    assert_eq!(encode(&a, &b, &me()), Err(EncodeError::BadServiceName));
    b.services = vec!["a".into(), "a".into()];
    assert_eq!(encode(&a, &b, &me()), Err(EncodeError::Repeat));
}

#[test]
fn service_count_on_the_wire_is_bounded() {
    let a = id(1);
    let mut b = body(&a, 1, NOW);
    b.neighbours.clear();
    b.services.clear();
    let mut w = encode(&a, &b, &me()).unwrap();
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
    let big = vec![0u8; MAX_WIRE + 1];
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
    w[0] = 3;
    assert_eq!(verify(&w, &me(), NOW), Err(Drop::BadVersion(3)));
    assert!(Drop::BadVersion(3).to_string().contains("version 3"));
}

/// An announcement of version 1, signed as a node of version 1 signs it: the
/// body with the version 1, the label of version 1 and the path `[origin]`.
fn version_1(who: &Identity, seq: u64, issued: u64) -> Vec<u8> {
    let mut body = encode_body(&body(who, seq, issued)).unwrap();
    body[0] = 1;
    let signature = who.sign(&[&b"podnet-mesh-announce-v1\0"[..], &body].concat());
    let mut out = body;
    out.extend_from_slice(&signature);
    out.push(1);
    out.extend_from_slice(&who.public().0);
    out
}

/// A replay of a valid announcement of version 1 is refused, and the reason
/// names the version (T-346).
#[test]
fn version_1_is_refused_and_named() {
    let a = id(1);
    let old = version_1(&a, 1, NOW);
    assert_eq!(verify(&old, &me(), NOW), Err(Drop::BadVersion(1)));
    let mut r = Receiver::new(me());
    assert_eq!(r.accept(&old, NOW), Err(Drop::BadVersion(1)));
    assert_eq!(r.seq_of(&a.public()), None);
    let text = Drop::BadVersion(1).to_string();
    assert!(
        text.contains("version 1") && text.contains("version 2"),
        "{text}"
    );
}

/// The attack of T-346: X gets the path `[O, A]` from A, removes the hop of
/// A, and sends `[O, X]` to Y. The hop of O names A, not X: Y refuses it. X
/// cannot sign a hop of O that names X.
#[test]
fn a_neighbour_cannot_strip_hops_and_claim_the_origin() {
    let (o, a, x) = (id(1), id(5), id(6));
    let y = me();
    let for_a = wire_to(&o, 1, NOW, &a.public());
    let for_x = append_hop(&for_a, &a, 4, &x.public()).unwrap();
    // The honest path through X is good.
    let honest = append_hop(&for_x, &x, 9, &y).unwrap();
    assert_eq!(verify(&honest, &y, NOW).unwrap().path.len(), 3);
    // X strips the hop of A and adds its own hop over the hop of O.
    assert_eq!(
        append_hop(&for_a, &x, 0, &y),
        Err(Drop::NotForMe),
        "the hop of O names A"
    );
    let at = for_a.len() - HOP - 1;
    assert_eq!(for_a[at], 1);
    let origin_sig: [u8; 64] = for_a[for_a.len() - 64..].try_into().unwrap();
    let mut forged = for_a.clone();
    forged[at] = 2;
    // The hop of X, signed by X over the hop of O, for Y.
    let mut hop_bytes = b"podnet-mesh-hop-v2\0".to_vec();
    hop_bytes.extend_from_slice(&origin_sig);
    hop_bytes.extend_from_slice(&x.public().0);
    hop_bytes.extend_from_slice(&0u32.to_be_bytes());
    hop_bytes.extend_from_slice(&y.0);
    forged.extend_from_slice(&x.public().0);
    forged.extend_from_slice(&0u32.to_be_bytes());
    forged.extend_from_slice(&y.0);
    forged.extend_from_slice(&x.sign(&hop_bytes));
    assert_eq!(verify(&forged, &y, NOW), Err(Drop::BadNextHop));
    // X also writes its own key as the next node of O: the signature of O
    // does not cover it.
    let next_at = at + 1 + 32 + 4;
    forged[next_at..next_at + 32].copy_from_slice(&x.public().0);
    assert_eq!(verify(&forged, &y, NOW), Err(Drop::BadHopSignature));
}

/// An announcement for another node is refused: a neighbour cannot send on
/// a copy that it saw for another node.
#[test]
fn an_announcement_for_another_node_is_refused() {
    let a = id(1);
    let w = wire_to(&a, 1, NOW, &key(7));
    assert_eq!(verify(&w, &me(), NOW), Err(Drop::NotForMe));
    assert!(verify(&w, &key(7), NOW).is_ok());
    // The hop of the origin has no cost.
    let mut t = wire(&a, 1, NOW);
    let cost_at = t.len() - HOP + 32;
    t[cost_at + 3] = 1;
    assert_eq!(verify(&t, &me(), NOW), Err(Drop::BadNextHop));
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
    let w = append_hop(&wire_to(&a, 1, NOW, &id(5).public()), &id(5), 3, &me()).unwrap();
    let multi = verify(&w, &me(), NOW).unwrap();
    assert!(multi.neighbour_claims().is_none());
}

#[test]
fn append_hop_signs_the_hop_and_the_chain_verifies() {
    let (a, b, c) = (id(1), id(5), id(6));
    let w = wire_to(&a, 1, NOW, &b.public());
    let w2 = append_hop(&w, &b, 4, &me()).unwrap();
    let v = verify(&w2, &me(), NOW).unwrap();
    assert_eq!(v.path, vec![a.public(), b.public()]);
    assert_eq!(v.costs, vec![0, 4]);
    assert_eq!(v.path_cost(), 4);
    // A forwarder that the last hop does not name cannot add its hop.
    assert_eq!(append_hop(&w2, &c, 9, &me()), Err(Drop::NotForMe));
    let w2c = append_hop(&w, &b, 4, &c.public()).unwrap();
    assert_eq!(verify(&w2c, &me(), NOW), Err(Drop::NotForMe));
    let w3 = append_hop(&w2c, &c, 9, &me()).unwrap();
    let v = verify(&w3, &me(), NOW).unwrap();
    assert_eq!(v.path, vec![a.public(), b.public(), c.public()]);
    assert_eq!(v.path_cost(), 13);
    // A forwarder that is in the path cannot add itself again.
    assert_eq!(append_hop(&w3, &b, 1, &me()), Err(Drop::PathRepeats));
}

/// The offset of the hop `i` (0 is the origin) of a wire that has `n`
/// nodes.
fn hop_at(wire: &[u8], i: usize, n: usize) -> usize {
    wire.len() - (n - i) * HOP
}

/// The path `[a, b, c]` for this node: a valid path of 3 hops.
fn chain3() -> Vec<u8> {
    let w = wire_to(&id(1), 1, NOW, &id(5).public());
    let w = append_hop(&w, &id(5), 4, &id(6).public()).unwrap();
    append_hop(&w, &id(6), 9, &me()).unwrap()
}

#[test]
fn a_valid_path_of_3_hops_is_taken() {
    let v = verify(&chain3(), &me(), NOW).unwrap();
    assert_eq!(v.path, vec![id(1).public(), id(5).public(), id(6).public()]);
    assert_eq!(v.costs, vec![0, 4, 9]);
    let mut r = Receiver::new(me());
    assert!(r.accept(&chain3(), NOW).is_ok());
}

#[test]
fn tampered_hop_is_dropped() {
    let w = chain3();
    for i in 0..3 {
        let at = hop_at(&w, i, 3);
        // The key, the cost, the next key and the signature of the hop.
        for off in [0, 31, 32, 35, 36, 67, 68, HOP - 1] {
            let mut t = w.clone();
            t[at + off] ^= 0x01;
            assert!(
                matches!(
                    verify(&t, &me(), NOW),
                    Err(Drop::BadHopSignature
                        | Drop::PathNotFromOrigin
                        | Drop::BadNextHop
                        | Drop::NotForMe)
                ),
                "hop {i} offset {off}"
            );
        }
    }
}

#[test]
fn dropped_hop_is_refused() {
    // Remove the middle hop (b) of the path [a, b, c]: the hop of a names
    // b, and c signed over b.
    let w = chain3();
    let at = hop_at(&w, 1, 3);
    let mut t = w[..at].to_vec();
    t.extend_from_slice(&w[at + HOP..]);
    let len_at = at - HOP - 1;
    assert_eq!(t[len_at], 3);
    t[len_at] = 2;
    assert_eq!(verify(&t, &me(), NOW), Err(Drop::BadNextHop));
    // The same wire with the hop left in is good.
    assert!(verify(&w, &me(), NOW).is_ok());
}

#[test]
fn lowered_cost_is_a_bad_hop_signature() {
    let w = chain3();
    for i in 1..3 {
        let mut t = w.clone();
        let at = hop_at(&t, i, 3) + 32;
        assert!(t[at..at + 4] != [0, 0, 0, 1]);
        t[at..at + 4].copy_from_slice(&1u32.to_be_bytes());
        assert_eq!(
            verify(&t, &me(), NOW),
            Err(Drop::BadHopSignature),
            "hop {i}"
        );
    }
}

#[test]
fn hop_signature_of_another_key_is_dropped() {
    // The forwarder b names the key of c: b signed, c did not. The hop of
    // the origin names b, so the path does not agree.
    let w = append_hop(&wire_to(&id(1), 1, NOW, &id(5).public()), &id(5), 4, &me()).unwrap();
    let mut t = w.clone();
    let at = hop_at(&t, 1, 2);
    t[at..at + 32].copy_from_slice(&id(6).public().0);
    assert_eq!(verify(&t, &me(), NOW), Err(Drop::BadNextHop));
    // The next key of the origin changed too: the origin did not sign it.
    let next_at = hop_at(&t, 0, 2) + 36;
    t[next_at..next_at + 32].copy_from_slice(&id(6).public().0);
    assert_eq!(verify(&t, &me(), NOW), Err(Drop::BadHopSignature));
}

#[test]
fn verified_announcement_feeds_the_route_table() {
    let a = id(1);
    let m = me();
    let mut rx = Receiver::new(m);
    let mut table = RouteTable::new(m, Limits::default());
    let v = rx.accept(&wire(&a, 3, NOW), NOW).unwrap();
    table.learn(a.public(), 5, &v.route(), NOW).unwrap();
    let best = table.best(&a.public(), NOW).unwrap();
    assert_eq!(best.next_hop, a.public());
    assert_eq!(best.cost, 5);
    assert_eq!(best.seq, 3);
    // A replay is dropped by the receiver before it reaches the table.
    assert_eq!(rx.accept(&wire(&a, 3, NOW), NOW), Err(Drop::SeenSeq));
    // A multi-hop announcement through the neighbour b.
    let b = id(2);
    let w = append_hop(&wire_to(&a, 4, NOW, &b.public()), &b, 2, &me()).unwrap();
    let v = rx.accept(&w, NOW).unwrap();
    table.learn(b.public(), 4, &v.route(), NOW).unwrap();
    assert_eq!(
        table.ranked(&a.public(), NOW).iter().map(|r| r.cost).max(),
        Some(6)
    );
    assert_eq!(table.ranked(&a.public(), NOW).len(), 2);
}
