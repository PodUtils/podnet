//! The datagrams of one TURN allocation (T-321).
//!
//! A task owns the [`pod_turn::Client`]: it binds a channel for each peer at
//! the first datagram to that peer, sends the datagram in ChannelData, and
//! reads what peers send back. `recv` of the client also refreshes the
//! allocation, the permissions and the channels on their timers. Any failure
//! of a stage ends the socket with a named error. The socket does not retry
//! on its own: the caller (a UDP flow) starts a new socket.
//!
//! UDP has no flow control: a datagram for a full queue is dropped, as in
//! `forwards/udp.rs`.

use std::collections::HashMap;
use std::io::{self, IoSliceMut};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use pod_turn::{Client, Error, Event, Kept};
use tokio::sync::mpsc;

use crate::network::{Class, Datagrams, Failure, RecvMeta, Stage, Transmit};

/// The datagrams that wait in each direction.
const QUEUE: usize = 256;
/// The longest wait for the answer to the end of the allocation.
const CLOSE_WAIT: Duration = Duration::from_secs(3);

/// The stage of TURN that failed. Its name starts the text of the error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TurnStage {
    /// The first Allocate, with its 401 challenge.
    Allocate,
    /// The Refresh of the allocation, or the end of its lifetime.
    Refresh,
    /// The CreatePermission that keeps a peer's address allowed.
    Permission,
    /// The ChannelBind of a peer, or its refresh.
    Bind,
    /// The ChannelData of a datagram.
    Send,
    /// The transport, while the socket waits for a datagram.
    Receive,
}

impl TurnStage {
    pub(super) fn name(self) -> &'static str {
        match self {
            TurnStage::Allocate => "allocate",
            TurnStage::Refresh => "refresh",
            TurnStage::Permission => "permission",
            TurnStage::Bind => "bind",
            TurnStage::Send => "send",
            TurnStage::Receive => "receive",
        }
    }

    /// The stage of a refresh that failed.
    fn of_kept(kept: Kept) -> TurnStage {
        match kept {
            Kept::Allocation => TurnStage::Refresh,
            Kept::Permission(_) => TurnStage::Permission,
            Kept::Channel(_) => TurnStage::Bind,
        }
    }
}

/// The error of a stage: the network, the stage, and the error of the
/// client. The text holds no credential (the client never prints one).
pub(super) fn named(stage: TurnStage, label: &str, error: &Error) -> Failure {
    Failure::new(
        Stage::Relay,
        error.class(),
        format!("{label}: {}: {error}", stage.name()),
    )
}

/// Why the socket ended.
#[derive(Debug, Clone)]
struct Ended {
    kind: io::ErrorKind,
    text: String,
}

impl Ended {
    fn of(failure: &Failure) -> Ended {
        let kind = match failure.class {
            Class::Timeout => io::ErrorKind::TimedOut,
            Class::Refused | Class::Policy => io::ErrorKind::PermissionDenied,
            Class::Usage => io::ErrorKind::InvalidInput,
            _ => io::ErrorKind::Other,
        };
        Ended {
            kind,
            text: failure.text().to_string(),
        }
    }

    fn error(&self) -> io::Error {
        io::Error::new(self.kind, self.text.clone())
    }
}

type Datagram = (SocketAddr, Vec<u8>);

/// The datagrams of an allocation: the address of a peer is the address
/// that the datagram goes to, and the address that it came from.
#[derive(Debug)]
pub(super) struct TurnDatagrams {
    relayed: SocketAddr,
    commands: mpsc::Sender<Datagram>,
    inbox: Mutex<mpsc::Receiver<Datagram>>,
    ended: Arc<Mutex<Option<Ended>>>,
}

impl TurnDatagrams {
    /// Take the allocation of `client`: the task starts.
    pub(super) fn start(client: Client, label: String) -> TurnDatagrams {
        let relayed = client.relayed_address();
        let (commands, command_rx) = mpsc::channel(QUEUE);
        let (inbox_tx, inbox) = mpsc::channel(QUEUE);
        let ended = Arc::new(Mutex::new(None));
        tokio::spawn(run(client, label, command_rx, inbox_tx, ended.clone()));
        TurnDatagrams {
            relayed,
            commands,
            inbox: Mutex::new(inbox),
            ended,
        }
    }

