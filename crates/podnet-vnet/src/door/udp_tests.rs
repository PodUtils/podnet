use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use podnet_core::network::{
    BoxFuture, Class, Connected, Datagrams, Dial, Failure, Stage, SystemDatagrams, Target,
};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::UdpSocket;
use tokio::time::Instant;

use super::super::Door;

/// A dialer with datagrams over a socket of this host (no TCP).
struct Udp {
    refuse: Option<Class>,
    /// The address of the socket that `datagrams` made last.
    local: Mutex<Option<SocketAddr>>,
}

impl Udp {
    fn new(refuse: Option<Class>) -> Arc<Udp> {
        Arc::new(Udp {
            refuse,
            local: Mutex::new(None),
        })
    }
}

impl Dial for Udp {
    fn label(&self) -> String {
        "udp".into()
    }

    fn dial<'a>(
        &'a self,
        _target: &'a Target,
        _deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(async move { Err(Failure::new(Stage::Connect, Class::Usage, "no TCP")) })
    }

    fn datagrams<'a>(&'a self) -> BoxFuture<'a, Result<Arc<dyn Datagrams>, Failure>> {
        Box::pin(async move {
            if let Some(class) = self.refuse {
                return Err(Failure::new(Stage::Connect, class, "no datagrams"));
            }
            let socket = SystemDatagrams::bind("127.0.0.1:0".parse().unwrap())
                .await
                .map_err(|e| Failure::new(Stage::Connect, Class::Unavailable, e.to_string()))?;
            *self.local.lock().unwrap() = socket.socket().local_addr().ok();
            Ok(Arc::new(socket) as Arc<dyn Datagrams>)
        })
    }
}

/// A UDP echo server: it answers `re:` and the payload, and counts the
/// datagrams that it got.
async fn echo() -> (SocketAddr, Arc<AtomicUsize>) {
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = socket.local_addr().unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let seen = count.clone();
    tokio::spawn(async move {
        let mut buf = [0u8; 2048];
        while let Ok((n, from)) = socket.recv_from(&mut buf).await {
            seen.fetch_add(1, Ordering::Relaxed);
            let mut out = b"re:".to_vec();
            out.extend_from_slice(&buf[..n]);
            let _ = socket.send_to(&out, from).await;
        }
    });
    (addr, count)
}

/// The SOCKS5 header of a datagram to `to`.
fn wrap(to: SocketAddr, payload: &[u8]) -> Vec<u8> {
    let mut out = vec![0, 0, 0, 1];
    let SocketAddr::V4(v4) = to else {
        unreachable!("the tests use IPv4")
    };
    out.extend_from_slice(&v4.ip().octets());
    out.extend_from_slice(&to.port().to_be_bytes());
    out.extend_from_slice(payload);
    out
}

/// The greeting and the UDP ASSOCIATE request; the reply.
async fn associate<S>(client: &mut S) -> Vec<u8>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    client.write_all(&[5, 1, 0]).await.unwrap();
    let mut method = [0u8; 2];
    client.read_exact(&mut method).await.unwrap();
    client
        .write_all(&[5, 3, 0, 1, 0, 0, 0, 0, 0, 0])
        .await
        .unwrap();
    let mut reply = [0u8; 10];
    client.read_exact(&mut reply).await.unwrap();
    reply.to_vec()
}

fn bound_of(reply: &[u8]) -> SocketAddr {
    assert_eq!(&reply[..4], &[5, 0, 0, 1], "{reply:?}");
    SocketAddr::from((
        [reply[4], reply[5], reply[6], reply[7]],
        u16::from_be_bytes([reply[8], reply[9]]),
    ))
}

async fn answer(client: &UdpSocket, wait: Duration) -> Option<Vec<u8>> {
    let mut buf = [0u8; 2048];
    match tokio::time::timeout(wait, client.recv_from(&mut buf)).await {
        Ok(Ok((n, _))) => Some(buf[..n].to_vec()),
        _ => None,
    }
}

