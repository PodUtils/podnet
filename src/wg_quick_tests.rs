//! The tests of the wg-quick reader (crates/podnet-vnet, T-116) with the
//! WireGuard code of the WARP adapter. They stay in the root crate: the
//! crate of the virtual network cannot use the WARP adapter, and the test
//! responder of `src/warp/wg/standin.rs` is a test module of this crate.

use base64::Engine as _;

use crate::vnet::wg_quick;
use crate::warp::wg::{self, standin};

/// A wg-quick file for the client of `keys`, with the peer at `endpoint`.
fn file_for(keys: &wg::Keys, endpoint: std::net::SocketAddr) -> String {
    let b64 = base64::engine::general_purpose::STANDARD;
    format!(
        "[Interface]\nPrivateKey = {}\nAddress = 172.16.0.2/32\n\n[Peer]\nPublicKey = {}\nEndpoint = {endpoint}\nAllowedIPs = 0.0.0.0/0\n",
        b64.encode(keys.private),
        b64.encode(keys.peer_public),
    )
}

/// The peer of the WireGuard code for the one peer of `conf`.
fn peer_of(conf: &wg_quick::WgQuick) -> wg::Peer {
    let peer = conf.single_peer().unwrap();
    let v4 = conf.interface.addresses.iter().find_map(|c| match c.addr {
        std::net::IpAddr::V4(v4) => Some(v4),
        std::net::IpAddr::V6(_) => None,
    });
    wg::Peer {
        keys: wg::Keys {
            private: conf.interface.private_key.0,
            peer_public: peer.public_key,
            reserved: peer.reserved,
        },
        endpoint: peer.endpoint.as_ref().unwrap().socket_addr().unwrap(),
        v4,
        v6: None,
    }
}

#[tokio::test]
async fn wg_handshake_with_the_test_responder() {
    crate::test_host::need_loopback!();
    // Reserved bytes that are not zero: the file has none, so the client
    // sends zeros, and the stand-in answers each handshake.
    let (keys, peer_private) = standin::keys([7, 7, 7]);
    let sock = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let endpoint = sock.local_addr().unwrap();
    tokio::spawn(standin::serve(
        sock,
        peer_private,
        keys.public(),
        standin::DataPath::All,
    ));

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wg0.conf");
    std::fs::write(&path, file_for(&keys, endpoint)).unwrap();
    let conf = wg_quick::read_file(&path).unwrap();
    let peer = peer_of(&conf);
    assert_eq!(peer.keys.reserved, [0, 0, 0]);
    assert_eq!(peer.keys.private, keys.private);
    assert_eq!(peer.v4, Some("172.16.0.2".parse().unwrap()));

    let result = tokio::time::timeout(std::time::Duration::from_secs(20), wg::connect(&peer))
        .await
        .expect("the handshake ends in 20 s");
    assert!(result.is_ok(), "{:#}", result.err().unwrap());
}
