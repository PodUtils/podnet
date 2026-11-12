//! Tests of the WireGuard network (T-117) against a userspace WireGuard
//! peer on the loopback. The peer has its own stack, and an HTTP service
//! behind it. `pod-wg` has the keys and the handshake of the responder
//! (`standin::respond`); the transport messages are sealed here.

use std::net::SocketAddr;
use std::time::Duration;

use base64::Engine as _;
use pod_netstack::{NetStack, StackConfig, Subnet, TcpListener};
use pod_wg::standin;
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, CHACHA20_POLY1305};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use super::*;
use crate::network::{Class, Ctx, Target};

const SERVICE: &str = "10.9.0.1:8080";

fn key(bytes: &[u8; 32]) -> LessSafeKey {
    LessSafeKey::new(UnboundKey::new(&CHACHA20_POLY1305, bytes).unwrap())
}

fn nonce(counter: u64) -> Nonce {
    let mut n = [0u8; 12];
    n[4..].copy_from_slice(&counter.to_le_bytes());
    Nonce::assume_unique_for_key(n)
}

/// An IPv4 packet with its padding cut by the length in its header.
fn cut(mut packet: Vec<u8>) -> Vec<u8> {
    if packet.len() >= 4 && packet[0] >> 4 == 4 {
        let len = u16::from_be_bytes([packet[2], packet[3]]) as usize;
        packet.truncate(len.min(packet.len()));
    }
    packet
}

/// The send key, the receive key, the index of the client, its address and
/// the counter of the messages out.
type Session = ([u8; 32], [u8; 32], [u8; 4], SocketAddr, u64);

/// The WireGuard peer: it answers the handshake of `client_public`, puts
/// each packet of the client in `stack`, and seals each packet of `stack`.
async fn peer(
    sock: tokio::net::UdpSocket,
    private: [u8; 32],
    client_public: [u8; 32],
    stack: NetStack,
) {
    let mut buf = vec![0u8; 2048];
    let mut session: Option<Session> = None;
    let out = stack.link_out();
    loop {
        tokio::select! {
            got = sock.recv_from(&mut buf) => {
                let Ok((n, from)) = got else { return };
                let m = &buf[..n];
                if m.first() == Some(&pod_wg::MSG_INITIATION) && n == pod_wg::INITIATION_LEN {
                    let (response, send, recv, _) = standin::respond(&private, &client_public, m);
                    session = Some((send, recv, m[4..8].try_into().unwrap(), from, 0));
                    let _ = sock.send_to(&response, from).await;
                } else if m.first() == Some(&pod_wg::MSG_DATA) && n >= 32 {
                    let Some((_, recv, _, _, _)) = &session else { continue };
                    let counter = u64::from_le_bytes(m[8..16].try_into().unwrap());
                    let mut body = m[16..].to_vec();
                    if let Ok(plain) = key(recv).open_in_place(nonce(counter), Aad::empty(), &mut body) {
                        if !plain.is_empty() {
                            stack.link_in().push(cut(plain.to_vec()));
                        }
                    }
                }
            }
            packet = out.recv() => {
                let Some((send, _, receiver, to, counter)) = &mut session else { continue };
                let mut body = packet;
                body.resize(body.len().div_ceil(16) * 16, 0);
                key(send).seal_in_place_append_tag(nonce(*counter), Aad::empty(), &mut body).unwrap();
                let mut message = vec![pod_wg::MSG_DATA, 0, 0, 0];
                message.extend_from_slice(receiver);
                message.extend_from_slice(&counter.to_le_bytes());
                message.extend_from_slice(&body);
                *counter += 1;
                let _ = sock.send_to(&message, *to).await;
            }
        }
    }
}

/// The service behind the peer: one answer for each connection.
async fn serve(mut listener: TcpListener) {
    while let Ok(mut stream) = listener.accept().await {
        tokio::spawn(async move {
            let mut head = [0u8; 512];
            let _ = stream.read(&mut head).await;
            let body = "behind the peer";
            let answer = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(answer.as_bytes()).await;
            stream.shutdown();
        });
    }
}

