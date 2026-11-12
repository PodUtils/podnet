//! The byte meter of a group member (T-178). A [`Metered`] stream counts
//! each byte of a flow, in both directions, as the copy runs, and gives the
//! count to the member that carried the flow. The cost of a chunk is two
//! atomic adds. The decay of the load (`leastload`) is folded in when the
//! pick reads the member.

use std::io::{self, IoSliceMut};
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::time::Instant;

use super::policy::Load;
use super::{BoxedIo, Datagrams, RecvMeta, Transmit};

/// The counters of one member.
#[derive(Default)]
pub struct Meter {
    /// The dials that have not ended (`leastload`).
    pub(super) opens: AtomicU64,
    /// The bytes since the start.
    pub(super) total: AtomicU64,
    /// The bytes that the decayed load has not taken yet.
    pending: AtomicU64,
    load: Mutex<Load>,
}

impl Meter {
    /// Count `bytes` of a flow (the copy path).
    fn note(&self, bytes: u64) {
        self.pending.fetch_add(bytes, Ordering::Relaxed);
        self.total.fetch_add(bytes, Ordering::Relaxed);
    }

    /// Add `bytes` at the time `now` to the decayed load.
    pub(super) fn add(&self, bytes: u64, now: Instant) {
        self.total.fetch_add(bytes, Ordering::Relaxed);
        self.load
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .add(bytes, now);
    }

    /// The decayed load at `now`, with the pending bytes in it.
    pub(super) fn load_at(&self, now: Instant) -> f64 {
        let pending = self.pending.swap(0, Ordering::Relaxed);
        let mut load = self.load.lock().unwrap_or_else(|e| e.into_inner());
        if pending > 0 {
            load.add(pending, now);
        }
        load.at(now)
    }
}

/// An open dial of a member: the count falls when the guard drops.
pub struct OpenGuard(pub(super) Arc<Meter>);

impl Drop for OpenGuard {
    fn drop(&mut self) {
        self.0.opens.fetch_sub(1, Ordering::Relaxed);
    }
}

/// A stream that counts its bytes for a member. The open dial ends at the
/// first byte, in either direction, or when the stream drops.
pub struct Metered {
    inner: BoxedIo,
    meter: Arc<Meter>,
    open: Option<OpenGuard>,
}

impl Metered {
    pub fn new(inner: BoxedIo, meter: Arc<Meter>, open: Option<OpenGuard>) -> Metered {
        Metered { inner, meter, open }
    }

    fn count(&mut self, bytes: usize) {
        if bytes > 0 {
            self.meter.note(bytes as u64);
            self.open = None;
        }
    }
}

impl AsyncRead for Metered {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        let before = buf.filled().len();
        let poll = Pin::new(&mut this.inner).poll_read(cx, buf);
        if let Poll::Ready(Ok(())) = poll {
            this.count(buf.filled().len() - before);
        }
        poll
    }
}

impl AsyncWrite for Metered {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<std::io::Result<usize>> {
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
        bufs: &[std::io::IoSlice<'_>],
    ) -> Poll<std::io::Result<usize>> {
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

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

/// A datagram socket that counts its bytes for a member (T-097), in both
/// directions, as [`Metered`] does for a stream.
#[derive(Debug)]
pub struct MeteredDatagrams {
    inner: Arc<dyn Datagrams>,
    meter: Arc<Meter>,
}

impl MeteredDatagrams {
    pub(super) fn new(inner: Arc<dyn Datagrams>, meter: Arc<Meter>) -> MeteredDatagrams {
        MeteredDatagrams { inner, meter }
    }
}

impl std::fmt::Debug for Meter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Meter")
            .field("total", &self.total.load(Ordering::Relaxed))
            .finish()
    }
}

impl Datagrams for MeteredDatagrams {
    fn poll_writable(&self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.inner.poll_writable(cx)
    }

    fn try_send(&self, transmit: &Transmit<'_>) -> io::Result<()> {
        self.inner.try_send(transmit)?;
        self.meter.note(transmit.contents.len() as u64);
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
            self.meter.note(bytes as u64);
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

#[cfg(test)]
mod tests;