/// T-098: UDP ASSOCIATE over the datagrams of the dialer. The client sends
/// SOCKS5 datagrams to the BND address and gets the answers wrapped; a
/// stranger and a fragment are dropped; the end of the control stream ends
/// the association.
#[tokio::test]
async fn front_door_udp_associate_carries_datagrams() {
    pod_testkit::need_loopback!();
    let (server, count) = echo().await;
    let door = Arc::new(Door::new(Udp::new(None)));
    let (mut control, stream) = tokio::io::duplex(4096);
    door.clone().spawn(stream);
    let bound = bound_of(&associate(&mut control).await);
    assert!(bound.ip().is_loopback(), "{bound}");
    let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    client.send_to(&wrap(server, b"one"), bound).await.unwrap();
    let got = answer(&client, Duration::from_secs(5))
        .await
        .expect("an answer");
    assert_eq!(got, wrap(server, b"re:one"));
    // A fragment (FRAG not zero) is dropped.
    let mut fragment = wrap(server, b"frag");
    fragment[2] = 1;
    client.send_to(&fragment, bound).await.unwrap();
    // Another source than the client is not relayed.
    let stranger = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    stranger
        .send_to(&wrap(server, b"stranger"), bound)
        .await
        .unwrap();
    assert!(answer(&stranger, Duration::from_millis(300))
        .await
        .is_none());
    assert!(answer(&client, Duration::from_millis(100)).await.is_none());
    assert_eq!(count.load(Ordering::Relaxed), 1, "only the first datagram");
    // The client still works after the dropped ones.
    client.send_to(&wrap(server, b"two"), bound).await.unwrap();
    let got = answer(&client, Duration::from_secs(5))
        .await
        .expect("an answer");
    assert_eq!(got, wrap(server, b"re:two"));
    // The end of the control stream ends the association.
    drop(control);
    tokio::time::sleep(Duration::from_millis(200)).await;
    client
        .send_to(&wrap(server, b"three"), bound)
        .await
        .unwrap();
    assert!(answer(&client, Duration::from_millis(300)).await.is_none());
    assert_eq!(count.load(Ordering::Relaxed), 2);
}

/// T-098: an answer from an address that the client sent nothing to is not
/// relayed back.
#[tokio::test]
async fn front_door_udp_associate_relays_answers_from_known_peers_only() {
    pod_testkit::need_loopback!();
    let (server, _) = echo().await;
    let udp = Udp::new(None);
    let door = Arc::new(Door::new(udp.clone()));
    let (mut control, stream) = tokio::io::duplex(4096);
    door.clone().spawn(stream);
    let bound = bound_of(&associate(&mut control).await);
    let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    client.send_to(&wrap(server, b"hi"), bound).await.unwrap();
    assert!(answer(&client, Duration::from_secs(5)).await.is_some());
    // A host that the client sent nothing to writes to the socket of the
    // way: the relay drops it.
    let local = udp.local.lock().unwrap().expect("the socket of the way");
    let other = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    other.send_to(b"unasked", local).await.unwrap();
    assert!(answer(&client, Duration::from_millis(300)).await.is_none());
    // The known peer still gets through.
    client
        .send_to(&wrap(server, b"again"), bound)
        .await
        .unwrap();
    assert_eq!(
        answer(&client, Duration::from_secs(5)).await,
        Some(wrap(server, b"re:again"))
    );
}

/// T-098: a dialer with no datagrams gives "command not supported"; a
/// dialer that the kill switch refuses gives "not allowed".
#[tokio::test]
async fn front_door_udp_associate_replies_for_a_way_with_no_datagrams() {
    for (class, code) in [(Class::Unavailable, 0x07), (Class::Policy, 0x02)] {
        let door = Arc::new(Door::new(Udp::new(Some(class))));
        let (mut client, server) = tokio::io::duplex(4096);
        door.clone().spawn(server);
        client.write_all(&[5, 1, 0]).await.unwrap();
        let mut method = [0u8; 2];
        client.read_exact(&mut method).await.unwrap();
        client
            .write_all(&[5, 3, 0, 1, 0, 0, 0, 0, 0, 0])
            .await
            .unwrap();
        let mut out = Vec::new();
        client.read_to_end(&mut out).await.unwrap();
        assert_eq!(out[1], code, "{class:?}: {out:?}");
    }
}
