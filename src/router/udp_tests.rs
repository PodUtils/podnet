use std::net::SocketAddr;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use tokio::net::UdpSocket;

use super::*;

const FILE: &str = "networks:\n  home: direct\n  proxy: socks5h://127.0.0.1:9\ngroups:\n  egress: home*1\n  viaproxy: proxy*1\ndoors:\n  - listen: 127.0.0.2:41080\n    via: egress\n  - listen: unix:/tmp/r-udp.sock\n    via: viaproxy\n";

/// The reply of a UDP ASSOCIATE on `door`, and the control stream.
async fn associate(door: &Arc<podnet_vnet::door::Door>) -> (tokio::io::DuplexStream, Vec<u8>) {
    let (mut client, server) = tokio::io::duplex(4096);
    door.clone().spawn(server);
    client.write_all(&[5, 1, 0]).await.unwrap();
    let mut method = [0u8; 2];
    client.read_exact(&mut method).await.unwrap();
    client
        .write_all(&[5, 3, 0, 1, 0, 0, 0, 0, 0, 0])
        .await
        .unwrap();
    let mut head = [0u8; 10];
    client.read_exact(&mut head).await.unwrap();
    (client, head.to_vec())
}

/// T-098 item 1: a door of the router carries UDP ASSOCIATE over its group
/// (the datagrams of the pick), and the bytes count for the member. A door
/// over a proxy network, which carries no UDP, answers "command not supported".
#[tokio::test]
async fn route_door_udp_associate_reaches_the_router() {
    crate::test_host::need_loopback!();
    let echo = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let server = echo.local_addr().unwrap();
    tokio::spawn(async move {
        let mut buf = [0u8; 512];
        while let Ok((n, from)) = echo.recv_from(&mut buf).await {
            let _ = echo.send_to(&buf[..n], from).await;
        }
    });
    let router = Router::up(&parse(FILE).unwrap()).await.unwrap();
    let (_control, reply) = associate(&router.doors[0].door).await;
    assert_eq!(&reply[..4], &[5, 0, 0, 1], "{reply:?}");
    let bound = SocketAddr::from(([127, 0, 0, 2], u16::from_be_bytes([reply[8], reply[9]])));
    assert_eq!(&reply[4..8], &[127, 0, 0, 2], "the address of the door");
    let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let mut datagram = vec![0, 0, 0, 1, 127, 0, 0, 1];
    datagram.extend_from_slice(&server.port().to_be_bytes());
    datagram.extend_from_slice(b"ping");
    client.send_to(&datagram, bound).await.unwrap();
    let mut buf = [0u8; 512];
    let (n, _) = tokio::time::timeout(Duration::from_secs(5), client.recv_from(&mut buf))
        .await
        .expect("an answer")
        .unwrap();
    assert_eq!(&buf[n - 4..n], b"ping");
    assert_eq!(
        router.status()["groups"][0]["members"][0]["bytes"],
        8,
        "4 bytes out and 4 bytes back count for the member"
    );
    // The proxy network carries no UDP: the door says so.
    let (_control, reply) = associate(&router.doors[1].door).await;
    assert_eq!(reply[1], 0x07, "{reply:?}");
}
