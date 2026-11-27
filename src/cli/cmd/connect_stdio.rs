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
            let lead = ws::remote_lead(remote, &err);
            podnet::exit::fault(Fault::Unavailable, err.context(lead))
        })?;
    let io: BoxedIo = Box::new(StdioIo::new()?);
    ws::bridge_session(socket, io, &podnet::metrics::Metrics::new()).await
}

/// Stdin and stdout as one stream.
///
/// Plain OS threads read stdin and write stdout: a blocking call on tokio's
/// blocking pool would keep the runtime from its end while stdin stays open or
/// a stdout pipe is full and unread. A shutdown of the write side (the remote
/// closed) also ends the read side, so the run does not wait for stdin.
pub(super) struct StdioIo {
    rx: mpsc::Receiver<Vec<u8>>,
    chunk: Vec<u8>,
    at: usize,
    stdout: StdoutQueue,
    closed: bool,
    reader: Option<Waker>,
}

const STDOUT_CAP: usize = 64 * 1024;

struct StdoutState {
    queue: std::collections::VecDeque<Vec<u8>>,
    buffered: usize,
    writing: bool,
    closed: bool,
    err: Option<std::io::ErrorKind>,
    writer_waker: Option<Waker>,
    flush_wakers: Vec<Waker>,
}

struct StdoutQueue {
    inner: std::sync::Arc<(std::sync::Mutex<StdoutState>, std::sync::Condvar)>,
}

impl StdoutQueue {
    fn spawn(mut file: std::fs::File) -> Self {
        let inner = std::sync::Arc::new((
            std::sync::Mutex::new(StdoutState {
                queue: std::collections::VecDeque::new(),
                buffered: 0,
                writing: false,
                closed: false,
                err: None,
                writer_waker: None,
                flush_wakers: Vec::new(),
            }),
            std::sync::Condvar::new(),
        ));
        let worker = inner.clone();
        std::thread::spawn(move || {
            use std::io::Write as _;
            let (lock, cv) = &*worker;
            loop {
                let next = {
                    let mut st = lock.lock().unwrap_or_else(|e| e.into_inner());
                    while st.queue.is_empty() && !st.closed {
                        st = cv.wait(st).unwrap_or_else(|e| e.into_inner());
                    }
                    match st.queue.pop_front() {
                        Some(chunk) => {
                            st.writing = true;
                            Some(chunk)
                        }
                        None => None,
                    }
                };
                let Some(chunk) = next else {
                    let _ = file.flush();
                    return;
                };
                let res = file.write_all(&chunk);
                let mut st = lock.lock().unwrap_or_else(|e| e.into_inner());
                st.buffered = st.buffered.saturating_sub(chunk.len());
                st.writing = false;
                if let Err(err) = res {
                    st.err = Some(err.kind());
                    st.queue.clear();
                    st.buffered = 0;
                    st.closed = true;
                }
                if let Some(w) = st.writer_waker.take() {
                    w.wake();
                }
                if st.queue.is_empty() {
                    for w in st.flush_wakers.drain(..) {
                        w.wake();
                    }
                }
                if st.err.is_some() {
                    return;
                }
            }
        });
        Self { inner }
    }

    fn poll_write(&self, cx: &mut Context<'_>, data: &[u8]) -> Poll<std::io::Result<usize>> {
        let (lock, cv) = &*self.inner;
        let mut st = lock.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(kind) = st.err {
            return Poll::Ready(Err(std::io::Error::from(kind)));
        }
        if data.is_empty() {
            return Poll::Ready(Ok(0));
        }
        if st.buffered >= STDOUT_CAP {
            st.writer_waker = Some(cx.waker().clone());
            return Poll::Pending;
        }
        let take = data.len().min(STDOUT_CAP - st.buffered);
        st.queue.push_back(data[..take].to_vec());
        st.buffered += take;
        cv.notify_one();
        Poll::Ready(Ok(take))
    }

    fn poll_flush(&self, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        let (lock, _) = &*self.inner;
        let mut st = lock.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(kind) = st.err {
            return Poll::Ready(Err(std::io::Error::from(kind)));
        }
        if st.queue.is_empty() && !st.writing {
            return Poll::Ready(Ok(()));
        }
        st.flush_wakers.push(cx.waker().clone());
        Poll::Pending
    }
}

impl Drop for StdoutQueue {
    fn drop(&mut self) {
        let (lock, cv) = &*self.inner;
        let mut st = lock.lock().unwrap_or_else(|e| e.into_inner());
        st.closed = true;
        cv.notify_one();
    }
}

impl StdioIo {
    pub(super) fn new() -> std::io::Result<Self> {
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
            stdout: StdoutQueue::spawn(stdout),
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
        self.get_mut().stdout.poll_write(cx, data)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        self.get_mut().stdout.poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        this.closed = true;
        if let Some(waker) = this.reader.take() {
            waker.wake();
        }
        this.stdout.poll_flush(cx)
    }
}
