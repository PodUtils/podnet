//! `podnet connect URL --stdio`: one stream on stdin and stdout, for an SSH
//! `ProxyCommand` through `tunnel --forward` (T-082).

use std::os::fd::AsFd as _;
use std::pin::Pin;
use std::task::{Context, Poll, Waker};

use anyhow::Result;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::mpsc;

use podnet::exit::Fault;
use podnet::net::BoxedIo;
use podnet::ws;

/// Dial `remote` and carry stdin to the WebSocket and the WebSocket to
/// stdout. The run ends when both directions end: the end of stdin closes
/// the WebSocket, and the close of the remote ends the read of stdin.
pub(crate) async fn run(remote: &str, auth: Option<&str>) -> Result<()> {
    let socket = ws::dial(remote, ws::DIAL_TIMEOUT, auth)
        .await
        .map_err(|err| {
            podnet::exit::fault(
                Fault::Unavailable,
                err.context(format!("the remote {remote} does not answer")),
            )
        })?;
    let io: BoxedIo = Box::new(StdioIo::new()?);
    ws::bridge_session(socket, io, &podnet::metrics::Metrics::new()).await
}

/// Stdin and stdout as one stream.
///
/// A plain thread reads stdin: a read of the blocking pool of tokio would
/// keep the runtime from its end while stdin stays open. A shutdown of the
/// write side (the remote closed) also ends the read side, so the run does
/// not wait for stdin.
struct StdioIo {
    rx: mpsc::Receiver<Vec<u8>>,
    chunk: Vec<u8>,
    at: usize,
    stdout: tokio::fs::File,
    closed: bool,
    reader: Option<Waker>,
}

impl StdioIo {
    fn new() -> std::io::Result<Self> {
        // A copy of descriptor 1 with no buffer: the Stdout of std holds
        // bytes until a new line.
        let stdout = std::fs::File::from(std::io::stdout().as_fd().try_clone_to_owned()?);
        let (tx, rx) = mpsc::channel::<Vec<u8>>(4);
        std::thread::spawn(move || {
            use std::io::Read as _;
            let mut stdin = std::io::stdin().lock();
            let mut buf = vec![0u8; 16 * 1024];
            loop {
                match stdin.read(&mut buf) {
                    Ok(0) => return,
                    Ok(n) => {
                        if tx.blocking_send(buf[..n].to_vec()).is_err() {
                            return;
                        }
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(err) => {
                        tracing::warn!(error = %err, "stdin read failed");
                        return;
                    }
                }
            }
        });
        Ok(StdioIo {
            rx,
            chunk: Vec::new(),
            at: 0,
            stdout: tokio::fs::File::from_std(stdout),
            closed: false,
            reader: None,
        })
    }
}

impl AsyncRead for StdioIo {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        if this.closed {
            return Poll::Ready(Ok(()));
        }
        if this.at == this.chunk.len() {
            match this.rx.poll_recv(cx) {
                Poll::Ready(Some(chunk)) => {
                    this.chunk = chunk;
                    this.at = 0;
                }
                Poll::Ready(None) => return Poll::Ready(Ok(())),
                Poll::Pending => {
                    this.reader = Some(cx.waker().clone());
                    return Poll::Pending;
                }
            }
        }
        let n = buf.remaining().min(this.chunk.len() - this.at);
        buf.put_slice(&this.chunk[this.at..this.at + n]);
        this.at += n;
        Poll::Ready(Ok(()))
    }
}

impl AsyncWrite for StdioIo {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.get_mut().stdout).poll_write(cx, data)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().stdout).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        this.closed = true;
        if let Some(waker) = this.reader.take() {
            waker.wake();
        }
        Pin::new(&mut this.stdout).poll_flush(cx)
    }
}
