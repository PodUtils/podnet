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

/// T-097: the datagrams of a socket count for the member that carries them,
/// in both directions, as the bytes of a TCP flow do.
#[tokio::test]
async fn group_meter_counts_udp_bytes_in_both_directions() {
    use super::super::fakes::{udp_way, Pipe};
    use super::super::{recv_from, send_to};
    let pipe = std::sync::Arc::new(Pipe::default());
    let b = Balance::new(
        "g",
        vec![Member::new("a", udp_way("a", Some(pipe.clone())), 1)],
    )
    .unwrap();
    let socket = b.datagrams().await.unwrap();
    let to = std::net::SocketAddr::from(([192, 0, 2, 9], 53));
    send_to(&*socket, &[1u8; 300], to).await.unwrap();
    assert_eq!(b.members()[0].bytes(), 300, "the send counts");
    pipe.inbox.lock().unwrap().push_back(vec![2u8; 120]);
    let mut buf = [0u8; 256];
    let (n, _) = recv_from(&*socket, &mut buf).await.unwrap();
    assert_eq!(n, 120);
    assert_eq!(b.members()[0].bytes(), 420, "the receive counts");
    assert_eq!(b.status()["members"][0]["bytes"], 420);
}

/// T-097 with T-179: with a split, `up` counts the bytes that it sends and
/// `down` counts the bytes that it receives.
#[tokio::test]
async fn group_meter_counts_split_udp_bytes_for_each_member() {
    use super::super::fakes::{udp_way, Pipe};
    use super::super::{recv_from, send_to};
    let (pa, pb) = (
        std::sync::Arc::new(Pipe::default()),
        std::sync::Arc::new(Pipe::default()),
    );
    let b = Balance::new(
        "g",
        vec![
            Member::new("a", udp_way("a", Some(pa.clone())), 1),
            Member::new("b", udp_way("b", Some(pb.clone())), 1),
        ],
    )
    .unwrap()
    .with_split("a", "b")
    .unwrap();
    let socket = b.datagrams().await.unwrap();
    let to = std::net::SocketAddr::from(([192, 0, 2, 9], 53));
    send_to(&*socket, &[1u8; 50], to).await.unwrap();
    pb.inbox.lock().unwrap().push_back(vec![2u8; 700]);
    let mut buf = [0u8; 1024];
    recv_from(&*socket, &mut buf).await.unwrap();
    assert_eq!(b.members()[0].bytes(), 50, "up counts what it sent");
    assert_eq!(b.members()[1].bytes(), 700, "down counts what it received");
}
