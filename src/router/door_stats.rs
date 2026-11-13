//! The counters of a door (T-327): the flows and the bytes that came through
//! it. [`Counting`] wraps the dial of a door and counts, so that a door has
//! its own numbers whatever member or network carries the flow.

use std::io::{self, IoSliceMut};
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::time::Instant;

use crate::network::{BoxFuture, Connected, Datagrams, Dial, Failure, RecvMeta, Target, Transmit};

/// The counters of one door.
#[derive(Debug, Default)]
pub struct DoorStats {
    flows: AtomicU64,
    open: AtomicU64,
    bytes: AtomicU64,
}

impl DoorStats {
    /// The flows that the door dialed, and the UDP associations that it opened.
    pub fn flows(&self) -> u64 {
        self.flows.load(Ordering::Relaxed)
    }

    /// The flows that have not ended.
    pub fn open(&self) -> u64 {
        self.open.load(Ordering::Relaxed)
    }

    /// The bytes of TCP and UDP, both directions.
    pub fn bytes(&self) -> u64 {
        self.bytes.load(Ordering::Relaxed)
    }

    /// The counters as a status document.
    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({"flows": self.flows(), "open": self.open(), "bytes": self.bytes()})
    }

    fn start(self: &Arc<Self>) -> Open {
        self.flows.fetch_add(1, Ordering::Relaxed);
        self.open.fetch_add(1, Ordering::Relaxed);
        Open(self.clone())
    }
}

/// One open flow: the count falls when the flow ends.
struct Open(Arc<DoorStats>);

impl Drop for Open {
    fn drop(&mut self) {
        self.0.open.fetch_sub(1, Ordering::Relaxed);
    }
}

/// A dial that counts the flows and the bytes of a door.
pub struct Counting {
    inner: Arc<dyn Dial>,
    stats: Arc<DoorStats>,
}

impl Counting {
    pub fn new(inner: Arc<dyn Dial>, stats: Arc<DoorStats>) -> Counting {
        Counting { inner, stats }
    }
}

impl Dial for Counting {
    fn label(&self) -> String {
        self.inner.label()
    }

    fn dial<'a>(
        &'a self,
        target: &'a Target,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(async move {
            let mut connected = self.inner.dial(target, deadline).await?;
            connected.io = Box::new(CountedIo {
                inner: connected.io,
                stats: self.stats.clone(),
                _open: self.stats.start(),
            });
            Ok(connected)
        })
    }

    fn names_inside(&self) -> bool {
        self.inner.names_inside()
    }

    fn datagrams<'a>(&'a self) -> BoxFuture<'a, Result<Arc<dyn Datagrams>, Failure>> {
        Box::pin(async move {
            let inner = self.inner.datagrams().await?;
            Ok(Arc::new(CountedDatagrams {
                inner,
                stats: self.stats.clone(),
                _open: self.stats.start(),
            }) as Arc<dyn Datagrams>)
        })
    }
}

struct CountedIo {
    inner: crate::network::BoxedIo,
    stats: Arc<DoorStats>,
    _open: Open,
}

impl CountedIo {
    fn count(&self, bytes: usize) {
        self.stats.bytes.fetch_add(bytes as u64, Ordering::Relaxed);
    }
}

impl AsyncRead for CountedIo {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let before = buf.filled().len();
        let poll = Pin::new(&mut this.inner).poll_read(cx, buf);
        if let Poll::Ready(Ok(())) = poll {
            this.count(buf.filled().len() - before);
        }
        poll
    }
}

impl AsyncWrite for CountedIo {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let poll = Pin::new(&mut this.inner).poll_write(cx, data);
        if let Poll::Ready(Ok(n)) = poll {
            this.count(n);
        }
        poll
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let poll = Pin::new(&mut this.inner).poll_write_vectored(cx, bufs);
        if let Poll::Ready(Ok(n)) = poll {
            this.count(n);
        }
        poll
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

struct CountedDatagrams {
    inner: Arc<dyn Datagrams>,
    stats: Arc<DoorStats>,
    _open: Open,
}

impl std::fmt::Debug for CountedDatagrams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CountedDatagrams").finish_non_exhaustive()
    }
}

impl Datagrams for CountedDatagrams {
    fn poll_writable(&self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.inner.poll_writable(cx)
    }

    fn try_send(&self, transmit: &Transmit<'_>) -> io::Result<()> {
        self.inner.try_send(transmit)?;
        self.stats
            .bytes
            .fetch_add(transmit.contents.len() as u64, Ordering::Relaxed);
        Ok(())
    }

    fn poll_recv(
        &self,
        cx: &mut Context<'_>,
        bufs: &mut [IoSliceMut<'_>],
        meta: &mut [RecvMeta],
    ) -> Poll<io::Result<usize>> {
        let poll = self.inner.poll_recv(cx, bufs, meta);
        if let Poll::Ready(Ok(n)) = &poll {
            let bytes: usize = meta.iter().take(*n).map(|m| m.len).sum();
            self.stats.bytes.fetch_add(bytes as u64, Ordering::Relaxed);
        }
        poll
    }

    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.inner.local_addr()
    }

    fn max_transmit_segments(&self) -> usize {
        self.inner.max_transmit_segments()
    }

    fn max_receive_segments(&self) -> usize {
        self.inner.max_receive_segments()
    }

    fn may_fragment(&self) -> bool {
        self.inner.may_fragment()
    }
}
