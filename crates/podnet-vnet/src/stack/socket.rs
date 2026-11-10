use super::*;

// ── connection plumbing ─────────────────────────────────────────────────────

#[derive(Debug)]
pub(super) enum AppEvent {
    /// The handshake completed (sent once, before or after the first data).
    Connected,
    Data(Vec<u8>),
    Eof,
    Reset(String),
}

#[derive(Debug)]
pub(super) enum ConnCmd {
    Close,
    Abort,
    SetNodelay(bool),
}

/// A TCP connection in the virtual network. It implements
/// [`AsyncRead`]/[`AsyncWrite`], so it composes with the rest of the crate.
pub struct TcpStream {
    pub(super) id: u32,
    pub(super) local: VirtAddr,
    pub(super) remote: VirtAddr,
    pub(super) read_rx: mpsc::Receiver<AppEvent>,
    pub(super) write_tx: mpsc::Sender<Vec<u8>>,
    pub(super) write_waker: Arc<AtomicWaker>,
    pub(super) ctl_tx: mpsc::Sender<ConnCmd>,
    pub(super) kick: Arc<Notify>,
    pub(super) read_buf: Vec<u8>,
    pub(super) read_pos: usize,
    pub(super) eof: bool,
    pub(super) nodelay: bool,
}

impl std::fmt::Debug for TcpStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TcpStream")
            .field("id", &self.id)
            .field("local", &self.local)
            .field("remote", &self.remote)
            .field("eof", &self.eof)
            .finish()
    }
}

impl TcpStream {
    /// Wait for the handshake of a connection that this stack opened. A
    /// refusal (a reset, or the end before the handshake) is an error of
    /// kind `ConnectionRefused` or `ConnectionReset`. Data that comes first
    /// is kept for the next read.
    pub async fn established(&mut self) -> io::Result<()> {
        if self.eof {
            return Err(io::Error::from(io::ErrorKind::ConnectionRefused));
        }
        if self.read_pos < self.read_buf.len() {
            return Ok(());
        }
        match self.read_rx.recv().await {
            Some(AppEvent::Connected) => Ok(()),
            Some(AppEvent::Data(data)) => {
                self.read_buf = data;
                self.read_pos = 0;
                Ok(())
            }
            Some(AppEvent::Eof) | None => {
                self.eof = true;
                Err(io::Error::new(
                    io::ErrorKind::ConnectionRefused,
                    "the connection was refused",
                ))
            }
            Some(AppEvent::Reset(message)) => {
                Err(io::Error::new(io::ErrorKind::ConnectionReset, message))
            }
        }
    }

    pub fn id(&self) -> u32 {
        self.id
    }

    pub fn local_addr(&self) -> VirtAddr {
        self.local
    }

    pub fn peer_addr(&self) -> VirtAddr {
        self.remote
    }

    /// Record `TCP_NODELAY`. The reactor applies it to the smoltcp socket,
    /// which is how the shim's swallowed `setsockopt` reaches the stack.
    pub fn set_nodelay(&mut self, enabled: bool) {
        self.nodelay = enabled;
        let _ = self.ctl_tx.try_send(ConnCmd::SetNodelay(enabled));
        self.kick.notify_one();
    }

    pub fn nodelay(&self) -> bool {
        self.nodelay
    }

    /// Half-close the write side.
    pub fn shutdown(&self) {
        let _ = self.ctl_tx.try_send(ConnCmd::Close);
        self.kick.notify_one();
    }

    /// Abort the connection (RST).
    pub fn abort(&self) {
        let _ = self.ctl_tx.try_send(ConnCmd::Abort);
        self.kick.notify_one();
    }

    pub(super) fn drain_read_buffer(&mut self, buf: &mut [u8]) -> usize {
        let available = self.read_buf.len() - self.read_pos;
        let n = available.min(buf.len());
        buf[..n].copy_from_slice(&self.read_buf[self.read_pos..self.read_pos + n]);
        self.read_pos += n;
        if self.read_pos >= self.read_buf.len() {
            self.read_buf.clear();
            self.read_pos = 0;
        }
        n
    }
}

impl AsyncRead for TcpStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if buf.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        if self.read_pos < self.read_buf.len() {
            let want = buf.remaining();
            let mut temp = vec![0u8; want.min(self.read_buf.len() - self.read_pos)];
            let n = self.drain_read_buffer(&mut temp);
            buf.put_slice(&temp[..n]);
            return Poll::Ready(Ok(()));
        }
        if self.eof {
            return Poll::Ready(Ok(()));
        }
        let event = loop {
            match self.read_rx.poll_recv(cx) {
                Poll::Ready(Some(AppEvent::Connected)) => continue,
                other => break other,
            }
        };
        match event {
            Poll::Ready(Some(AppEvent::Connected)) => unreachable!(),
            Poll::Ready(Some(AppEvent::Data(data))) => {
                self.read_buf = data;
                self.read_pos = 0;
                let want = buf.remaining();
                let mut temp = vec![0u8; want.min(self.read_buf.len())];
                let n = self.drain_read_buffer(&mut temp);
                buf.put_slice(&temp[..n]);
                Poll::Ready(Ok(()))
            }
            Poll::Ready(Some(AppEvent::Eof)) => {
                self.eof = true;
                Poll::Ready(Ok(()))
            }
            Poll::Ready(Some(AppEvent::Reset(message))) => {
                Poll::Ready(Err(io::Error::new(io::ErrorKind::ConnectionReset, message)))
            }
            Poll::Ready(None) => {
                self.eof = true;
                Poll::Ready(Ok(()))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl AsyncWrite for TcpStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if buf.is_empty() {
            return Poll::Ready(Ok(0));
        }
        match self.write_tx.try_send(buf.to_vec()) {
            Ok(()) => {
                // The reactor may be parked waiting for link traffic; tell it
                // there is application data to move now, not on the next tick.
                self.kick.notify_one();
                Poll::Ready(Ok(buf.len()))
            }
            Err(mpsc::error::TrySendError::Full(_)) => {
                // The reactor drains the channel each time it services the
                // connection and wakes this waker; that is the write-side
                // backpressure path.
                self.write_waker.register(cx.waker());
                Poll::Pending
            }
            Err(mpsc::error::TrySendError::Closed(_)) => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "virtual TCP connection is closed",
            ))),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let _ = self.ctl_tx.try_send(ConnCmd::Close);
        self.kick.notify_one();
        Poll::Ready(Ok(()))
    }
}

