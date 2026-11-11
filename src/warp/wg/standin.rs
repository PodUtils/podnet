//! A stand-in of the WireGuard front door of WARP, for the tests (T-074):
//! the responder of pod-wg (PodUtils/podcrates, its feature `standin`). It
//! answers each handshake. It takes the data messages as [`DataPath`] says:
//! a TCP SYN gets a SYN-ACK, and each other packet comes back. This module
//! gives its client the `Peer` of this crate, and keeps the tests of the
//! driver.

use super::*;
pub use pod_wg::standin::{keys, serve, session, DataPath};

/// Start the stand-in on the loopback for a new client. It gives the peer of
/// the client (the device address 172.16.0.2).
pub async fn start(reserved: [u8; 3], path: DataPath) -> Peer {
    let (keys, endpoint) = pod_wg::standin::start(reserved, path).await;
    Peer {
        keys,
        endpoint,
        v4: Some("172.16.0.2".parse().unwrap()),
        v6: None,
    }
}

/// A tunnel over the stand-in carries packets both ways.
#[tokio::test]
async fn a_tunnel_over_udp_carries_packets_both_ways() {
    crate::test_host::need_loopback!();
    let peer = start([7, 8, 9], DataPath::NeedsReserved).await;
    let (mut tx, mut rx, _keepalive) = connect(&peer).await.unwrap();
    let mut packet = vec![0x45, 0, 0, 40];
    packet.resize(40, 3);
    tx.send_packets(&[packet.clone()]).await.unwrap();
    let back = tokio::time::timeout(Duration::from_secs(5), rx.recv_packet())
        .await
        .expect("a packet back")
        .unwrap()
        .unwrap();
    assert_eq!(back.to_vec(), packet);
}

/// A front door under load answers the first try with a cookie reply; the
/// next try carries the mac2 of the cookie, and the handshake ends. The
/// next try waits for its time: the test takes about REKEY_TIMEOUT.
#[tokio::test]
async fn a_front_door_under_load_takes_the_mac2_of_its_cookie() {
    crate::test_host::need_loopback!();
    let (keys, endpoint) = pod_wg::standin::start_under_load([0; 3], DataPath::All).await;
    let peer = Peer {
        keys,
        endpoint,
        v4: None,
        v6: None,
    };
    let (mut tx, mut rx, _keepalive) = connect(&peer).await.unwrap();
    let mut packet = vec![0x45, 0, 0, 40];
    packet.resize(40, 3);
    tx.send_packets(&[packet.clone()]).await.unwrap();
    let back = tokio::time::timeout(Duration::from_secs(5), rx.recv_packet())
        .await
        .expect("a packet back")
        .unwrap()
        .unwrap();
    assert_eq!(back.to_vec(), packet);
}

/// T-074: the WireGuard leg of `--transport wireguard` has fresh
/// handshakes and no data path. `WgKeepalive` sees no fault; the TCP probe
/// of T-068 on the stack of the leg renews the session.
#[tokio::test]
async fn wireguard_probe_renews_a_dead_leg() {
    use crate::warp::ip::{StackConfig, WarpStack};
    use crate::warp::live::{Connect, Live, Probe};
    crate::test_host::need_loopback!();
    let peer = start([1, 2, 3], DataPath::Dead).await;
    let open = |peer: Peer| async move {
        let session = crate::warp::session::MasqueSession::connect_wg(&peer).await?;
        let (tx, rx) = session.split();
        WarpStack::start(tx, rx, StackConfig::new("172.16.0.2".parse().unwrap()))
    };
    let first = open(peer.clone()).await.unwrap();
    let opens = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let connect: Connect<WarpStack> = {
        let opens = opens.clone();
        Box::new(move || {
            opens.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Box::pin(open(peer.clone()))
        })
    };
    let probe = Probe {
        target: "1.1.1.1:443".parse().unwrap(),
        every: Duration::from_millis(200),
        limit: Duration::from_millis(300),
        misses: 3,
    };
    let live = Live::start_with(
        first,
        connect,
        Duration::from_secs(1),
        Duration::from_secs(1),
    )
    .probed(Some(probe));
    let old = live.get().await.unwrap();
    let started = Instant::now();
    while live.reconnects() == 0 {
        assert!(started.elapsed() < Duration::from_secs(10), "no renewal");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    // The handshakes are fresh: the old leg is open, and drains.
    assert!(!old.is_closed(), "{:?}", old.close_reason());
    assert!(opens.load(std::sync::atomic::Ordering::SeqCst) >= 1);
}
