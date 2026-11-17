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

use super::door_limit::{Limiter, Way};
use crate::network::{
    BoxFuture, Class, Connected, Datagrams, Dial, Failure, RecvMeta, Stage, Target, Transmit,
};

/// The counters of one door.
#[derive(Debug, Default)]
pub struct DoorStats {
    flows: AtomicU64,
    open: AtomicU64,
    bytes: AtomicU64,
    /// The rate and the quota of the door (T-405).
    limiter: Option<Arc<Limiter>>,
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

    /// Counters with the limits of `limiter`.
    pub fn limited(limiter: Arc<Limiter>) -> DoorStats {
        DoorStats {
            limiter: Some(limiter),
            ..DoorStats::default()
        }
    }

    pub fn limiter(&self) -> Option<&Arc<Limiter>> {
        self.limiter.as_ref()
    }

    /// The counters as a status document, with `limits` when the door has
    /// a rate or a quota.
    pub fn json(&self) -> serde_json::Value {
        let mut v =
            serde_json::json!({"flows": self.flows(), "open": self.open(), "bytes": self.bytes()});
        if let Some(limiter) = &self.limiter {
            v["limits"] = limiter.json();
        }
        v
    }

    fn add(&self, bytes: usize) {
        self.bytes.fetch_add(bytes as u64, Ordering::Relaxed);
        if let Some(limiter) = &self.limiter {
            limiter.count(bytes as u64);
        }
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

    /// A door at its quota refuses each new flow (T-405).
    fn refuse_at_quota(&self) -> Result<(), Failure> {
        match &self.stats.limiter {
            Some(l) if l.exhausted() => Err(Failure::new(
                Stage::Connect,
                Class::Policy,
                "the quota of the door is used for this period",
            )),
            _ => Ok(()),
        }
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
            self.refuse_at_quota()?;
            let mut connected = self.inner.dial(target, deadline).await?;
            connected.io = Box::new(CountedIo {
                inner: connected.io,
                stats: self.stats.clone(),
                _open: self.stats.start(),
                sleep: None,
            });
            Ok(connected)
        })
    }

    fn names_inside(&self) -> bool {
        self.inner.names_inside()
    }

    fn datagrams<'a>(&'a self) -> BoxFuture<'a, Result<Arc<dyn Datagrams>, Failure>> {
        Box::pin(async move {
            self.refuse_at_quota()?;
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
    /// The wait for the rate of the door.
    sleep: Option<Pin<Box<tokio::time::Sleep>>>,
}

impl CountedIo {
    fn count(&self, bytes: usize) {
        self.stats.add(bytes);
    }

    /// How many of `want` bytes may pass now (T-405): all with no limiter;
    /// an error when the flow must end at the quota; `Pending` while the
    /// rate waits.
    fn allow(&mut self, cx: &mut Context<'_>, way: Way, want: usize) -> Poll<io::Result<usize>> {
        let Some(limiter) = self.stats.limiter.clone() else {
            return Poll::Ready(Ok(want));
        };
        if limiter.must_close() {
            return Poll::Ready(Err(io::Error::other(
                "the quota of the door is used for this period",
            )));
        }
        let want = limiter.cap(want);
        loop {
            if let Some(sleep) = &mut self.sleep {
                if std::future::Future::poll(sleep.as_mut(), cx).is_pending() {
                    return Poll::Pending;
                }
                self.sleep = None;
            }
            match limiter.take(way, want) {
                Ok(n) => return Poll::Ready(Ok(n)),
                Err(wait) => self.sleep = Some(Box::pin(tokio::time::sleep(wait))),
            }
        }
    }

    fn give_back(&self, way: Way, n: usize) {
        if let Some(limiter) = &self.stats.limiter {
            limiter.give_back(way, n);
        }
    }
}

impl AsyncRead for CountedIo {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let allowed = match this.allow(cx, Way::In, buf.remaining()) {
            Poll::Ready(Ok(n)) => n,
            other => return other.map(|r| r.map(|_| ())),
        };
        let mut part = buf.take(allowed);
        let poll = Pin::new(&mut this.inner).poll_read(cx, &mut part);
        let n = part.filled().len();
        if let Poll::Ready(Ok(())) = poll {
            // SAFETY: `part` is the unfilled part of `buf`; `n` bytes of it
            // are filled.
            unsafe { buf.assume_init(n) };
            buf.advance(n);
            this.count(n);
        }
        this.give_back(Way::In, allowed - if poll.is_ready() { n } else { 0 });
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
        let allowed = match this.allow(cx, Way::Out, data.len()) {
            Poll::Ready(Ok(n)) => n,
            other => return other,
        };
        let poll = Pin::new(&mut this.inner).poll_write(cx, &data[..allowed]);
        let written = match &poll {
            Poll::Ready(Ok(n)) => *n,
            _ => 0,
        };
        this.count(written);
        this.give_back(Way::Out, allowed - written);
        poll
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        if this.stats.limiter.is_some() {
            // A rate counts each byte: write the first buffer only.
            let first = bufs
                .iter()
                .find(|b| !b.is_empty())
                .map_or(&[][..], |b| &b[..]);
            return Pin::new(this).poll_write(cx, first);
        }
        let poll = Pin::new(&mut this.inner).poll_write_vectored(cx, bufs);
        if let Poll::Ready(Ok(n)) = poll {
            this.count(n);
        }
        poll
    }

    fn is_write_vectored(&self) -> bool {
        self.stats.limiter.is_none() && self.inner.is_write_vectored()
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
        let len = transmit.contents.len();
        if let Some(limiter) = &self.stats.limiter {
            // UDP has no flow control: a datagram over the rate or the
            // quota is dropped (T-405).
            if limiter.exhausted() || limiter.take(Way::Out, len).ok() != Some(len) {
                return Ok(());
            }
        }
        self.inner.try_send(transmit)?;
        self.stats.add(len);
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
            self.stats.add(bytes);
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
