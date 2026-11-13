//! The service `http` of a stream of a link (T-292).
//!
//! The peer that opened the stream speaks an HTTP proxy on it: one request
//! for each stream, in `CONNECT` form or in absolute form (`GET
//! http://NAME/path HTTP/1.1`). This end runs the front door
//! ([`podnet_netns::door::Door`], the same code as `podnet pod --serve`).
//! The allow list of the listener has admitted the peer before the stream
//! opened.
//!
//! A target is the name of a service of this listener (`--service
//! NAME=TARGET`, `--to` is `raw`). The request port is not used: the name
//! stands for the target of the service. A host that is no such name, and
//! each IP address, is refused with 403. This end does no DNS lookup.
//!
//! Limits: the request head is 8 KiB at most (else 431). A `CONNECT` with a
//! body (`Content-Length` other than 0, or `Transfer-Encoding`) gets 400.
//! A request that is neither `CONNECT` nor absolute form gets 400. The
//! door adds `Connection: close` to a forwarded request and caches
//! nothing.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Result};
use pod_core::{BoxFuture, Class, Connected, Dial, Failure, Stage, Target};
use podnet_netns::door::{Door, REQUEST_LIMIT};
use podnet_netns::socks::http_reply_with_body;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};
use tokio::time::Instant;

/// The name of the service.
pub const NAME: &str = "http";

/// The most bytes of a request head.
pub const HEAD_LIMIT: usize = 8 * 1024;

/// What a service name stands for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Origin {
    /// A TCP host and port.
    Tcp(String, u16),
    /// A unix socket.
    Unix(PathBuf),
}

/// A function from a service name to its origin. It is `None` for a name
/// that is no service that a proxy can reach.
pub type Resolver = Arc<dyn Fn(&str) -> Option<Origin> + Send + Sync>;

/// The way out of the service: the origin of a service name.
struct ByName(Resolver);

impl Dial for ByName {
    fn label(&self) -> String {
        "link http".to_string()
    }

    /// The door sends the name: this end maps it, with no DNS.
    fn names_inside(&self) -> bool {
        true
    }

    fn dial<'a>(
        &'a self,
        target: &'a Target,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(async move {
            let fail = |class, text: String| Failure::new(Stage::Connect, class, text);
            let host = target.host();
            let Some(origin) = (self.0)(&host) else {
                return Err(fail(
                    Class::Policy,
                    format!("{host:?} is no service of this peer"),
                ));
            };
            let limit = deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_secs(10));
            let work = async {
                Ok::<Connected, std::io::Error>(match &origin {
                    Origin::Tcp(host, port) => {
                        let tcp = tokio::net::TcpStream::connect((host.as_str(), *port)).await?;
                        let _ = tcp.set_nodelay(true);
                        let (local, peer) = (tcp.local_addr().ok(), tcp.peer_addr().ok());
                        let mut done = Connected::new(Box::new(tcp));
                        done.local = local;
                        done.peer = peer;
                        done
                    }
                    Origin::Unix(path) => {
                        Connected::new(Box::new(tokio::net::UnixStream::connect(path).await?))
                    }
                })
            };
            match tokio::time::timeout(limit, work).await {
                Err(_) => Err(fail(
                    Class::Timeout,
                    format!("no answer from the service {host:?} in time"),
                )),
                Ok(Err(e)) => Err(fail(
                    Class::Unavailable,
                    format!("connecting to the service {host:?}: {e}"),
                )),
                Ok(Ok(done)) => Ok(done),
            }
        })
    }
}

/// Read the head of a request: up to the first empty line, `HEAD_LIMIT`
/// bytes at most. `None` means the head is too long.
async fn read_head<R: AsyncRead + Unpin>(read: &mut R) -> Result<Option<Vec<u8>>> {
    let mut head = Vec::with_capacity(512);
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() >= HEAD_LIMIT {
            return Ok(None);
        }
        if read.read(&mut byte).await? == 0 {
            bail!("the stream ended before the request head was complete");
        }
        head.push(byte[0]);
    }
    Ok(Some(head))
}

/// The reason to refuse a head with 400, or `None`.
fn bad_head(head: &[u8]) -> Option<&'static str> {
    let text = String::from_utf8_lossy(head);
    let mut lines = text.split("\r\n");
    let method = lines.next()?.split_whitespace().next()?;
    if !method.eq_ignore_ascii_case("CONNECT") {
        return None;
    }
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim();
        if name.eq_ignore_ascii_case("transfer-encoding")
            || (name.eq_ignore_ascii_case("content-length") && value.trim() != "0")
        {
            return Some("a CONNECT request has no body\n");
        }
    }
    None
}

/// Serve one HTTP proxy request on `stream`. `resolve` maps a name to an
/// origin.
pub async fn serve<S>(stream: S, resolve: Resolver) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    let (mut read, mut write) = tokio::io::split(stream);
    let head = tokio::time::timeout(REQUEST_LIMIT, read_head(&mut read)).await??;
    let Some(head) = head else {
        http_reply_with_body(
            &mut write,
            431,
            "Request Header Fields Too Large",
            "the request head is 8 KiB at most\n",
        )
        .await?;
        bail!("the request head of the service {NAME} is over {HEAD_LIMIT} bytes");
    };
    if let Some(why) = bad_head(&head) {
        http_reply_with_body(&mut write, 400, "Bad Request", why).await?;
        bail!("the service {NAME} got a bad request: {}", why.trim());
    }
    // The door reads the head again: give it back in front of the stream.
    let again = tokio::io::join(std::io::Cursor::new(head).chain(read), write);
    Door::new(Arc::new(ByName(resolve))).handle(again).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_connect_with_a_body_is_bad() {
        let head = b"CONNECT a:1 HTTP/1.1\r\nContent-Length: 5\r\n\r\n";
        assert!(bad_head(head).is_some());
        let head = b"CONNECT a:1 HTTP/1.1\r\ntransfer-encoding: chunked\r\n\r\n";
        assert!(bad_head(head).is_some());
        let head = b"CONNECT a:1 HTTP/1.1\r\nContent-Length: 0\r\n\r\n";
        assert!(bad_head(head).is_none());
        let head = b"POST http://a/ HTTP/1.1\r\nContent-Length: 5\r\n\r\n";
        assert!(bad_head(head).is_none());
    }
}