    /// The error of the end; a socket that the caller closed has none.
    fn end_error(&self) -> io::Error {
        let ended = self.ended.lock().unwrap_or_else(|e| e.into_inner());
        match &*ended {
            Some(ended) => ended.error(),
            None => io::Error::new(io::ErrorKind::BrokenPipe, "the TURN socket is closed"),
        }
    }

    fn has_ended(&self) -> bool {
        self.ended
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
    }
}

impl Datagrams for TurnDatagrams {
    fn poll_writable(&self, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.has_ended() {
            return Poll::Ready(Err(self.end_error()));
        }
        Poll::Ready(Ok(()))
    }

    fn try_send(&self, transmit: &Transmit<'_>) -> io::Result<()> {
        let size = transmit
            .segment_size
            .filter(|s| *s > 0)
            .unwrap_or(transmit.contents.len().max(1));
        for chunk in transmit.contents.chunks(size) {
            match self
                .commands
                .try_send((transmit.destination, chunk.to_vec()))
            {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Full(_)) => {
                    tracing::debug!(to = %transmit.destination, "the TURN queue is full; a datagram dropped");
                }
                Err(mpsc::error::TrySendError::Closed(_)) => return Err(self.end_error()),
            }
        }
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
        let mut inbox = self.inbox.lock().unwrap_or_else(|e| e.into_inner());
        match inbox.poll_recv(cx) {
            Poll::Ready(Some((addr, data))) => {
                let len = data.len().min(buf.len());
                buf[..len].copy_from_slice(&data[..len]);
                *m = RecvMeta {
                    addr,
                    len,
                    stride: len,
                    dst_ip: None,
                };
                Poll::Ready(Ok(1))
            }
            Poll::Ready(None) => Poll::Ready(Err(self.end_error())),
            Poll::Pending => Poll::Pending,
        }
    }

    /// The relayed address: the address that peers send to.
    fn local_addr(&self) -> io::Result<SocketAddr> {
        Ok(self.relayed)
    }
}

/// Send `data` to `peer`: bind its channel first when it has none.
async fn send(
    client: &mut Client,
    channels: &mut HashMap<SocketAddr, u16>,
    peer: SocketAddr,
    data: &[u8],
) -> Result<(), (TurnStage, Error)> {
    let channel = match channels.get(&peer) {
        Some(channel) => *channel,
        None => {
            let channel = client
                .bind_channel(peer)
                .await
                .map_err(|e| (TurnStage::Bind, e))?;
            channels.insert(peer, channel);
            channel
        }
    };
    client
        .send(channel, data)
        .await
        .map_err(|e| (TurnStage::Send, e))
}

/// The task: commands in, events out, until the caller drops the socket or
/// a stage fails.
async fn run(
    mut client: Client,
    label: String,
    mut commands: mpsc::Receiver<Datagram>,
    inbox: mpsc::Sender<Datagram>,
    ended: Arc<Mutex<Option<Ended>>>,
) {
    let mut channels = HashMap::new();
    let failure = loop {
        tokio::select! {
            command = commands.recv() => {
                let Some((peer, data)) = command else { break None };
                if let Err((stage, error)) = send(&mut client, &mut channels, peer, &data).await {
                    break Some(named(stage, &label, &error));
                }
            }
            event = client.recv() => match event {
                Ok(Event::Data { peer, data, .. }) => {
                    if inbox.try_send((peer, data)).is_err() {
                        tracing::debug!(%peer, "the TURN inbox is full; a datagram dropped");
                    }
                }
                Ok(Event::RefreshFailed { kept, error }) => {
                    break Some(named(TurnStage::of_kept(kept), &label, &error));
                }
                Ok(Event::ServerError { method, code, reason }) => {
                    tracing::debug!(method, code, reason, "TURN: an error that no request waits for");
                }
                Err(error) => {
                    let stage = match error {
                        Error::NoAllocation => TurnStage::Refresh,
                        _ => TurnStage::Receive,
                    };
                    break Some(named(stage, &label, &error));
                }
            },
        }
    };
    // Close the queues first, then the record: a send sees the error.
    commands.close();
    if let Some(failure) = &failure {
        tracing::warn!("{}", failure.text());
        *ended.lock().unwrap_or_else(|e| e.into_inner()) = Some(Ended::of(failure));
    }
    drop(inbox);
    let _ = tokio::time::timeout(CLOSE_WAIT, client.close()).await;
}