/// A peer on the loopback; the wg-quick file of its client is in `dir`.
async fn start(dir: &std::path::Path) -> std::path::PathBuf {
    let (keys, private) = standin::keys([0; 3]);
    let sock = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let endpoint = sock.local_addr().unwrap();
    let stack = NetStack::over_link(StackConfig {
        subnet: Subnet {
            v4: "10.9.0.0".parse().unwrap(),
            ..Subnet::default()
        },
        local_v4: "10.9.0.1".parse().unwrap(),
        mtu: 1420,
        ..StackConfig::default()
    });
    let listener = stack
        .listen(SERVICE.parse::<SocketAddr>().unwrap())
        .await
        .unwrap();
    tokio::spawn(serve(listener));
    tokio::spawn(peer(sock, private, keys.public(), stack));
    let b64 = base64::engine::general_purpose::STANDARD;
    let file = dir.join("wg0.conf");
    std::fs::write(
        &file,
        format!(
            "[Interface]\nPrivateKey = {}\nAddress = 10.9.0.2/32\n\n[Peer]\nPublicKey = {}\nEndpoint = {endpoint}\nAllowedIPs = 10.9.0.0/24\n",
            b64.encode(keys.private),
            b64.encode(keys.peer_public),
        ),
    )
    .unwrap();
    file
}

async fn get(network: &SpecNetwork, ctx: &Ctx) -> String {
    let link = network.up(ctx).await.unwrap();
    let way = link.way.as_ref().expect("a way out");
    let target = Target::Addr(SERVICE.parse().unwrap());
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let mut io = way.dial.dial(&target, deadline).await.unwrap().io;
    io.write_all(b"GET / HTTP/1.1\r\nHost: peer\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut text = String::new();
    tokio::time::timeout(Duration::from_secs(20), io.read_to_string(&mut text))
        .await
        .expect("the answer comes in 20 s")
        .unwrap();
    link.close().await;
    text
}

#[tokio::test]
async fn wg_network_reaches_a_service_behind_the_peer() {
    crate::test_host::need_loopback!();
    let dir = tempfile::tempdir().unwrap();
    let file = start(dir.path()).await;
    let network = parse(&format!("wg:{}", file.display())).unwrap();
    assert_eq!(network.scheme(), "wg");
    let caps = network.caps().names();
    assert_eq!(caps, ["egress-tcp", "egress-udp"]);
    let text = get(&network, &Ctx::default()).await;
    assert!(text.starts_with("HTTP/1.1 200 OK"), "{text}");
    assert!(text.ends_with("behind the peer"), "{text}");
}

#[tokio::test]
async fn wg_network_sends_its_udp_over_the_datagrams_of_the_way_below() {
    crate::test_host::need_loopback!();
    let dir = tempfile::tempdir().unwrap();
    let file = start(dir.path()).await;
    let network = parse(&format!("wg:{}", file.display())).unwrap();
    // The carrier is a way that carries UDP: the direct way is one.
    let ctx = Ctx {
        below: Some(std::sync::Arc::new(crate::dialers::Direct)),
        state_dir: None,
    };
    let text = get(&network, &ctx).await;
    assert!(text.ends_with("behind the peer"), "{text}");
    // A way that carries no UDP is an error that names the way.
    let ctx = Ctx {
        below: Some(crate::dialers::proxy(
            crate::egress::Proxy::parse_trusted("socks5h://127.0.0.1:9").unwrap(),
        )),
        state_dir: None,
    };
    let err = network.up(&ctx).await.unwrap_err();
    assert!(format!("{err:#}").contains("carries no UDP"), "{err:#}");
}

#[test]
fn wg_network_spec_a_bad_file_is_one_usage_line() {
    let dir = tempfile::tempdir().unwrap();
    let b64 = base64::engine::general_purpose::STANDARD;
    let key = b64.encode([7u8; 32]);
    let file = |extra: &str, endpoint: bool| {
        let path = dir.path().join("w.conf");
        let endpoint = if endpoint {
            "Endpoint = 127.0.0.1:51820\n"
        } else {
            ""
        };
        std::fs::write(
            &path,
            format!("[Interface]\nPrivateKey = {key}\nAddress = 10.9.0.2/32\n\n[Peer]\nPublicKey = {key}\n{endpoint}{extra}"),
        )
        .unwrap();
        format!("wg:{}", path.display())
    };
    assert!(parse(&file("", true)).is_ok());
    let err = parse(&file(&format!("PresharedKey = {key}\n"), true)).unwrap_err();
    assert!(err.text().contains("PresharedKey"), "{}", err.text());
    assert_eq!(err.class, Class::Usage);
    let err = parse(&file("", false)).unwrap_err();
    assert!(err.text().contains("no Endpoint"), "{}", err.text());
    let err = parse("wg:").unwrap_err();
    assert!(err.text().contains("wg-quick file"), "{}", err.text());
    let err = parse("wg:/nonexistent/wg0.conf").unwrap_err();
    assert!(
        err.text().contains("/nonexistent/wg0.conf"),
        "{}",
        err.text()
    );
}
