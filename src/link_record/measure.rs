//! The rate that the link measured (T-371, #493).
//!
//! The opening write of a profile is bounded by two seconds of the rate of
//! the link (FLY-1). The query cannot know that rate, so the record layer
//! measures it before the opening write is sized: it writes one padded
//! record with no payload, and times how long that write waits on the
//! transport. The peer opens the record and drops it, because its payload
//! is empty; it is a keepalive of the profile's size.
//!
//! Waiting time, not wall time, is the sample. A fast link accepts the
//! write at once, so it measures as unknown and keeps the profile target.
//! A link whose queue is smaller than the record makes the write wait, and
//! the bytes over that wait are its rate. The sample has no idle time,
//! which wall time would count, and it does not include the time that the
//! task itself was off the processor.

use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// The shortest wait that gives a rate. Below it the link was not seen to
/// queue, so the rate is 0 and the profile target stands.
const SHORTEST: Duration = Duration::from_millis(5);

/// The smallest sample that gives a rate.
const FEWEST: u64 = 64;

/// The counters of the measured direction.
#[derive(Default)]
struct Counts {
    bytes: u64,
    busy: Duration,
    /// The poll that returned `Pending`, while the wait goes on.
    waiting: Option<Instant>,
}

impl Counts {
    fn add(&mut self, n: usize) {
        self.bytes = self.bytes.saturating_add(n as u64);
    }

    fn pending(&mut self) {
        self.waiting.get_or_insert_with(Instant::now);
    }

    fn ready(&mut self) {
        if let Some(since) = self.waiting.take() {
            self.busy = self.busy.saturating_add(since.elapsed());
        }
    }

    fn rate(&self) -> u64 {
        if self.busy < SHORTEST || self.bytes < FEWEST {
            return 0;
        }
        let per_second = (u128::from(self.bytes) * 1_000_000_000u128) / self.busy.as_nanos();
        let bits = per_second.saturating_mul(8);
        u64::try_from(bits).unwrap_or(u64::MAX)
    }
}

/// The sample that [`Meter`] fills, readable after the probe.
#[derive(Default)]
pub(super) struct Sample {
    write: Mutex<Counts>,
}

impl Sample {
    /// The rate of the write direction in bits per second, or 0 when the
    /// link did not queue the probe enough to say.
    pub(super) fn measured_bps(&self) -> u64 {
        self.write.lock().unwrap_or_else(|e| e.into_inner()).rate()
    }
}

/// A transport that counts the bytes and the waiting time of the probe.
pub(super) struct Meter<S> {
    inner: S,
    sample: Arc<Sample>,
}

impl<S> Meter<S> {
    pub(super) fn new(inner: S) -> Meter<S> {
        Meter {
            inner,
            sample: Arc::new(Sample::default()),
        }
    }

    /// The sample of this meter.
    pub(super) fn sample(&self) -> Arc<Sample> {
        self.sample.clone()
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for Meter<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Meter<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let poll = Pin::new(&mut self.inner).poll_write(cx, buf);
        let mut counts = self.sample.write.lock().unwrap_or_else(|e| e.into_inner());
        match &poll {
            Poll::Pending => counts.pending(),
            Poll::Ready(Ok(n)) => {
                counts.ready();
                counts.add(*n);
            }
            Poll::Ready(Err(_)) => counts.ready(),
        }
        drop(counts);
        poll
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counts(bytes: u64, busy: Duration) -> Counts {
        Counts {
            bytes,
            busy,
            waiting: None,
        }
    }

    #[test]
    fn a_slow_sample_reads_its_rate() {
        // 800 bytes over 100 ms of waiting is 64 kbit/s.
        assert_eq!(counts(800, Duration::from_millis(100)).rate(), 64_000);
    }

    #[test]
    fn a_sample_too_short_or_too_small_reads_zero() {
        assert_eq!(counts(800, Duration::from_millis(1)).rate(), 0);
        assert_eq!(counts(8, Duration::from_millis(100)).rate(), 0);
    }
}
