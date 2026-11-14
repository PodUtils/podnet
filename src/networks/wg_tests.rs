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

/// The WireGuard peer: it answers the handshake of `client_public` with the
/// preshared key `psk` (`None`: the zero key), puts each packet of the
/// client in `stack`, and seals each packet of `stack`.
async fn peer(
    sock: tokio::net::UdpSocket,
    private: [u8; 32],
    client_public: [u8; 32],
    psk: Option<[u8; 32]>,
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
                    let mut e_priv = [0u8; 32];
                    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut e_priv).unwrap();
                    let (response, send, recv, _) =
                        standin::respond_preshared(&private, &client_public, psk.as_ref(), m, &e_priv, 77);
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
    start_at(dir, None, "").await.0
}

/// The DNS server behind the peer, on TCP (RFC 7766): each A question gets
/// the address of the service, each other question no answer.
async fn serve_dns(mut listener: TcpListener) {
    while let Ok(mut stream) = listener.accept().await {
        tokio::spawn(async move {
            loop {
                let mut len = [0u8; 2];
                if stream.read_exact(&mut len).await.is_err() {
                    return;
                }
                let mut query = vec![0u8; usize::from(u16::from_be_bytes(len))];
                if stream.read_exact(&mut query).await.is_err() || query.len() < 12 {
                    return;
                }
                let kind = query[query.len() - 4..query.len() - 2].to_vec();
                let mut answer = query.clone();
                answer[2] = 0x81;
                answer[3] = 0x80;
                if kind == [0, 1] {
                    answer[7] = 1;
                    answer.extend_from_slice(&[0xc0, 12, 0, 1, 0, 1, 0, 0, 0, 60, 0, 4]);
                    answer.extend_from_slice(&[10, 9, 0, 1]);
                }
                let mut framed = (answer.len() as u16).to_be_bytes().to_vec();
                framed.extend_from_slice(&answer);
                if stream.write_all(&framed).await.is_err() {
                    return;
                }
            }
        });
    }
}

/// [`start`] with `shown` as the `Endpoint` of the file, else the address of
/// the peer, and `interface` as more lines of `[Interface]`. It gives the
/// file and the address of the peer.
async fn start_at(
    dir: &std::path::Path,
    shown: Option<SocketAddr>,
    interface: &str,
) -> (std::path::PathBuf, SocketAddr) {
    start_psk(dir, shown, interface, None, None).await
}

/// [`start_at`] with the preshared key `file_psk` in the `[Peer]` of the
/// file, and `peer_psk` at the peer.
async fn start_psk(
    dir: &std::path::Path,
    shown: Option<SocketAddr>,
    interface: &str,
    file_psk: Option<[u8; 32]>,
    peer_psk: Option<[u8; 32]>,
) -> (std::path::PathBuf, SocketAddr) {
    let (keys, private) = standin::keys([0; 3]);
    let sock = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let outside = sock.local_addr().unwrap();
    let endpoint = shown.unwrap_or(outside);
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
    let dns = stack
        .listen("10.9.0.1:53".parse::<SocketAddr>().unwrap())
        .await
        .unwrap();
    tokio::spawn(serve_dns(dns));
    tokio::spawn(peer(sock, private, keys.public(), peer_psk, stack));
    let b64 = base64::engine::general_purpose::STANDARD;
    let psk_line = file_psk.map_or(String::new(), |k| {
        format!("PresharedKey = {}\n", b64.encode(k))
    });
    let file = dir.join("wg0.conf");
    std::fs::write(
        &file,
        format!(
            "[Interface]\nPrivateKey = {}\nAddress = 10.9.0.2/32\n{interface}\n[Peer]\nPublicKey = {}\nEndpoint = {endpoint}\nAllowedIPs = 10.9.0.0/24\n{psk_line}",
            b64.encode(keys.private),
            b64.encode(keys.peer_public),
        ),
    )
    .unwrap();
    (file, outside)
}

async fn get(network: &dyn Network, ctx: &Ctx) -> String {
    get_with_status(network, ctx).await.0
}

