//! Active MASQUE HTTP/3 sessions: `CONNECT-IP`, `CONNECT-UDP`, and `WebTransport`.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};

use bytes::{Bytes, BytesMut};
use pod_quic::noq;
use pod_warp::capsule::{self, CapsuleDecoder, CAPSULE_DATA};
use pod_warp::h3;
use pod_warp::h3::wire::{self, Frames, FRAME_DATA};
use tokio::sync::mpsc;

use super::error::MasqueError;

/// WebTransport bidirectional stream signal value (draft-ietf-webtrans-http3).
pub const WEBTRANSPORT_BIDI_STREAM: u64 = 0x41;

/// Maximum chunk size for reading HTTP/3 streams.
const CHUNK: usize = 64 * 1024;

/// Shared QUIC/HTTP/3 connection state and HTTP/3 datagram demultiplexer.
pub(crate) struct SharedConn {
    pub(crate) conn: noq::Connection,
    pub(crate) _control: noq::SendStream,
    pub(crate) _endpoint: noq::Endpoint,
    pub(crate) routes: Arc<Mutex<HashMap<u64, mpsc::Sender<Bytes>>>>,
    pub(crate) router_task: tokio::task::JoinHandle<()>,
}

impl Drop for SharedConn {
    fn drop(&mut self) {
        self.router_task.abort();
        self.conn.close(0u32.into(), b"");
    }
}

/// Active `CONNECT-IP` session (RFC 9484) carrying IP packets over HTTP/3 datagrams
/// and `DATAGRAM` capsules.
pub struct IpSession {
    pub(crate) shared: Arc<SharedConn>,
    pub(crate) send: noq::SendStream,
    pub(crate) recv: noq::RecvStream,
    pub(crate) frames: Frames,
    pub(crate) dgram_rx: mpsc::Receiver<Bytes>,
    pub(crate) capsules: CapsuleDecoder,
    pub(crate) ready: VecDeque<Vec<u8>>,
    pub(crate) quarter: u64,
    pub(crate) capsule_context: bool,
}

impl IpSession {
    /// Send one IP packet as an HTTP/3 datagram, or as a `DATAGRAM` capsule if oversized.
    pub async fn send_packet(&mut self, packet: &[u8]) -> Result<(), MasqueError> {
        send_context_payload(
            &self.shared.conn,
            &mut self.send,
            self.quarter,
            packet,
            self.capsule_context,
        )
        .await
    }

    /// Force sending one IP packet as a `DATAGRAM` capsule on the request stream.
    pub async fn send_capsule(&mut self, packet: &[u8]) -> Result<(), MasqueError> {
        send_capsule_only(&mut self.send, packet, self.capsule_context).await
    }

    /// Receive the next IP packet from either a QUIC datagram or a `DATAGRAM` capsule.
    pub async fn recv_packet(&mut self) -> Result<Option<Vec<u8>>, MasqueError> {
        recv_context_payload(
            &mut self.recv,
            &mut self.frames,
            &mut self.dgram_rx,
            &mut self.capsules,
            &mut self.ready,
            self.capsule_context,
        )
        .await
    }

    /// Finish the `CONNECT-IP` request stream.
    pub fn close(&mut self) {
        let _ = self.send.finish();
        unregister_route(&self.shared.routes, self.quarter);
    }
}

impl Drop for IpSession {
    fn drop(&mut self) {
        unregister_route(&self.shared.routes, self.quarter);
    }
}

/// Active `CONNECT-UDP` session (RFC 9298) carrying UDP payloads to one target.
pub struct UdpSession {
    pub(crate) shared: Arc<SharedConn>,
    pub(crate) send: noq::SendStream,
    pub(crate) recv: noq::RecvStream,
    pub(crate) frames: Frames,
    pub(crate) dgram_rx: mpsc::Receiver<Bytes>,
    pub(crate) capsules: CapsuleDecoder,
    pub(crate) ready: VecDeque<Vec<u8>>,
    pub(crate) quarter: u64,
}

impl UdpSession {
    /// Send one UDP payload via QUIC HTTP/3 datagram (or `DATAGRAM` capsule if oversized).
    pub async fn send(&mut self, payload: &[u8]) -> Result<(), MasqueError> {
        send_context_payload(
            &self.shared.conn,
            &mut self.send,
            self.quarter,
            payload,
            true,
        )
        .await
    }

    /// Receive the next UDP payload from the target.
    pub async fn recv(&mut self) -> Result<Option<Vec<u8>>, MasqueError> {
        recv_context_payload(
            &mut self.recv,
            &mut self.frames,
            &mut self.dgram_rx,
            &mut self.capsules,
            &mut self.ready,
            true,
        )
        .await
    }

