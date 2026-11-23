//! The session's stream on a TUIC connection: the bidirectional QUIC stream
//! that `Connect` opened, its first bytes kept once read, a `Heartbeat`
//! datagram on a timer while it lives, and the connection and its endpoint
//! closed when it goes. QUIC streams half-close, so the end of a pipe's input
//! finishes the way up and the target's bytes still come.
//!
//! This is `podssh-ws/src/tuic/stream.rs`. One adaptation: the road does not
//! advertise QUIC DATAGRAM support (the reference server would then reply to
//! UDP with unreliable datagrams; see `mod.rs`), so the heartbeat task ends
//! at once when the connection has no datagram size. QUIC's own keepalive,
//! set by `Settings`, keeps the connection alive.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use pod_quic::noq::{
    Connection, ConnectionError, Endpoint, ReadError, RecvStream, SendStream, VarInt,
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::task::JoinHandle;

use super::command::HEARTBEAT_DATAGRAM;

/// sing-box's default heartbeat, and the wait that its server and
/// Itsusinn's take before a silent connection is reaped.
pub const HEARTBEAT: Duration = Duration::from_secs(10);
/// How long the close of a connection may take to leave, before its
/// endpoint goes.
const CLOSE_WAIT: Duration = Duration::from_millis(500);

/// How the first read of the stream ended.
pub enum First {
    /// The target's first bytes came.
    Bytes,
    /// The server finished or reset the stream with no byte: it could not
    /// reach the target, or the connection is closing.
    Ended,
    /// The connection itself ended.
    Lost(ConnectionError),
    /// Another failure of the read.
    Failed(String),
}

/// The session's stream.
pub struct TuicStream {
    endpoint: Endpoint,
    connection: Connection,
    send: SendStream,
    recv: RecvStream,
    /// Bytes read ahead by [`TuicStream::first`], and how many are given.
    ahead: Vec<u8>,
    given: usize,
    heartbeat: JoinHandle<()>,
}

impl TuicStream {
    /// The stream, and a `Heartbeat` each [`HEARTBEAT`] on `connection`.
    pub fn new(
        endpoint: Endpoint,
        connection: Connection,
        send: SendStream,
        recv: RecvStream,
    ) -> TuicStream {
        let heartbeat = tokio::spawn(beat(connection.clone()));
        TuicStream {
            endpoint,
            connection,
            send,
            recv,
            ahead: Vec::new(),
            given: 0,
            heartbeat,
        }
    }

    pub fn connection(&self) -> &Connection {
        &self.connection
    }

    /// The first read: the server never answers a command, so the first
    /// bytes, or the end with none, are all that tells what it did.
    pub async fn first(&mut self) -> First {
        let mut buf = vec![0u8; 16 * 1024];
        match self.recv.read(&mut buf).await {
            Ok(Some(n)) => {
                buf.truncate(n);
                self.ahead = buf;
                First::Bytes
            }
            Ok(None) | Err(ReadError::Reset(_)) => First::Ended,
            Err(ReadError::ConnectionLost(e)) => First::Lost(e),
            Err(e) => First::Failed(e.to_string()),
        }
    }
}

/// A `Heartbeat` datagram on each tick, until the connection ends or takes
/// none. The road advertises no DATAGRAM, so a server that sees no
/// capability cannot answer with one; QUIC's own keepalive still runs.
async fn beat(connection: Connection) {
    if connection.max_datagram_size().is_none() {
        return;
    }
    let mut tick = tokio::time::interval_at(tokio::time::Instant::now() + HEARTBEAT, HEARTBEAT);
    loop {
        tick.tick().await;
        if connection
            .send_datagram(bytes::Bytes::from_static(&HEARTBEAT_DATAGRAM))
            .is_err()
        {
            return;
        }
    }
}

impl AsyncRead for TuicStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if this.given < this.ahead.len() {
            let n = (this.ahead.len() - this.given).min(buf.remaining());
            buf.put_slice(&this.ahead[this.given..this.given + n]);
            this.given += n;
            if this.given == this.ahead.len() {
                this.ahead = Vec::new();
                this.given = 0;
            }
            return Poll::Ready(Ok(()));
        }
        // The trait's, by name: noq's own `poll_read` takes a slice.
        AsyncRead::poll_read(Pin::new(&mut this.recv), cx, buf)
    }
}

impl AsyncWrite for TuicStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        // The trait's, by name: noq's own `poll_write` gives its own error.
        AsyncWrite::poll_write(Pin::new(&mut self.get_mut().send), cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        AsyncWrite::poll_flush(Pin::new(&mut self.get_mut().send), cx)
    }

    /// FIN on the way up; the way down stays open.
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        AsyncWrite::poll_shutdown(Pin::new(&mut self.get_mut().send), cx)
    }
}

impl Drop for TuicStream {
    fn drop(&mut self) {
        self.heartbeat.abort();
        // One connection carries one session, so the session's end is the
        // connection's: code 0, as TUIC's clients close.
        self.connection.close(VarInt::from_u32(0), b"");
        let endpoint = self.endpoint.clone();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let _ = tokio::time::timeout(CLOSE_WAIT, endpoint.wait_idle()).await;
            });
        }
    }
}