/// The answer of the service through `network`, and the status of the link.
async fn get_with_status(network: &dyn Network, ctx: &Ctx) -> (String, serde_json::Value) {
    fetch(network, ctx, Target::Addr(SERVICE.parse().unwrap())).await
}

/// The answer of the service at `target` through `network`, and the status
/// of the link.
async fn fetch(network: &dyn Network, ctx: &Ctx, target: Target) -> (String, serde_json::Value) {
    let link = network.up(ctx).await.unwrap();
    let status = link.status.clone();
    let way = link.way.as_ref().expect("a way out");
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
    (text, status)
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
    // T-337: a good PresharedKey is taken; a bad one is a usage line (exit 64),
    // as each other key of the file.
    assert!(parse(&file(&format!("PresharedKey = {key}\n"), true)).is_ok());
    let err = parse(&file("PresharedKey = c2hvcnQ=\n", true)).unwrap_err();
    assert!(err.text().contains("32 bytes in base64"), "{}", err.text());
    assert_eq!(err.class, Class::Usage);
    assert_eq!(err.class.exit_code(), 64);
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

// T-117: the WARP chain. The UDP of the tunnel goes through the datagrams of
// a WARP session (the stand-in edge); the peer is a UDP route of the far end.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wg_network_over_warp_reaches_a_service_behind_the_peer() {
    crate::test_host::need_loopback!();
    let dir = tempfile::tempdir().unwrap();
    // The endpoint is an address inside WARP; the edge carries its port to
    // the peer on the loopback.
    let inside: SocketAddr = "203.0.113.20:51820".parse().unwrap();
    let (file, outside) = start_at(dir.path(), Some(inside), "").await;
    let route = super::warp_tests::edge::Route {
        port: inside.port(),
        outside,
    };
    let world = super::warp_tests::world_with_udp(vec![route]).await;
    let wg = parse(&format!("wg:{}", file.display())).unwrap();
    let layers: Vec<std::sync::Arc<dyn Network>> = vec![
        std::sync::Arc::new(world.network()),
        std::sync::Arc::new(wg),
    ];
    let chain = crate::network::Chain::new(layers).unwrap();
    assert_eq!(chain.label(), "wg over warp");
    let state = tempfile::tempdir().unwrap();
    let (text, status) = get_with_status(&chain, &super::warp_tests::ctx_of(state.path())).await;
    assert!(text.ends_with("behind the peer"), "{text}");
    assert_eq!(status[1]["status"]["carrier"], "WARP", "{status}");
    // The far end of WARP saw the UDP of the tunnel to the endpoint.
    let seen = world.edge.watch.udp.lock().unwrap().clone();
    assert!(seen.iter().any(|(_, to)| *to == inside), "{seen:?}");
}

// T-117: a spec `TOP over BELOW` is a chain of the registry.
#[tokio::test]
async fn network_spec_a_chain_comes_up_layer_over_layer() {
    crate::test_host::need_loopback!();
    let dir = tempfile::tempdir().unwrap();
    let file = start(dir.path()).await;
    let spec = format!("wg:{} over direct", file.display());
    let network = parse(&spec).unwrap();
    assert!(network.is_chain());
    assert_eq!(network.scheme(), "chain");
    assert_eq!(network.label(), spec);
    assert_eq!(network.caps().names(), ["egress-tcp", "egress-udp"]);
    let (text, status) = get_with_status(&network, &Ctx::default()).await;
    assert!(text.ends_with("behind the peer"), "{text}");
    assert_eq!(status[0]["scheme"], "direct", "{status}");
    assert_eq!(status[1]["status"]["carrier"], "direct", "{status}");
    // A layer that takes no way below it is refused, and the error names it.
    let err = parse("warp over direct")
        .unwrap()
        .up(&Ctx::default())
        .await
        .unwrap_err();
    let text = format!("{err:#}");
    assert!(text.contains("layer 2 of 2 (warp)"), "{text}");
    assert!(text.contains("cannot go over another way"), "{text}");
}

