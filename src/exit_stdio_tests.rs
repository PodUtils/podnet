use super::*;

/// T-329: a stream with no UDP socket carries CONNECT and UDP in the stream
/// over the dial of the process; the command 3 is refused.
#[tokio::test]
async fn exit_stdio_serves_connect_and_refuses_associate() {
    crate::test_host::need_loopback!();
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let echo = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = echo.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (mut s, _) = echo.accept().await.unwrap();
        let mut buf = [0u8; 16];
        let n = s.read(&mut buf).await.unwrap();
        s.write_all(&buf[..n]).await.unwrap();
    });
    let (mut client, stream) = tokio::io::duplex(4096);
    let dial = Arc::new(ExitDial::with(Arc::new(crate::dialers::Direct), None));
    tokio::spawn(serve(stream, dial.clone()));
    client.write_all(&[5, 1, 0]).await.unwrap();
    let mut method = [0u8; 2];
    client.read_exact(&mut method).await.unwrap();
    let mut req = vec![5, 1, 0, 1, 127, 0, 0, 1];
    req.extend_from_slice(&port.to_be_bytes());
    client.write_all(&req).await.unwrap();
    let mut reply = [0u8; 10];
    client.read_exact(&mut reply).await.unwrap();
    assert_eq!(reply[1], 0, "{reply:?}");
    client.write_all(b"ping").await.unwrap();
    let mut got = [0u8; 4];
    client.read_exact(&mut got).await.unwrap();
    assert_eq!(&got, b"ping");

    let (mut client, stream) = tokio::io::duplex(4096);
    tokio::spawn(serve(stream, dial));
    client.write_all(&[5, 1, 0]).await.unwrap();
    client.read_exact(&mut method).await.unwrap();
    client
        .write_all(&[5, 3, 0, 1, 0, 0, 0, 0, 0, 0])
        .await
        .unwrap();
    let mut out = Vec::new();
    client.read_to_end(&mut out).await.unwrap();
    assert_eq!(out[1], 0x07, "{out:?}");
}

/// GitHub #719: the reply of the exit follows the class of the failure of
/// the dial (RFC 1928; `docs/proxies.md`): a name with no address and a
/// time limit get 0x04, a refused port 0x05.
#[tokio::test]
async fn the_reply_of_a_failed_dial_follows_its_class() {
    crate::test_host::need_loopback!();
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    /// A dial that ends at its time limit.
    struct Short;
    impl Dial for Short {
        fn label(&self) -> String {
            "short".into()
        }
        fn dial<'a>(
            &'a self,
            target: &'a Target,
            _: Instant,
        ) -> BoxFuture<'a, Result<Connected, Failure>> {
            // The failure of a dial that ends at its time limit, as
            // pod-proxy gives it, through the mapping of podnet's dialers
            // (a host network can refuse 192.0.2.1 at once).
            let timeout = pod_proxy::DialError::Timeout {
                step: format!("connecting to {target}"),
                after: std::time::Duration::from_secs(30),
                stage: Stage::Connect,
            };
            let failure = crate::dialers::failure(anyhow::Error::new(timeout), Stage::Connect);
            Box::pin(async move { Err(failure) })
        }
    }
    let (_held, refused) = pod_testkit::refused::refused();
    let name = |host: &str, port: u16| {
        let mut req = vec![5, 1, 0, 3, host.len() as u8];
        req.extend_from_slice(host.as_bytes());
        req.extend_from_slice(&port.to_be_bytes());
        req
    };
    let direct: Arc<dyn Dial> = Arc::new(crate::dialers::Direct);
    let short: Arc<dyn Dial> = Arc::new(Short);
    type Case = (&'static str, Arc<dyn Dial>, Vec<u8>, u8);
    let cases: [Case; 3] = [
        (
            "a refused port",
            direct.clone(),
            name("127.0.0.1", refused.port()),
            0x05,
        ),
        (
            "a name with no address",
            direct,
            name("nothing.invalid", 80),
            0x04,
        ),
        ("a time limit", short, name("192.0.2.1", 443), 0x04),
    ];
    for (what, dial, req, want) in cases {
        let (mut client, stream) = tokio::io::duplex(4096);
        tokio::spawn(serve(stream, Arc::new(ExitDial::with(dial, None))));
        client.write_all(&[5, 1, 0]).await.unwrap();
        let mut method = [0u8; 2];
        client.read_exact(&mut method).await.unwrap();
        client.write_all(&req).await.unwrap();
        let mut reply = [0u8; 10];
        client.read_exact(&mut reply).await.unwrap();
        assert_eq!(reply[1], want, "{what}: {reply:?}");
    }
}
