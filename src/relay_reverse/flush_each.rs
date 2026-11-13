//! A writer that flushes after each write (T-315).
//!
//! `pod_relay::reverse::operator` up to podcrates v0.31.0 flushes the local
//! side only at the end of a session. `tokio::io::stdout()` keeps bytes in a
//! buffer until a newline or a flush, so a frame with no newline (the binary
//! KEXINIT of sshd) stays in the buffer, and `ssh` through `ProxyCommand`
//! waits until sshd ends the session. This writer gives each write to the
//! inner writer and returns only when the inner writer has flushed it.
//!
//! [`LayerGuard`] stops a plain session whose node greets with the
//! resumable layer (`relay node --resume`, T-087): the operator then gets a
//! clear error, not the bytes of the layer.

use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{ready, Context, Poll};

use tokio::io::AsyncWrite;

/// A writer that flushes the inner writer after each write.
pub struct FlushEach<W> {
    inner: W,
    /// The count of bytes that the inner writer took and that wait for the
    /// end of their flush. The caller polls again with the same buffer.
    written: Option<usize>,
}

impl<W> FlushEach<W> {
    /// Wrap `inner`.
    pub fn new(inner: W) -> FlushEach<W> {
        FlushEach {
            inner,
            written: None,
        }
    }
}

impl<W: AsyncWrite + Unpin> AsyncWrite for FlushEach<W> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = &mut *self;
        let n = match this.written {
            Some(n) => n,
            None => {
                let n = ready!(Pin::new(&mut this.inner).poll_write(cx, buf))?;
                this.written = Some(n);
                n
            }
        };
        ready!(Pin::new(&mut this.inner).poll_flush(cx))?;
        this.written = None;
        Poll::Ready(Ok(n))
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

/// The first bytes of a `GREETING` of the layer: its type, its length (4
/// bytes) and the name of the protocol.
const GREETING_HEAD: usize = 5 + pod_session::record::MAGIC.len();

/// Whether `first`, the first bytes of the node, start a `GREETING` of the
/// resumable layer.
pub fn greets_with_the_layer(first: &[u8]) -> bool {
    first.len() >= GREETING_HEAD
        && first[0] == pod_session::record::kind::GREETING
        && first[5..GREETING_HEAD] == pod_session::record::MAGIC[..]
}

/// A writer that refuses the first write when it is a `GREETING` of the
/// resumable layer, and sets `seen`. Each other byte goes to the inner
/// writer. The far end of the layer writes its `GREETING` in one record, so
/// the first frame of the node holds it.
pub struct LayerGuard<W> {
    inner: W,
    first: bool,
    seen: Arc<AtomicBool>,
}

impl<W> LayerGuard<W> {
    /// Wrap `inner`; `seen` is set when the node greets with the layer.
    pub fn new(inner: W, seen: Arc<AtomicBool>) -> LayerGuard<W> {
        LayerGuard {
            inner,
            first: true,
            seen,
        }
    }
}

impl<W: AsyncWrite + Unpin> AsyncWrite for LayerGuard<W> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.first && !buf.is_empty() {
            if greets_with_the_layer(buf) {
                self.seen.store(true, Ordering::SeqCst);
                return Poll::Ready(Err(io::Error::other(
                    "the node greets with the resumable layer",
                )));
            }
            self.first = false;
        }
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tokio::io::AsyncWriteExt;

    /// A writer that holds each write until a flush, as a buffered stdout does.
    #[derive(Clone, Default)]
    struct Held {
        held: Arc<Mutex<Vec<u8>>>,
        out: Arc<Mutex<Vec<u8>>>,
    }

    impl AsyncWrite for Held {
        fn poll_write(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<io::Result<usize>> {
            self.held.lock().unwrap().extend_from_slice(buf);
            Poll::Ready(Ok(buf.len()))
        }
        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            let mut held = self.held.lock().unwrap();
            self.out.lock().unwrap().extend_from_slice(&held);
            held.clear();
            Poll::Ready(Ok(()))
        }
        fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            self.poll_flush(cx)
        }
    }

    #[tokio::test]
    async fn relay_connect_output_reaches_the_reader_with_no_newline() {
        let held = Held::default();
        let mut w = FlushEach::new(held.clone());
        // A KEXINIT-like frame: binary, no newline.
        w.write_all(&[0, 0, 1, 0x14, 0xaa, 0xbb]).await.unwrap();
        assert_eq!(*held.out.lock().unwrap(), vec![0, 0, 1, 0x14, 0xaa, 0xbb]);
        assert!(held.held.lock().unwrap().is_empty());
    }

    // T-087: the guard stops a GREETING of the layer, and passes each other
    // first frame, also one that starts with the same type byte.
    #[tokio::test]
    async fn the_guard_stops_only_a_greeting_of_the_layer() {
        let mut greeting = vec![pod_session::record::kind::GREETING, 0, 0, 0, 40];
        greeting.extend_from_slice(pod_session::record::MAGIC);
        greeting.extend_from_slice(&[1, 2, 0, 0]);
        let seen = Arc::new(AtomicBool::new(false));
        let mut out = Vec::new();
        let mut w = LayerGuard::new(&mut out, seen.clone());
        assert!(w.write_all(&greeting).await.is_err());
        assert!(seen.load(Ordering::SeqCst));
        assert!(out.is_empty());
        for first in [
            &b"SSH-2.0-OpenSSH_9.6\r\n"[..],
            &[1u8, 0, 0, 0, 3, 7, 7, 7][..],
        ] {
            let seen = Arc::new(AtomicBool::new(false));
            let mut out = Vec::new();
            let mut w = LayerGuard::new(&mut out, seen.clone());
            w.write_all(first).await.unwrap();
            w.write_all(&greeting).await.unwrap();
            assert!(!seen.load(Ordering::SeqCst));
            assert_eq!(out.len(), first.len() + greeting.len());
        }
    }
}