#[test]
fn network_spec_a_chain_is_checked_layer_by_layer() {
    for (spec, part) in [
        (" over warp", "on each side"),
        ("warp over ", "on each side"),
        ("warp over relay:", "layer 2"),
        ("wg: over direct", "layer 1"),
    ] {
        let err = parse(spec).expect_err(spec);
        assert_eq!(err.class, Class::Usage, "{spec}");
        assert!(err.text().contains(part), "{spec}: {}", err.text());
        assert!(!err.text().contains('\n'), "{}", err.text());
    }
    let nine = ["direct"; 9].join(" over ");
    let err = parse(&nine).unwrap_err();
    assert!(err.text().contains("at most 8 layers"), "{}", err.text());
    // A layer keeps no credential in a label or an error.
    let chain = parse("socks5h://u:pw@p.example:1080 over direct").unwrap();
    assert_eq!(chain.label(), "socks5h://p.example:1080 over direct");
    let err = parse("http://u:pw@p.example:1 over bad://u:pw@x").unwrap_err();
    assert!(!err.text().contains("pw"), "{}", err.text());
    // A command can hold the word over: a spec with +exec: is one layer.
    let one = parse("socks5h+exec:echo go over there").unwrap();
    assert!(!one.is_chain());
}

// T-117: a name resolves at the DNS server of the file, through the tunnel.
#[tokio::test]
async fn wg_network_resolves_a_name_at_the_dns_of_the_file() {
    crate::test_host::need_loopback!();
    let dir = tempfile::tempdir().unwrap();
    let (file, _) = start_at(dir.path(), None, "DNS = 10.9.0.1, corp.example\n").await;
    let network = parse(&format!("wg:{}", file.display())).unwrap();
    let target = Target::new("service.corp.example", 8080);
    let (text, status) = fetch(&network, &Ctx::default(), target).await;
    assert!(text.ends_with("behind the peer"), "{text}");
    assert_eq!(status["dns"], "10.9.0.1", "{status}");
    // With no DNS line, a name is one line that names the fix.
    let (plain, _) = start_at(dir.path(), None, "").await;
    let network = parse(&format!("wg:{}", plain.display())).unwrap();
    let link = network.up(&Ctx::default()).await.unwrap();
    let way = link.way.clone().unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let dialed = way
        .dial
        .dial(&Target::new("service.corp.example", 8080), deadline)
        .await;
    let Err(err) = dialed else {
        panic!("no DNS line: no name")
    };
    assert!(err.text().contains("DNS = ADDRESS"), "{}", err.text());
    link.close().await;
}

// T-337: the PresharedKey of the file goes into the handshake.
#[tokio::test]
async fn wg_network_takes_a_preshared_key() {
    crate::test_host::need_loopback!();
    let dir = tempfile::tempdir().unwrap();
    let psk = Some([0x5a; 32]);
    let (file, _) = start_psk(dir.path(), None, "", psk, psk).await;
    let network = parse(&format!("wg:{}", file.display())).unwrap();
    let text = get(&network, &Ctx::default()).await;
    assert!(text.ends_with("behind the peer"), "{text}");
}

// T-337: another key, or a key at one end only, is refused at once (exit
// 77), with the text of the crate and the name of the line.
#[tokio::test]
async fn wg_network_refuses_a_preshared_key_that_differs() {
    crate::test_host::need_loopback!();
    let (a, b) = (Some([0x5a; 32]), Some([0xa5; 32]));
    for (file_psk, peer_psk) in [(a, b), (a, None), (None, b)] {
        let dir = tempfile::tempdir().unwrap();
        let (file, _) = start_psk(dir.path(), None, "", file_psk, peer_psk).await;
        let network = parse(&format!("wg:{}", file.display())).unwrap();
        let started = std::time::Instant::now();
        let err = network.up(&Ctx::default()).await.unwrap_err();
        let failure = err.downcast_ref::<Failure>().expect("a Failure");
        let text = failure.text().to_string();
        assert_eq!(failure.class, Class::Refused, "{text}");
        assert_eq!(crate::exit::code_of(&super::up_fault(err)), 77, "{text}");
        assert!(
            text.contains(&pod_wg::Error::ResponseNotAuthentic.to_string()),
            "{text}"
        );
        assert!(text.contains("PresharedKey"), "{text}");
        assert!(started.elapsed() < Duration::from_secs(5), "{text}");
    }
}
