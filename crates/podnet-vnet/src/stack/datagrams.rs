//! A virtual UDP socket as the [`Datagrams`] of `podnet_core::network`
//! (T-056), so that a consumer of a dialer (QUIC, a DNS client) sends UDP
//! through the virtual network.

use std::io::{self, IoSliceMut};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use podnet_core::network::{Datagrams, RecvMeta, Transmit};
use tokio::sync::{mpsc, Notify};

use super::UdpSocket;
use crate::addr::VirtAddr;

/// The datagrams of one virtual UDP socket.
///
/// A send does not wait: when the queue to the stack is full, the datagram
/// is dropped, as a full kernel UDP buffer drops it.
pub struct VnetDatagrams {
    id: u32,
    local: VirtAddr,
    recv_rx: Mutex<mpsc::Receiver<(Vec<u8>, VirtAddr)>>,
    send_tx: mpsc::Sender<(Vec<u8>, VirtAddr)>,
    kick: Arc<Notify>,
}

impl std::fmt::Debug for VnetDatagrams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VnetDatagrams")
            .field("id", &self.id)
            .field("local", &self.local)
            .finish()
    }
}

impl UdpSocket {
    /// This socket as [`Datagrams`].
    pub fn into_datagrams(self) -> VnetDatagrams {
        VnetDatagrams {
            id: self.id,
            local: self.local,
            recv_rx: Mutex::new(self.recv_rx),
            send_tx: self.send_tx,
            kick: self.kick,
        }
    }
}

impl Datagrams for VnetDatagrams {
    fn poll_writable(&self, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.send_tx.is_closed() {
            return Poll::Ready(Err(closed()));
        }
        Poll::Ready(Ok(()))
    }

    fn try_send(&self, t: &Transmit<'_>) -> io::Result<()> {
        let to = VirtAddr::new(t.destination.ip(), t.destination.port());
        let size = t
            .segment_size
            .filter(|s| *s > 0)
            .unwrap_or(t.contents.len().max(1));
        for chunk in t.contents.chunks(size) {
            match self.send_tx.try_send((chunk.to_vec(), to)) {
                Ok(()) | Err(mpsc::error::TrySendError::Full(_)) => {}
                Err(mpsc::error::TrySendError::Closed(_)) => return Err(closed()),
            }
        }
        self.kick.notify_one();
        Ok(())
    }

    fn poll_recv(
        &self,
        cx: &mut Context<'_>,
        bufs: &mut [IoSliceMut<'_>],
        meta: &mut [RecvMeta],
    ) -> Poll<io::Result<usize>> {
        let (Some(buf), Some(m)) = (bufs.first_mut(), meta.first_mut()) else {
            return Poll::Ready(Ok(0));
        };
        let mut rx = self.recv_rx.lock().unwrap_or_else(|e| e.into_inner());
        match rx.poll_recv(cx) {
            Poll::Ready(Some((data, from))) => {
                let len = data.len().min(buf.len());
                buf[..len].copy_from_slice(&data[..len]);
                *m = RecvMeta {
                    addr: SocketAddr::new(from.ip, from.port),
                    len,
                    stride: len,
                    dst_ip: Some(self.local.ip),
                };
                Poll::Ready(Ok(1))
            }
            Poll::Ready(None) => Poll::Ready(Err(closed())),
            Poll::Pending => Poll::Pending,
        }
    }

    fn local_addr(&self) -> io::Result<SocketAddr> {
        Ok(SocketAddr::new(self.local.ip, self.local.port))
    }
}

fn closed() -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, "virtual UDP socket is closed")
}
