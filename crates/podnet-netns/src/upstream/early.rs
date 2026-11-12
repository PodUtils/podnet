//! The early SOCKS5 handshake of `--early-reply` (T-237): the greeting and
//! the request go in one write, and the last hop can return before its two
//! replies, so that the bytes of the client follow the request.

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use anyhow::{bail, Result};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _, ReadBuf};

use super::{
    check_socks5_credentials, read_socks5_reply, socks5_connect, socks5_request, Proxy, Upstream,
    UPSTREAM_TIMEOUT,
};

/// The order of the SOCKS5 handshake of one hop.
#[derive(Clone, Default)]
pub enum Handshake {
    /// The greeting, its reply, the request, its reply: each in turn.
    #[default]
    Serial,
    /// The greeting and the request in one write, then the two replies.
    /// No byte of the client has left the host, so a failed attempt can go
    /// to the next route. A hop with a user and a password stays serial: it
    /// needs the method reply before the next bytes.
    Pipelined,
    /// As `Pipelined`, but the stream returns at once. The two replies are
    /// read at the first read of the stream, so the bytes of the client can
    /// follow the request. The hook gets the code of a refusal. Only the
    /// last hop of a chain can be optimistic; the hops in front are
    /// pipelined.
    Optimistic(RefusedHook),
}

/// A function that gets the reply code of an upstream that refuses a flow.
pub type RefusedHook = Arc<dyn Fn(u8) + Send + Sync>;

impl Handshake {
    /// The order for a hop whose reply must be read before the next bytes.
    pub(super) fn confirmed(&self) -> Handshake {
        match self {
            Handshake::Serial => Handshake::Serial,
            _ => Handshake::Pipelined,
        }
    }
}

/// An upstream that answered a SOCKS5 request with a code other than 0.
#[derive(Debug)]
pub struct UpstreamRefused {
    pub target: String,
    pub code: u8,
}

impl std::fmt::Display for UpstreamRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "SOCKS5 connect to {} failed: code {}",
            self.target, self.code
        )
    }
}

impl std::error::Error for UpstreamRefused {}

/// One SOCKS5 hop over `stream`, in the order of `handshake`. A hop with a
/// user and a password is serial.
pub async fn socks5_hop(
    mut stream: Upstream,
    proxy: &Proxy,
    target: &str,
    handshake: Handshake,
) -> Result<Upstream> {
    if proxy.user.is_some() || matches!(handshake, Handshake::Serial) {
        return socks5_connect(stream, proxy, target).await;
    }
    check_socks5_credentials(None, proxy.pass.as_deref())?;
    let mut flight = vec![0x05, 0x01, 0x00];
    flight.extend_from_slice(&socks5_request(target)?);
    stream.write_all(&flight).await?;
    if let Handshake::Optimistic(hook) = handshake {
        return Ok(Box::new(Unconfirmed::new(stream, target, hook)));
    }
    let mut method = [0u8; 2];
    stream.read_exact(&mut method).await?;
    if method != [0x05, 0x00] {
        bail!("proxy refused SOCKS5 methods");
    }
    read_socks5_reply(&mut stream, target).await?;
    Ok(stream)
}

/// What is wrong with the replies of an upstream.
enum Fault {
    Methods,
    Refused(u8),
    Address(u8),
}

/// The length that the replies need, from the bytes read so far: `Some`
/// when more bytes follow, `None` when the replies are complete. The two
/// replies are the method reply (2 bytes) and the reply of the request.
fn reply_step(got: &[u8]) -> Result<Option<usize>, Fault> {
    match got.len() {
        2 if got != [0x05, 0x00] => Err(Fault::Methods),
        2 => Ok(Some(6)),
        6 if got[3] != 0 => Err(Fault::Refused(got[3])),
        6 => match got[5] {
            0x01 => Ok(Some(12)),
            0x04 => Ok(Some(24)),
            0x03 => Ok(Some(7)),
            other => Err(Fault::Address(other)),
        },
        7 if got[5] == 0x03 => Ok(Some(7 + got[6] as usize + 2)),
        _ => Ok(None),
    }
}

enum State {
    Waiting,
    Ready,
    Failed(io::ErrorKind, String),
}

/// A SOCKS5 upstream whose two replies are not read yet. A write goes to
/// the upstream at once. A read first reads and checks the two replies, in
/// the time limit of an upstream. A refusal is an error of the read.
struct Unconfirmed {
    inner: Upstream,
    target: String,
    hook: RefusedHook,
    got: Vec<u8>,
    want: usize,
    state: State,
    limit: Pin<Box<tokio::time::Sleep>>,
}

impl Unconfirmed {
    fn new(inner: Upstream, target: &str, hook: RefusedHook) -> Self {
        Unconfirmed {
            inner,
            target: target.to_string(),
            hook,
            got: Vec::new(),
            want: 2,
            state: State::Waiting,
            limit: Box::pin(tokio::time::sleep(UPSTREAM_TIMEOUT)),
        }
    }

    fn fail(&mut self, kind: io::ErrorKind, text: String) {
        self.state = State::Failed(kind, text);
    }

    /// Check the bytes that were read, when they are the wanted length.
    fn step(&mut self) {
        match reply_step(&self.got) {
            Ok(Some(want)) => self.want = want,
            Ok(None) => self.state = State::Ready,
            Err(Fault::Methods) => self.fail(
                io::ErrorKind::ConnectionRefused,
                "proxy refused SOCKS5 methods".to_string(),
            ),
            Err(Fault::Refused(code)) => {
                (self.hook)(code);
                let text = UpstreamRefused {
                    target: self.target.clone(),
                    code,
                }
                .to_string();
                self.fail(io::ErrorKind::ConnectionRefused, text);
            }
            Err(Fault::Address(atyp)) => self.fail(
                io::ErrorKind::InvalidData,
                format!("bad SOCKS5 address type {atyp}"),
            ),
        }
    }
}

impl AsyncRead for Unconfirmed {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = &mut *self;
        loop {
            match &this.state {
                State::Ready => return Pin::new(&mut this.inner).poll_read(cx, buf),
                State::Failed(kind, text) => {
                    return Poll::Ready(Err(io::Error::new(*kind, text.clone())))
                }
                State::Waiting => {}
            }
            if this.limit.as_mut().poll(cx).is_ready() {
                let text = format!("the upstream sent no reply in {UPSTREAM_TIMEOUT:?}");
                this.fail(io::ErrorKind::TimedOut, text);
                continue;
            }
            // Read the wanted bytes and no more: the bytes after the replies
            // are data for the reader.
            let mut scratch = [0u8; 24];
            let room = this.want - this.got.len();
            let mut part = ReadBuf::new(&mut scratch[..room]);
            match Pin::new(&mut this.inner).poll_read(cx, &mut part) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Ready(Ok(())) if part.filled().is_empty() => {
                    this.fail(
                        io::ErrorKind::UnexpectedEof,
                        "the upstream closed before its reply".to_string(),
                    );
                }
                Poll::Ready(Ok(())) => {
                    this.got.extend_from_slice(part.filled());
                    if this.got.len() == this.want {
                        this.step();
                    }
                }
            }
        }
    }
}

impl AsyncWrite for Unconfirmed {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, data)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

#[cfg(test)]
#[path = "early_tests.rs"]
mod tests;
