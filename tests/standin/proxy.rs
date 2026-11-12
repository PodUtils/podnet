//! A CONNECT proxy on the loopback. It keeps the authority of each CONNECT
//! request, as `scripts/connect-proxy.py` does for `live.yml`.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// The authorities that the proxy got, in order.
pub type Log = Arc<Mutex<Vec<String>>>;

/// Serve the proxy on a free loopback port.
pub async fn serve() -> (SocketAddr, Log) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind the proxy");
    let addr = listener.local_addr().unwrap();
    let log: Log = Arc::default();
    let seen = log.clone();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let log = seen.clone();
            tokio::spawn(async move {
                let _ = handle(stream, log).await;
            });
        }
    });
    (addr, log)
}

async fn handle(mut client: TcpStream, log: Log) -> std::io::Result<()> {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if client.read(&mut byte).await? == 0 {
            return Ok(());
        }
        head.push(byte[0]);
    }
    let text = String::from_utf8_lossy(&head).to_string();
    let mut words = text.split_whitespace();
    let (method, authority) = (words.next().unwrap_or(""), words.next().unwrap_or(""));
    if method != "CONNECT" {
        return client
            .write_all(b"HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\n\r\n")
            .await;
    }
    log.lock().unwrap().push(authority.to_string());
    let mut upstream = TcpStream::connect(authority).await?;
    client
        .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
        .await?;
    tokio::io::copy_bidirectional(&mut client, &mut upstream)
        .await
        .map(|_| ())
}
