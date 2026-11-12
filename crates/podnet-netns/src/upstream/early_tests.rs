use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::*;

const OK_REPLY: [u8; 10] = [5, 0, 0, 1, 0, 0, 0, 0, 0, 0];

fn hop(user: Option<&str>) -> Proxy {
    let mut proxy = Proxy::parse("socks5://127.0.0.1:1").unwrap();
    proxy.user = user.map(str::to_string);
    proxy.pass = user.map(|_| "secret".to_string());
    proxy
}

/// The hop writes the greeting and the request in one flight, and reads
/// the two replies after it.
#[tokio::test]
async fn a_pipelined_hop_sends_the_greeting_and_the_request_together() {
    let (near, mut far) = tokio::io::duplex(1024);
    let server = tokio::spawn(async move {
        // 3 bytes of the greeting and 10 of the request, with no reply
        // between them: a serial hop would wait for the method reply.
        let mut flight = [0u8; 13];
        tokio::time::timeout(Duration::from_secs(5), far.read_exact(&mut flight))
            .await
            .expect("the request came with the greeting")
            .unwrap();
        far.write_all(&[5, 0]).await.unwrap();
        far.write_all(&OK_REPLY).await.unwrap();
        flight
    });
    let up: Upstream = Box::new(near);
    socks5_hop(up, &hop(None), "1.2.3.4:80", Handshake::Pipelined)
        .await
        .unwrap();
    let flight = server.await.unwrap();
    assert_eq!(&flight[..3], &[5, 1, 0]);
    assert_eq!(&flight[3..], &[5, 1, 0, 1, 1, 2, 3, 4, 0, 80]);
}

/// A hop with a user and a password stays serial: the request waits for
/// the method reply.
#[tokio::test]
async fn a_hop_with_a_user_stays_serial_in_every_order() {
    for handshake in [
        Handshake::Pipelined,
        Handshake::Optimistic(Arc::new(|_| {})),
    ] {
        let (near, mut far) = tokio::io::duplex(1024);
        let client = tokio::spawn(async move {
            let up: Upstream = Box::new(near);
            let _ = socks5_hop(up, &hop(Some("u")), "1.2.3.4:80", handshake).await;
        });
        let mut greeting = [0u8; 4];
        far.read_exact(&mut greeting).await.unwrap();
        assert_eq!(greeting, [5, 2, 0, 2]);
        let mut more = [0u8; 1];
        let early = tokio::time::timeout(Duration::from_millis(100), far.read(&mut more)).await;
        assert!(early.is_err(), "no byte before the method reply: {more:?}");
        client.abort();
    }
}

/// The optimistic hop returns before the replies. The bytes written to it
/// are behind the request, and its first read checks the replies.
#[tokio::test]
async fn an_optimistic_hop_returns_before_the_replies() {
    let (near, mut far) = tokio::io::duplex(1024);
    let up: Upstream = Box::new(near);
    let mut hop = socks5_hop(
        up,
        &hop(None),
        "1.2.3.4:80",
        Handshake::Optimistic(Arc::new(|_| {})),
    )
    .await
    .unwrap();
    hop.write_all(b"early").await.unwrap();
    let mut got = [0u8; 18];
    far.read_exact(&mut got).await.unwrap();
    assert_eq!(&got[13..], b"early");
    // The replies and the first data in one write: no byte is lost.
    let mut reply = vec![5, 0];
    reply.extend_from_slice(&OK_REPLY);
    reply.extend_from_slice(b"data");
    far.write_all(&reply).await.unwrap();
    let mut data = [0u8; 4];
    hop.read_exact(&mut data).await.unwrap();
    assert_eq!(&data, b"data");
}

/// A refusal is an error of the first read, and the hook gets the code.
#[tokio::test]
async fn an_optimistic_hop_reports_the_code_of_a_refusal() {
    let code = Arc::new(AtomicU8::new(0));
    let seen = code.clone();
    let (near, mut far) = tokio::io::duplex(1024);
    let up: Upstream = Box::new(near);
    let mut hop = socks5_hop(
        up,
        &hop(None),
        "example.test:443",
        Handshake::Optimistic(Arc::new(move |c| seen.store(c, Ordering::SeqCst))),
    )
    .await
    .unwrap();
    far.write_all(&[5, 0, 5, 5, 0, 1]).await.unwrap();
    let mut byte = [0u8; 1];
    let err = hop.read(&mut byte).await.unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::ConnectionRefused);
    assert_eq!(
        err.to_string(),
        "SOCKS5 connect to example.test:443 failed: code 5"
    );
    assert_eq!(code.load(Ordering::SeqCst), 5);
}

/// An upstream that sends no reply ends the first read at the time limit.
#[tokio::test(start_paused = true)]
async fn an_optimistic_hop_ends_at_the_time_limit() {
    let (near, _far) = tokio::io::duplex(1024);
    let up: Upstream = Box::new(near);
    let mut hop = socks5_hop(
        up,
        &hop(None),
        "1.2.3.4:80",
        Handshake::Optimistic(Arc::new(|_| {})),
    )
    .await
    .unwrap();
    let mut byte = [0u8; 1];
    let err = hop.read(&mut byte).await.unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::TimedOut);
}

/// A pipelined hop that is refused gives the typed error with the code.
#[tokio::test]
async fn a_pipelined_hop_gives_the_typed_refusal() {
    let (near, mut far) = tokio::io::duplex(1024);
    tokio::spawn(async move {
        let mut flight = [0u8; 13];
        far.read_exact(&mut flight).await.unwrap();
        far.write_all(&[5, 0, 5, 2, 0, 1, 0, 0, 0, 0, 0, 0])
            .await
            .unwrap();
    });
    let up: Upstream = Box::new(near);
    let err = match socks5_hop(up, &hop(None), "1.2.3.4:80", Handshake::Pipelined).await {
        Ok(_) => panic!("the upstream refused"),
        Err(err) => err,
    };
    let refused = err.downcast_ref::<UpstreamRefused>().expect("typed");
    assert_eq!(refused.code, 2);
    assert_eq!(refused.target, "1.2.3.4:80");
}
