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
