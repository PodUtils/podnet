//! A stand-in of the WireGuard front door of WARP, for the tests (T-074):
//! the responder of pod-wg (PodUtils/podcrates, its feature `standin`). It
//! answers each handshake. It takes the data messages as [`DataPath`] says:
//! a TCP SYN gets a SYN-ACK, and each other packet comes back. This module
//! gives its client the `Peer` of `pod-warp`.

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

/// A tunnel over a UDP socket of the host carries packets both ways.
#[tokio::test]
async fn a_tunnel_over_udp_carries_packets_both_ways() {
    crate::test_host::need_loopback!();
    let peer = start([7, 8, 9], DataPath::NeedsReserved).await;
    let (mut tx, mut rx, _keepalive) = connect(&peer).await.unwrap();
    let mut packet = vec![0x45, 0, 0, 40];
    packet.resize(40, 3);
    tx.send_packets(&[packet.clone()]).await.unwrap();
    let back = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv_packet())
        .await
        .expect("a packet back")
        .unwrap()
        .unwrap();
    assert_eq!(back.to_vec(), packet);
}
