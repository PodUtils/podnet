//! A writer that flushes after each write (T-315).
//!
//! `pod_relay::reverse::operator` up to podcrates v0.31.0 flushes the local
//! side only at the end of a session. `tokio::io::stdout()` keeps bytes in a
//! buffer until a newline or a flush, so a frame with no newline (the binary
//! KEXINIT of sshd) stays in the buffer, and `ssh` through `ProxyCommand`
//! waits until sshd ends the session. This writer gives each write to the
//! inner writer and returns only when the inner writer has flushed it.

use std::io;
use std::pin::Pin;
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
}
