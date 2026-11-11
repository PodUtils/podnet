use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use super::super::balance::{Balance, Member, Selection};
use super::super::fakes::{log, soon, Fake};
use super::super::{Dial, Policy, Target};

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

fn target() -> Target {
    Target::new("svc.example", 443)
}

/// T-178: the bytes of a flow count for its member in both directions,
/// while the flow runs.
#[tokio::test]
async fn group_meter_counts_bytes_in_both_directions_as_the_flow_runs() {
    let b = group(Policy::Weighted, &["a"]);
    let mut conn = b.dial(&target(), soon()).await.unwrap();
    assert_eq!(b.members()[0].bytes(), 0);
    conn.io.write_all(&[7u8; 1000]).await.unwrap();
    assert_eq!(b.members()[0].bytes(), 1000, "the send counts at once");
    let mut label = [0u8; 1];
    conn.io.read_exact(&mut label).await.unwrap();
    assert_eq!(b.members()[0].bytes(), 1001, "the receive counts at once");
    assert_eq!(b.status()["members"][0]["bytes"], 1001);
}

/// T-178: an open dial counts until the first byte, in either direction,
/// or until the flow drops.
#[tokio::test]
async fn group_meter_open_ends_at_the_first_byte() {
    let b = group(Policy::LeastLoad, &["a"]);
    let mut conn = b.dial(&target(), soon()).await.unwrap();
    assert_eq!(b.members()[0].opens(), 1, "no byte yet");
    let mut label = [0u8; 1];
    conn.io.read_exact(&mut label).await.unwrap();
    assert_eq!(b.members()[0].opens(), 0, "the first byte ends the open");
    drop(conn);
    assert_eq!(b.members()[0].opens(), 0);

    let conn = b.dial(&target(), soon()).await.unwrap();
    assert_eq!(b.members()[0].opens(), 1);
    drop(conn);
    assert_eq!(b.members()[0].opens(), 0, "a flow dropped before a byte");

    let mut conn = b.dial(&target(), soon()).await.unwrap();
    conn.io.write_all(b"x").await.unwrap();
    assert_eq!(b.members()[0].opens(), 0, "a sent byte ends the open too");
}

/// T-178: `leastload` reads the bytes that the flows carried: the member
/// of a bulk flow gets no new flow while the other member is idle.
#[tokio::test]
async fn group_meter_feeds_leastload_with_the_bytes_of_the_flows() {
    let b = group(Policy::LeastLoad, &["a", "b"]);
    let mut conn = b.dial(&target(), soon()).await.unwrap();
    let mut label = [0u8; 1];
    conn.io.read_exact(&mut label).await.unwrap();
    let busy = String::from_utf8(label.to_vec()).unwrap();
    for _ in 0..10 {
        conn.io.write_all(&[0u8; 10_000]).await.unwrap();
    }
    for _ in 0..10 {
        match b.pick() {
            Selection::One(m) => assert_ne!(m.name, busy, "the busy member is picked"),
            Selection::None => panic!("members are up"),
        }
    }
}
