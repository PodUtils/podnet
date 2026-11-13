//! A piping server as the last way of the transit (T-252). It is a podnet
//! extension of the transit of magic-wormhole, for two podnet ends that
//! reach no direct hint, no transit relay and no podnet relay, but reach
//! HTTPS (for example two hosts behind proxies that pass HTTPS only).
//!
//! * A piping server gives the body of a request `PUT /PATH` to the one
//!   request `GET /PATH` on the same path. Each end gives its servers as
//!   hints of the kind [`PIPING`] (`url`). The two ends use the first
//!   server of the sender that the receiver names too: with no such
//!   server, the transfer does not use a piping server.
//! * The base path is the first 32 hex digits of HKDF-SHA256 of the
//!   transit key with the info [`PATH_INFO`]. Only the two ends know it.
//! * The bytes go in numbered pieces, not in one stream: a server or a
//!   proxy can give the body of a request only at its end. The bytes of
//!   the sender go to `/BASE/s/N`, the bytes of the receiver to
//!   `/BASE/r/N`, N from 0. Each piece is at most [`PIECE`] bytes. An
//!   empty piece is the end of the bytes of that end.
//! * On the stream, the two ends run the handshake of the transit
//!   ([`super::connect`]), as on a direct connection. The server sees only
//!   the handshakes and the records that the transit key seals.

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use pod_core::{BoxedIo, Class};
use pod_proxy::client::Request;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, DuplexStream, ReadBuf};
use tokio::task::JoinHandle;

use super::connect::Way;
use super::messages::Hint;
pub use super::messages::PIPING;
use super::transit::{self, Role};

/// The info of the HKDF of the base path.
pub const PATH_INFO: &[u8] = b"podnet piping path v1";
/// The most bytes of one piece.
pub const PIECE: usize = 256 * 1024;
/// The bound of one request. A request that times out goes again: the
/// other end did not send its piece yet.
pub const REQUEST: Duration = Duration::from_secs(60);
/// The failures in a row (not time-outs) that end a direction.
pub const TRIES: u32 = 5;
/// The wait after a failure.
const AGAIN: Duration = Duration::from_secs(1);
/// The time to gather more bytes into one piece.
const GATHER: Duration = Duration::from_millis(5);
/// How long the stream waits, at its shutdown, for its last piece.
pub const END_WAIT: Duration = Duration::from_secs(15);
/// The bytes between the pieces and the transit, in each direction.
const PIPE: usize = 2 * PIECE;

/// The server of the transit: the first piping server of the sender that
/// the receiver names too.
pub fn common(role: Role, ours: &[Hint], theirs: &[Hint]) -> Option<String> {
    let servers = |hints: &[Hint]| -> Vec<String> {
        hints
            .iter()
            .filter_map(|h| match h {
                Hint::Piping { url } => Some(url.clone()),
                _ => None,
            })
            .collect()
    };
    let (sender, receiver) = match role {
        Role::Sender => (servers(ours), servers(theirs)),
        Role::Receiver => (servers(theirs), servers(ours)),
    };
    sender.into_iter().find(|url| receiver.contains(url))
}

/// The base path of the transit of `key`: `/` and 32 hex digits.
pub fn base(key: &[u8; 32]) -> String {
    let hex = pod_wormhole::hex(&transit::derive(key, PATH_INFO));
    format!("/{}", &hex[..32])
}

/// The URL of a piece: the server, the base path, the end, the number.
fn piece_url(server: &str, base: &str, end: char, n: u64) -> String {
    format!("{}{base}/{end}/{n}", server.trim_end_matches('/'))
}

/// A stream to the other end through the piping server `server`. The
/// requests go through the way of the transfer.
pub fn open(server: &str, key: &[u8; 32], role: Role, way: &Way) -> BoxedIo {
    let client = pod_proxy::Client::new(way.way.tls.clone(), way.way.dial.clone())
        .with_user_agent(way.way.user_agent.clone());
    let link = Arc::new(Link {
        client,
        trust: way.trust.clone(),
        server: server.to_string(),
        base: base(key),
    });
    let (mine, app) = tokio::io::duplex(PIPE);
    let (from, to) = tokio::io::split(app);
    let (out, back) = match role {
        Role::Sender => ('s', 'r'),
        Role::Receiver => ('r', 's'),
    };
    let up = tokio::spawn(up(link.clone(), out, from));
    let down = tokio::spawn(down(link, back, to));
    Box::new(Piped {
        io: mine,
        up: Some(up),
        down,
        end: None,
    })
}

struct Link {
    client: pod_proxy::Client,
    trust: pod_ws::Trust,
    server: String,
    base: String,
}

impl Link {
    /// One request until it passes: a time-out goes again with no count; a
    /// failure goes again after [`AGAIN`], [`TRIES`] times in a row at most.
    async fn exchange(&self, method: &'static str, url: &str, body: &[u8]) -> io::Result<Vec<u8>> {
        let mut failures = 0;
        loop {
            let mut request = Request::new(method, url)
                .timeout(REQUEST)
                .trust(self.trust.clone())
                .max_body(PIECE + 1024);
            if method == "PUT" {
                request = request
                    .header("Content-Type", "application/octet-stream")
                    .body(body.to_vec());
            }
            let why = match self.client.send(request).await {
                Ok(answer) if (200..300).contains(&answer.status) => return Ok(answer.body),
                Ok(answer) => format!("the piping server answered {}", answer.status),
                Err(failure) if failure.class == Class::Timeout => continue,
                Err(failure) => failure.to_string(),
            };
            failures += 1;
            if failures >= TRIES {
                return Err(io::Error::other(why));
            }
            tokio::time::sleep(AGAIN).await;
        }
    }
}