impl Drop for TcpStream {
    fn drop(&mut self) {
        // A dropped stream must release its virtual socket, but it must not do
        // so by closing the connection while data is still in flight.
        //
        // The problem this solves: without any Drop impl, dropping the handle
        // drops `ctl_tx` and the reactor never learns about it. It watches
        // `to_app.is_closed()` to notice that the application has gone, but
        // `to_app` runs the other way and stays open. The socket then sits in
        // CloseWait until the peer closes, which for two handles dropped by the
        // same task never happens, and each such connection keeps its smoltcp
        // socket resident for the life of the process.
        //
        // Why not an unconditional graceful close: a program that writes, then
        // reads to EOF, then drops, has its own `Drop` fire before the peer's
        // reply arrives, and a graceful close cuts the exchange short. That was
        // observed as two tests reading an empty response.
        //
        // Why not an unconditional abort: an abort discards queued data, so a
        // program that wrote and then dropped without reading loses the reply it
        // had not collected.
        //
        // So the close is deferred to the reactor, which knows whether bytes
        // were still owed in either direction, and reaped the socket either way.
        // The handle only asks; the reactor decides.
        let _ = self.ctl_tx.try_send(ConnCmd::Close);
        self.kick.notify_one();
    }
}

/// A UDP socket in the virtual network.
pub struct UdpSocket {
    pub(super) id: u32,
    pub(super) local: VirtAddr,
    pub(super) recv_rx: mpsc::Receiver<(Vec<u8>, VirtAddr)>,
    pub(super) send_tx: mpsc::Sender<(Vec<u8>, VirtAddr)>,
    pub(super) kick: Arc<Notify>,
}

impl std::fmt::Debug for UdpSocket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UdpSocket")
            .field("id", &self.id)
            .field("local", &self.local)
            .finish()
    }
}

impl UdpSocket {
    pub fn id(&self) -> u32 {
        self.id
    }

    pub fn local_addr(&self) -> VirtAddr {
        self.local
    }

    pub async fn send_to(&self, buf: &[u8], remote: VirtAddr) -> io::Result<usize> {
        self.send_tx
            .send((buf.to_vec(), remote))
            .await
            .map_err(|_| {
                io::Error::new(io::ErrorKind::BrokenPipe, "virtual UDP socket is closed")
            })?;
        self.kick.notify_one();
        Ok(buf.len())
    }

    pub async fn recv_from(&mut self, buf: &mut [u8]) -> io::Result<(usize, VirtAddr)> {
        match self.recv_rx.recv().await {
            Some((data, from)) => {
                let n = data.len().min(buf.len());
                buf[..n].copy_from_slice(&data[..n]);
                Ok((n, from))
            }
            None => Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "virtual UDP socket is closed",
            )),
        }
    }
}

/// An accepted-connection queue for one virtual listener.
pub struct TcpListener {
    pub(super) local: VirtAddr,
    pub(super) rx: mpsc::Receiver<TcpStream>,
}

impl std::fmt::Debug for TcpListener {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TcpListener")
            .field("local", &self.local)
            .finish()
    }
}

impl TcpListener {
    pub fn local_addr(&self) -> VirtAddr {
        self.local
    }

    pub async fn accept(&mut self) -> io::Result<TcpStream> {
        self.rx
            .recv()
            .await
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "virtual listener is closed"))
    }
}

/// Live and high-water socket counts for one stack.
///
/// `live` is the number of sockets the reactor currently holds, `peak` the
/// largest it has ever been. A `peak` that rises with every connection while
/// `live` returns to its starting value is the signature of a set that is not
/// being pruned.
#[derive(Debug, Default)]
pub struct SocketCount {
    pub(super) live: AtomicU64,
    pub(super) peak: AtomicU64,
}

impl SocketCount {
    pub(super) fn add(&self) {
        let now = self.live.fetch_add(1, Ordering::Relaxed) + 1;
        self.peak.fetch_max(now, Ordering::Relaxed);
    }

    pub(super) fn remove(&self) {
        // Saturating: a double remove would otherwise wrap the live count to
        // u64::MAX and make every later observation meaningless.
        let mut current = self.live.load(Ordering::Relaxed);
        loop {
            if current == 0 {
                return;
            }
            match self.live.compare_exchange_weak(
                current,
                current - 1,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return,
                Err(actual) => current = actual,
            }
        }
    }

    /// Sockets the reactor holds right now.
    pub fn live(&self) -> u64 {
        self.live.load(Ordering::Relaxed)
    }

    /// The largest number of sockets ever held at once.
    pub fn peak(&self) -> u64 {
        self.peak.load(Ordering::Relaxed)
    }
}
