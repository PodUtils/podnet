use super::*;

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

/// A stand-in host of the relay that answers each request with HTTP 200
/// and a short page, as the stand-ins of the lab `proxy-only` do.
async fn stand_in() -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut seen = Vec::new();
                let mut buf = [0u8; 1024];
                while !seen.windows(4).any(|w| w == b"\r\n\r\n") {
                    match socket.read(&mut buf).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => seen.extend_from_slice(&buf[..n]),
                    }
                }
                let page = "pod-lab stand-in";
                let answer = format!(
                    "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{page}",
                    page.len()
                );
                let _ = socket.write_all(answer.as_bytes()).await;
            });
        }
    });
    port
}

/// GitHub #738: the failure of `warp up` through the relay names each
/// cause once. The legs fail as in the lab `proxy-only`: the IPv6 door
/// and the IPv4 door through a relay of four hosts that each answer HTTP
/// 200. The words around the failures of the dial are those of pod-warp
/// (`session.rs`, `open.rs`).
#[tokio::test]
async fn a_failure_through_the_relay_names_each_cause_once() {
    crate::test_host::need_loopback!();
    let mut urls = Vec::new();
    for _ in 0..4 {
        urls.push(format!("http://127.0.0.1:{}", stand_in().await));
    }
    let relay = Arc::new(
        crate::ws_relay::Relay::new(&urls.join(","), Some("testonly-token-0123456789".into()))
            .unwrap(),
    );
    let url = relay.url();
    let dial = RelayDial(relay);
    let way = dial.label();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    let v4: std::net::SocketAddr = "162.159.198.2:443".parse().unwrap();
    let v6: std::net::SocketAddr = "[2606:4700:103::2]:443".parse().unwrap();
    let mut legs = Vec::new();
    for door in [v4, v6] {
        let Err(failure) = dial.dial(&Target::Addr(door), deadline).await else {
            panic!("the stand-ins carry nothing");
        };
        // pod-warp, session.rs: the dial of one door.
        let text = format!("TCP to the MASQUE front door {door} through {way}");
        legs.push(Failure::caused(failure.stage, failure.class, text, failure));
    }
    let (first, other) = (legs.remove(0), legs.remove(0));
    // pod-warp, open.rs: the other address, then the relay leg.
    let text = format!(
        "{}; the other address of the front door failed too ({})",
        first.text(),
        other.chain()
    );
    let through = Failure::caused(first.stage, first.class, text, first);
    let direct = Failure::new(
        Stage::Tls,
        Class::Unavailable,
        "TLS handshake with the MASQUE front door 162.159.198.2:443: no answer",
    );
    let text = format!(
        "{}; and through the relay {url}: {}",
        direct.chain(),
        through.chain()
    );
    let err = anyhow::Error::new(Failure::caused(direct.stage, direct.class, text, direct));
    let line = crate::report::error_line(&err);
    let first = line.lines().next().unwrap_or_default();
    let once = [
        format!("through the relay {url}"),
        "the relay has no IPv6 egress".to_string(),
        "HTTP 200: pod-lab stand-in".to_string(),
        "TCP to the MASQUE front door 162.159.198.2:443".to_string(),
    ];
    for phrase in once {
        assert_eq!(first.matches(&phrase).count(), 1, "{phrase}: {first}");
    }
    assert!(!line.contains("connect hop"), "{line}");
    assert!(first.len() < 600, "{} bytes: {first}", first.len());
}