    /// Finish the `CONNECT-UDP` request stream.
    pub fn close(&mut self) {
        let _ = self.send.finish();
        unregister_route(&self.shared.routes, self.quarter);
    }
}

impl Drop for UdpSession {
    fn drop(&mut self) {
        unregister_route(&self.shared.routes, self.quarter);
    }
}

/// Active `WebTransport` session multiplexed over the shared HTTP/3 QUIC connection.
pub struct WebTransportSession {
    pub(crate) shared: Arc<SharedConn>,
    pub(crate) send: noq::SendStream,
    pub(crate) recv: noq::RecvStream,
    pub(crate) frames: Frames,
    pub(crate) dgram_rx: mpsc::Receiver<Bytes>,
    pub(crate) session_id: u64,
    pub(crate) quarter: u64,
}

impl WebTransportSession {
    /// Return the HTTP/3 WebTransport session ID (the CONNECT stream ID).
    pub fn session_id(&self) -> u64 {
        self.session_id
    }

    /// Open a bidirectional WebTransport stream bound to this session.
    pub async fn open_bi(&self) -> Result<(noq::SendStream, noq::RecvStream), MasqueError> {
        let (mut send, recv) =
            self.shared.conn.open_bi().await.map_err(|e| {
                MasqueError::transport(format!("open WebTransport bidi stream: {e}"))
            })?;
        let mut prefix = Vec::new();
        wire::put_varint(&mut prefix, WEBTRANSPORT_BIDI_STREAM);
        wire::put_varint(&mut prefix, self.session_id);
        send.write_all(&prefix).await.map_err(|e| {
            MasqueError::transport(format!("write WebTransport stream header: {e}"))
        })?;
        Ok((send, recv))
    }

    /// Send a WebTransport HTTP/3 datagram tagged with this session's quarter stream ID.
    pub async fn send_datagram(&self, payload: &[u8]) -> Result<(), MasqueError> {
        let mut prefix = Vec::new();
        wire::put_varint(&mut prefix, self.quarter);
        let mut frame = BytesMut::with_capacity(prefix.len() + payload.len());
        frame.extend_from_slice(&prefix);
        frame.extend_from_slice(payload);
        self.shared
            .conn
            .send_datagram_wait(frame.freeze())
            .await
            .map_err(|e| MasqueError::transport(format!("send WebTransport datagram: {e}")))
    }

    /// Receive the next WebTransport HTTP/3 datagram for this session.
    pub async fn recv_datagram(&mut self) -> Result<Option<Vec<u8>>, MasqueError> {
        Ok(self.dgram_rx.recv().await.map(|b| b.to_vec()))
    }

    /// Write bytes as an HTTP/3 `DATA` frame on the WebTransport CONNECT stream.
    pub async fn send_stream_data(&mut self, bytes: &[u8]) -> Result<(), MasqueError> {
        self.send
            .write_all(&h3::frame(FRAME_DATA, bytes))
            .await
            .map_err(|e| MasqueError::transport(format!("write WebTransport DATA frame: {e}")))
    }

    /// Read the next HTTP/3 `DATA` frame from the WebTransport CONNECT stream.
    pub async fn recv_stream_data(&mut self) -> Result<Option<Vec<u8>>, MasqueError> {
        loop {
            if let Some((kind, payload)) = self
                .frames
                .next_frame()
                .map_err(|e| MasqueError::from_transport_failure(&e))?
            {
                if kind == FRAME_DATA {
                    return Ok(Some(payload));
                }
                continue;
            }
            match self.recv.read_chunk(CHUNK).await {
                Ok(Some(chunk)) => self.frames.push(&chunk),
                Ok(None) => return Ok(None),
                Err(e) => {
                    return Err(MasqueError::transport(format!(
                        "read WebTransport stream: {e}"
                    )))
                }
            }
        }
    }

    /// Close the WebTransport CONNECT stream.
    pub fn close(&mut self) {
        let _ = self.send.finish();
        unregister_route(&self.shared.routes, self.quarter);
    }
}

impl Drop for WebTransportSession {
    fn drop(&mut self) {
        unregister_route(&self.shared.routes, self.quarter);
    }
}

pub(crate) fn quarter_stream_id(id: noq::StreamId) -> u64 {
    u64::from(noq::VarInt::from(id)) / 4
}

pub(crate) fn unregister_route(routes: &Mutex<HashMap<u64, mpsc::Sender<Bytes>>>, quarter: u64) {
    let mut map = routes.lock().unwrap_or_else(PoisonError::into_inner);
    map.remove(&quarter);
}

