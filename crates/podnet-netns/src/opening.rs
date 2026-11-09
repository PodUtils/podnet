//! The opening of a proxy client: the first byte and the request, with a
//! time limit.
//!
//! A client that connects and sends nothing, or a part of a greeting, must
//! not hold a task and a socket. The doors of `net proxy`, `net tailscale`
//! and `pod --serve` read the opening with [`read_opening`].

use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, BufReader};

use crate::socks::{self, ProxyRequest};

/// The longest time for the opening of one client.
pub const REQUEST_LIMIT: Duration = Duration::from_secs(30);

/// The result of [`read_opening`].
#[derive(Debug)]
pub enum Opening {
    /// A request. `is_socks` is true for SOCKS5 and false for HTTP.
    Request {
        is_socks: bool,
        request: ProxyRequest,
    },
    /// The client closed, or sent a request that is not valid. The answer,
    /// if any, is sent.
    Closed,
    /// The time limit ended. The answer (SOCKS5 `0x04` or HTTP 504) is sent.
    TimedOut,
}

/// Read the first byte and the request of a client within `limit`. On
/// expiry, answer the client and return [`Opening::TimedOut`].
pub async fn read_opening<S>(reader: &mut BufReader<S>, limit: Duration) -> Opening
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let first = match tokio::time::timeout(limit, reader.fill_buf()).await {
        Ok(Ok(buffer)) if !buffer.is_empty() => buffer[0],
        Ok(_) => return Opening::Closed,
        Err(_) => {
            refuse_slow(reader, false).await;
            return Opening::TimedOut;
        }
    };
    let is_socks = socks::looks_like_socks5(first);
    let read = async {
        if is_socks {
            socks::socks5_read_request(reader).await
        } else {
            socks::http_read_connect(reader).await
        }
    };
    match tokio::time::timeout(limit, read).await {
        Ok(Ok(request)) => Opening::Request { is_socks, request },
        Ok(Err(_)) => Opening::Closed,
        Err(_) => {
            refuse_slow(reader, is_socks).await;
            Opening::TimedOut
        }
    }
}

/// Answer a client that was too slow: SOCKS5 `0x04`, or HTTP 504.
pub async fn refuse_slow<S>(stream: &mut S, is_socks: bool)
where
    S: AsyncWrite + Unpin,
{
    if is_socks {
        let _ = socks::socks5_reply(stream, 0x04, None).await;
    } else {
        let _ = socks::http_reply_with_body(
            stream,
            504,
            "Gateway Timeout",
            "no request from the client in time\n",
        )
        .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn run(send: &[u8]) -> (Vec<u8>, bool) {
        let (mut client, server) = tokio::io::duplex(1024);
        let task = tokio::spawn(async move {
            let mut reader = BufReader::new(server);
            let opening = read_opening(&mut reader, Duration::from_secs(30)).await;
            matches!(opening, Opening::TimedOut)
        });
        client.write_all(send).await.unwrap();
        let mut answer = Vec::new();
        let mut buf = [0u8; 512];
        let read = client.read(&mut buf).await.unwrap();
        answer.extend_from_slice(&buf[..read]);
        (answer, task.await.unwrap())
    }

    #[tokio::test(start_paused = true)]
    async fn no_byte_ends_with_a_504() {
        let (answer, timed_out) = run(b"").await;
        assert!(timed_out);
        assert!(answer.starts_with(b"HTTP/1.1 504"));
    }

    #[tokio::test(start_paused = true)]
    async fn one_socks_byte_ends_with_code_4() {
        let (answer, timed_out) = run(&[0x05]).await;
        assert!(timed_out);
        assert_eq!(answer[0], 0x05);
        assert_eq!(answer[1], 0x04);
    }

    #[tokio::test(start_paused = true)]
    async fn a_connect_head_without_a_blank_line_ends_with_a_504() {
        let (answer, timed_out) = run(b"CONNECT a.b:80 HTTP/1.1\r\nHost: a.b\r\n").await;
        assert!(timed_out);
        assert!(answer.starts_with(b"HTTP/1.1 504"));
    }

    #[tokio::test(start_paused = true)]
    async fn a_complete_request_is_read() {
        let (client, server) = tokio::io::duplex(1024);
        let mut client = client;
        client
            .write_all(b"CONNECT a.b:80 HTTP/1.1\r\n\r\n")
            .await
            .unwrap();
        let mut reader = BufReader::new(server);
        let opening = read_opening(&mut reader, REQUEST_LIMIT).await;
        assert!(matches!(
            opening,
            Opening::Request {
                is_socks: false,
                ..
            }
        ));
    }
}
