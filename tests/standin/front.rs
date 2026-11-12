//! The front of the stand-in: an HTTP/1.1 server on the loopback. It maps
//! the `Host` of a visitor to the tunnel of that name and sends the
//! request down a stream of the tunnel, as the edge does.

use std::net::SocketAddr;
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use super::edge::Edge;

/// Serve the front on a free loopback port.
pub async fn serve(edge: Arc<Edge>) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind the front");
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let edge = edge.clone();
            tokio::spawn(async move {
                let _ = handle(edge, stream).await;
            });
        }
    });
    addr
}

async fn handle(edge: Arc<Edge>, mut stream: TcpStream) -> std::io::Result<()> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let (head_len, method, path, headers) = loop {
        let mut slots = [httparse::EMPTY_HEADER; 64];
        let mut request = httparse::Request::new(&mut slots);
        if let Ok(httparse::Status::Complete(len)) = request.parse(&buf) {
            let headers: Vec<(String, String)> = request
                .headers
                .iter()
                .map(|h| {
                    (
                        h.name.to_string(),
                        String::from_utf8_lossy(h.value).to_string(),
                    )
                })
                .collect();
            break (
                len,
                request.method.unwrap_or("GET").to_string(),
                request.path.unwrap_or("/").to_string(),
                headers,
            );
        }
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let value = |name: &str| {
        headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.clone())
    };
    let length: usize = value("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    while buf.len() < head_len + length {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let body = buf[head_len..].to_vec();
    let host = value("host").unwrap_or_default();
    let hostname = host.split(':').next().unwrap_or("").to_string();
    // The headers of the connection are not the headers of the stream.
    let forwarded: Vec<(String, String)> = headers
        .into_iter()
        .filter(|(n, _)| {
            !["host", "connection", "content-length", "transfer-encoding"]
                .iter()
                .any(|skip| n.eq_ignore_ascii_case(skip))
        })
        .collect();
    let (status, answer_headers, answer) = match edge
        .visit(&hostname, &method, &path, &forwarded, body)
        .await
    {
        Ok(visit) => (visit.status, visit.headers, visit.body),
        // The real edge answers 530 with error 1033 for a tunnel that is
        // not connected.
        Err(err) => (
            530,
            Vec::new(),
            format!("error code: 1033 ({err})").into_bytes(),
        ),
    };
    let mut head = format!("HTTP/1.1 {status} Stand-in\r\n");
    for (name, value) in &answer_headers {
        if ["content-length", "transfer-encoding", "connection"]
            .iter()
            .any(|skip| name.eq_ignore_ascii_case(skip))
        {
            continue;
        }
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str(&format!(
        "Content-Length: {}\r\nConnection: close\r\n\r\n",
        answer.len()
    ));
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(&answer).await?;
    stream.shutdown().await
}