pub(crate) async fn route_datagrams(
    conn: noq::Connection,
    routes: Arc<Mutex<HashMap<u64, mpsc::Sender<Bytes>>>>,
) {
    while let Ok(dgram) = conn.read_datagram().await {
        let Some((quarter, used)) = wire::get_varint(&dgram) else {
            continue;
        };
        let payload = dgram.slice(used..);
        let sender = {
            let map = routes.lock().unwrap_or_else(PoisonError::into_inner);
            map.get(&quarter).cloned()
        };
        if let Some(tx) = sender {
            let _ = tx.try_send(payload);
        }
    }
}

async fn send_context_payload(
    conn: &noq::Connection,
    send: &mut noq::SendStream,
    quarter: u64,
    payload: &[u8],
    capsule_context: bool,
) -> Result<(), MasqueError> {
    let mut prefix = Vec::new();
    wire::put_varint(&mut prefix, quarter);
    wire::put_varint(&mut prefix, 0);
    let fits = conn
        .max_datagram_size()
        .is_some_and(|max| prefix.len() + payload.len() <= max);
    if fits {
        let mut dgram = BytesMut::with_capacity(prefix.len() + payload.len());
        dgram.extend_from_slice(&prefix);
        dgram.extend_from_slice(payload);
        match conn.send_datagram_wait(dgram.freeze()).await {
            Ok(()) => return Ok(()),
            Err(noq::SendDatagramError::TooLarge) => {}
            Err(e) => return Err(MasqueError::transport(format!("send HTTP/3 datagram: {e}"))),
        }
    }
    send_capsule_only(send, payload, capsule_context).await
}

async fn send_capsule_only(
    send: &mut noq::SendStream,
    payload: &[u8],
    with_context: bool,
) -> Result<(), MasqueError> {
    let mut value = BytesMut::with_capacity(payload.len() + 1);
    if with_context {
        wire::put_varint(&mut value, 0);
    }
    value.extend_from_slice(payload);
    let mut capsule_bytes = BytesMut::with_capacity(value.len() + 10);
    capsule::put_capsule(&mut capsule_bytes, CAPSULE_DATA, &value);
    send.write_all(&h3::frame(FRAME_DATA, &capsule_bytes))
        .await
        .map_err(|e| MasqueError::transport(format!("send DATAGRAM capsule: {e}")))
}

async fn recv_context_payload(
    recv: &mut noq::RecvStream,
    frames: &mut Frames,
    dgram_rx: &mut mpsc::Receiver<Bytes>,
    capsules: &mut CapsuleDecoder,
    ready: &mut VecDeque<Vec<u8>>,
    capsule_context: bool,
) -> Result<Option<Vec<u8>>, MasqueError> {
    loop {
        if let Some(payload) = ready.pop_front() {
            return Ok(Some(payload));
        }
        if let Some((kind, payload)) = frames
            .next_frame()
            .map_err(|e| MasqueError::from_transport_failure(&e))?
        {
            if kind == FRAME_DATA {
                decode_capsules(capsules, &payload, ready, capsule_context)?;
            }
            continue;
        }
        tokio::select! {
            dgram = dgram_rx.recv() => match dgram {
                Some(bytes) => {
                    let Some((context, used)) = wire::get_varint(&bytes) else {
                        continue;
                    };
                    if context == 0 {
                        return Ok(Some(bytes[used..].to_vec()));
                    }
                }
                None => return Ok(None),
            },
            chunk = recv.read_chunk(CHUNK) => match chunk {
                Ok(Some(chunk)) => frames.push(&chunk),
                Ok(None) => return Ok(None),
                Err(e) => return Err(MasqueError::transport(format!("read MASQUE stream: {e}"))),
            },
        }
    }
}

fn decode_capsules(
    capsules: &mut CapsuleDecoder,
    bytes: &[u8],
    ready: &mut VecDeque<Vec<u8>>,
    with_context: bool,
) -> Result<(), MasqueError> {
    capsules.push(bytes);
    while let Some((kind, value)) = capsules.next_capsule() {
        if kind != CAPSULE_DATA {
            continue;
        }
        if with_context {
            let Some((ctx, used)) = capsule::get_varint(&value) else {
                continue;
            };
            if ctx == 0 {
                ready.push_back(value[used..].to_vec());
            }
        } else {
            ready.push_back(value.to_vec());
        }
    }
    if let Some(err) = capsules.error() {
        return Err(MasqueError::transport(format!(
            "invalid DATAGRAM capsule: {err}"
        )));
    }
    Ok(())
}