/// The bytes of this end: the pieces `end`/0, 1, ... and, at the end of the
/// bytes, an empty piece.
async fn up(
    link: Arc<Link>,
    end: char,
    mut from: tokio::io::ReadHalf<DuplexStream>,
) -> io::Result<()> {
    let mut buf = vec![0u8; PIECE];
    for n in 0.. {
        let mut got = from.read(&mut buf).await?;
        while got > 0 && got < PIECE {
            match tokio::time::timeout(GATHER, from.read(&mut buf[got..])).await {
                Ok(Ok(more)) if more > 0 => got += more,
                _ => break,
            }
        }
        let url = piece_url(&link.server, &link.base, end, n);
        link.exchange("PUT", &url, &buf[..got]).await?;
        if got == 0 {
            break;
        }
    }
    Ok(())
}

/// The bytes of the other end, piece by piece, until an empty piece.
async fn down(
    link: Arc<Link>,
    end: char,
    mut to: tokio::io::WriteHalf<DuplexStream>,
) -> io::Result<()> {
    let mut done = Ok(());
    for n in 0.. {
        let url = piece_url(&link.server, &link.base, end, n);
        match link.exchange("GET", &url, &[]).await {
            Ok(body) if body.is_empty() => break,
            Ok(body) => {
                if let Err(err) = to.write_all(&body).await {
                    done = Err(err);
                    break;
                }
            }
            Err(err) => {
                done = Err(err);
                break;
            }
        }
    }
    let _ = to.shutdown().await;
    done
}

/// The stream of an end: a write fails when the pieces of this end
/// failed; at its shutdown it waits at most [`END_WAIT`] for its last
/// piece. A drop stops the requests.
struct Piped {
    io: DuplexStream,
    up: Option<JoinHandle<io::Result<()>>>,
    down: JoinHandle<io::Result<()>>,
    end: Option<Pin<Box<tokio::time::Sleep>>>,
}

impl Drop for Piped {
    fn drop(&mut self) {
        if let Some(up) = self.up.take() {
            up.abort();
        }
        self.down.abort();
    }
}

impl AsyncRead for Piped {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.io).poll_read(cx, buf)
    }
}

impl AsyncWrite for Piped {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.up.as_ref().is_none_or(|up| up.is_finished()) {
            let why = "the pieces to the piping server failed";
            return Poll::Ready(Err(io::Error::new(io::ErrorKind::BrokenPipe, why)));
        }
        Pin::new(&mut self.io).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.io).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = &mut *self;
        if Pin::new(&mut this.io).poll_shutdown(cx).is_pending() {
            return Poll::Pending;
        }
        let Some(up) = this.up.as_mut() else {
            return Poll::Ready(Ok(()));
        };
        if let Poll::Ready(done) = Pin::new(up).poll(cx) {
            this.up = None;
            return Poll::Ready(match done {
                Ok(Ok(())) => Ok(()),
                Ok(Err(err)) => Err(err),
                Err(_) => Err(io::Error::other("the pieces stopped")),
            });
        }
        let end = this
            .end
            .get_or_insert_with(|| Box::pin(tokio::time::sleep(END_WAIT)));
        if end.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "the last piece did not reach the other end",
            )));
        }
        Poll::Pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transit_over_piping_server_the_base_path_is_hkdf_of_the_transit_key() {
        let key = [3u8; 32];
        let path = base(&key);
        assert_eq!(path.len(), 33);
        assert_eq!(
            &path[1..],
            &pod_wormhole::hex(&transit::derive(&key, b"podnet piping path v1"))[..32]
        );
        assert_ne!(path, base(&[4u8; 32]));
        assert_eq!(
            piece_url("https://p.example/", &path, 's', 7),
            format!("https://p.example{path}/s/7")
        );
    }

    #[test]
    fn transit_over_piping_server_the_server_is_the_first_of_the_sender_that_both_name() {
        let hint = |u: &str| Hint::Piping { url: u.into() };
        let sender = [hint("https://a.example"), hint("https://b.example")];
        let receiver = [hint("https://b.example"), hint("https://a.example")];
        let only_b = [hint("https://b.example")];
        assert_eq!(
            common(Role::Sender, &sender, &receiver).as_deref(),
            Some("https://a.example")
        );
        assert_eq!(
            common(Role::Receiver, &receiver, &sender).as_deref(),
            Some("https://a.example")
        );
        assert_eq!(
            common(Role::Receiver, &only_b, &sender).as_deref(),
            Some("https://b.example")
        );
        assert_eq!(common(Role::Sender, &sender, &[]), None);
        assert_eq!(common(Role::Receiver, &[], &sender), None);
    }
}
