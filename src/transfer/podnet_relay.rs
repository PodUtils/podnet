//! The podnet relay as a way of the transit (T-104). It is a podnet
//! extension of the transit of magic-wormhole, for two podnet ends that
//! reach no direct hint and no transit relay (for example two hosts behind
//! proxies that pass HTTPS only).
//!
//! * The sender runs the node of a stored pair of the reverse road
//!   (`podnet relay pair`) for the time of the transfer ([`start`]), with
//!   the resumable layer of `pod-session` (T-087) on each session. Its hint
//!   is [`Hint::PodnetRelay`] (the URL of the relay and the name of the
//!   pair). The connect token goes in its `transit` message, which the
//!   wormhole seals.
//! * The receiver dials the node as an operator (`dial`), with the layer:
//!   the session moves to a new leg before the byte cap of the relay, and
//!   resumes after the loss of a leg. A node that is not online yet is
//!   dialed again after [`RETRY`].
//! * On the stream, the two ends run the handshake of the transit
//!   ([`super::connect`]), as on a direct connection. The relay sees only
//!   records that the transit key seals.
//! * A client of magic-wormhole ignores the hint, as it ignores each kind
//!   of hint that it does not know. podnet sends a file only to podnet.

use std::future::Future;
use std::io;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use pod_core::BoxedIo;
use tokio::io::{AsyncRead, AsyncWrite, DuplexStream, ReadBuf};
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;

use super::messages::Hint;
use super::transit::Role;
use crate::relay_reverse::{self, Resume, SessionId};
use crate::ws_relay::Relay;

/// The wait before a new dial of a node that did not answer.
pub const RETRY: Duration = Duration::from_secs(1);
/// How long the receiver waits, after the end of its records, for the
/// session of the layer to end.
pub const END_WAIT: Duration = Duration::from_secs(15);
/// How long a sender waits, after its ack, for the end of the receiver
/// before its node stops.
pub const GRACE: Duration = Duration::from_secs(2);
/// The sessions of a node that wait for the transit.
const DEPTH: usize = 4;
/// The bytes between the layer and the transit, in each direction.
const PIPE: usize = 256 * 1024;

/// A stored pair of the podnet relay that a sender offers: the directory
/// of the pairs and the name of the pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayPair {
    pub dir: PathBuf,
    pub name: String,
}

/// The sessions that the node of a sender takes.
pub type Sessions = mpsc::Receiver<(SessionId, DuplexStream)>;

/// The node of a sender. It runs while this value lives.
pub struct Node {
    task: JoinHandle<anyhow::Result<()>>,
    /// The hint of the node.
    pub hint: Hint,
    /// The connect token of the pair, for the sealed `transit` message.
    pub token: String,
}

impl Drop for Node {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Load the pair and start its node, with the layer on each session.
pub fn start(pair: &RelayPair) -> anyhow::Result<(Node, Sessions)> {
    let pair = relay_reverse::load(&pair.dir, Some(&pair.name))?;
    let hint = Hint::PodnetRelay {
        url: pair.relay.clone(),
        name: pair.name.clone(),
    };
    let token = pair.connect_token().to_string();
    let (handler, sessions) = relay_reverse::channel(DEPTH);
    let handler = Arc::new(relay_reverse::layered(handler));
    let task = tokio::spawn(relay_reverse::serve_pair(pair, handler, "the transfer"));
    Ok((Node { task, hint, token }, sessions))
}

/// Dial the node `name` at the relay `url` with `token`, again after each
/// [`RETRY`], until a stream passes the handshake of the transit. The
/// stream and what it is (`podnet relay NAME`).
pub(super) async fn dial(
    url: String,
    name: String,
    token: String,
    resume: Resume,
    role: Role,
    key: [u8; 32],
    won: Arc<Mutex<bool>>,
) -> Option<(BoxedIo, String)> {
    let relay = Relay::single(&url, None).ok()?;
    loop {
        let (mine, app) = tokio::io::duplex(PIPE);
        let (relay, name_of, token) = (relay.clone(), name.clone(), token.clone());
        let task = tokio::spawn(async move {
            let (read, write) = tokio::io::split(app);
            relay_reverse::connect_resumable(&relay, &name_of, &token, read, write, resume).await
        });
        let carried = Carried {
            io: mine,
            task: Some(task),
            end: None,
        };
        if let Some(io) = super::connect::handshake(Box::new(carried), role, key, won.clone()).await
        {
            return Some((io, format!("podnet relay {name}")));
        }
        tokio::time::sleep(RETRY).await;
    }
}

/// The stream of a receiver over the layer: at its shutdown, it waits at
/// most [`END_WAIT`] for the session of the layer to end, so that its last
/// bytes reach the node. A drop stops the session.
struct Carried {
    io: DuplexStream,
    task: Option<JoinHandle<anyhow::Result<()>>>,
    end: Option<Pin<Box<tokio::time::Sleep>>>,
}

impl Drop for Carried {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

impl AsyncRead for Carried {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.io).poll_read(cx, buf)
    }
}

impl AsyncWrite for Carried {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
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
        let Some(task) = this.task.as_mut() else {
            return Poll::Ready(Ok(()));
        };
        if Pin::new(task).poll(cx).is_ready() {
            this.task = None;
            return Poll::Ready(Ok(()));
        }
        // The end of the bytes of the sender: it closes after it read the
        // last record of this end, so these bytes reached it.
        let mut byte = [0u8; 1];
        let mut buf = ReadBuf::new(&mut byte);
        if let Poll::Ready(read) = Pin::new(&mut this.io).poll_read(cx, &mut buf) {
            if read.is_err() || buf.filled().is_empty() {
                return Poll::Ready(Ok(()));
            }
        }
        let end = this
            .end
            .get_or_insert_with(|| Box::pin(tokio::time::sleep(END_WAIT)));
        if end.as_mut().poll(cx).is_ready() {
            if let Some(task) = this.task.take() {
                task.abort();
            }
            return Poll::Ready(Ok(()));
        }
        Poll::Pending
    }
}
