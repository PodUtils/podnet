//! The record layer of the link (T-298, T-299, T-300, T-301).
//!
//! `pod-e2e` of PodUtils/podcrates does the work (podcrates call C246): the
//! end-to-end handshake (Noise XX with the key of each end), the agreement
//! on a suite, the exporter, and the records (`pod_e2e::record`). A stream of
//! a link runs this layer between its two ends. The hops between them (a
//! mesh node that copies the bytes of a forwarded stream, T-287) see sealed
//! records only: the keys come from the handshake of the two ends, not from
//! the TLS exporter of a hop.
//!
//! * [`dial`] is the end that opens the stream. [`accept`] is the end that
//!   serves it. Each gives a [`Sealed`]: a byte stream (`AsyncRead` and
//!   `AsyncWrite`) over the records.
//! * A suite that the two ends do not share fails with exit 78
//!   ([`Options::suites`]). A peer that is not the expected key, or that
//!   the allow list does not hold, fails with exit 77.
//! * [`Options::pad`] pads each record to a bucket (T-300).
//!   [`Options::compress`] compresses the payload with snappy before the
//!   seal (T-299).
//! * A record that comes again inside the replay window is dropped and
//!   counted ([`Stats::replayed`], T-301). A record that fails its check
//!   ends the stream.
//! * [`Sealed::resume`] sends the RESUME record of the layer: the peer
//!   sends again the payloads that this end did not get (T-301). The token
//!   is used once. The window is [`crate::session::RESUME_WINDOW`]: the
//!   default [`Options`] put it in `resume_window` of `pod-e2e`, so the two
//!   layers read one constant.
//! * [`Sealed::rebind`] moves the stream to a new carrier when the stream
//!   has no `pod-session` below it ([`Options::carry_over`]). Each end sends
//!   its RESUME record first on the new carrier, and the peer sends again
//!   from the counter that it names. A link over [`crate::session`] does
//!   not need this: the session sends the lost bytes again from offsets.
//! * Three failed RESUME records, or no new carrier in the window, end the
//!   stream with [`crate::session::ResumeExpired`] (exit 69, [`ended`]).
//!
//! A dropped [`Sealed`] ends its direction as a shutdown does: the tasks
//! send the `FIN` record and end.
//!
//! The wire: the type `DATA` carries bytes of the stream, `FIN` ends one
//! direction (the half-close). A stream that ends with no `FIN` is a cut.

mod pack;
mod pump;

use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use anyhow::anyhow;
use pod_e2e::record::{Records, Settings};
use pod_e2e::{self as e2e, Refusal};
use pod_identity::PublicKey;
use tokio::io::{AsyncRead, AsyncWrite, DuplexStream, ReadBuf};
use tokio::sync::mpsc;

pub use pack::{MAX_RATIO, MAX_UNPACKED, MIN_PACK};
pub use pod_e2e::record::Suite;

use crate::exit::{self, Fault};
use crate::identity::{AllowList, Identity};
use crate::session::{ResumeExpired, RESUME_WINDOW};

/// The type of a record that carries bytes of the stream.
pub const DATA: u8 = 1;
/// The type of a record that ends one direction of the stream.
pub const FIN: u8 = 2;
/// The flag of a record whose payload is compressed with snappy (T-299).
pub const COMPRESSED: u16 = 0x0002;

/// The settings of the layer of one stream.
#[derive(Debug, Clone)]
pub struct Options {
    /// The suites that this end offers, in its order. The initiator's first
    /// suite that the responder offers too is the suite.
    pub suites: Vec<Suite>,
    /// Pad each record to a bucket (T-300).
    pub pad: bool,
    /// Compress the payloads with snappy before the seal (T-299).
    pub compress: bool,
    /// The settings of `pod-e2e` (the rekey, the resume window). `pad` of
    /// this value is set from [`Options::pad`].
    pub records: Settings,
    /// After a cut, wait for a new carrier ([`Sealed::rebind`]) for the
    /// resume window, and do not end the stream. Off: a cut ends the stream.
    pub carry_over: bool,
}

/// The settings of `pod-e2e` of podnet: the resume window is
/// [`RESUME_WINDOW`] of the session (T-301), 10 minutes.
pub const SETTINGS: Settings = Settings {
    resume_window: RESUME_WINDOW,
    ..Settings::DEFAULT
};

impl Default for Options {
    fn default() -> Options {
        Options {
            suites: Suite::ALL.to_vec(),
            pad: false,
            compress: false,
            records: SETTINGS,
            carry_over: false,
        }
    }
}

impl Options {
    /// The options of the query keys `compress` and `pad` (T-306).
    pub fn of_query(query: &crate::link_query::Query) -> Options {
        Options {
            pad: query.pad.unwrap_or(false),
            compress: query.compress == Some("snappy"),
            ..Options::default()
        }
    }

    fn settings(&self) -> Settings {
        Settings {
            pad: self.pad,
            ..self.records
        }
    }
}

/// What a sealed stream counted.
#[derive(Debug, Default)]
pub struct Stats {
    sent: AtomicU64,
    received: AtomicU64,
    replayed: AtomicU64,
    refused: AtomicU64,
    wire_out: AtomicU64,
    wire_in: AtomicU64,
}

impl Stats {
    /// The records sealed, `RESUME` records not counted.
    pub fn sent(&self) -> u64 {
        self.sent.load(Ordering::SeqCst)
    }
    /// The records opened, `RESUME` records not counted.
    pub fn received(&self) -> u64 {
        self.received.load(Ordering::SeqCst)
    }
    /// The records that came again inside the replay window, or below it.
    pub fn replayed(&self) -> u64 {
        self.replayed.load(Ordering::SeqCst)
    }
    /// The RESUME records that failed and were dropped.
    pub fn refused(&self) -> u64 {
        self.refused.load(Ordering::SeqCst)
    }
    /// The bytes that this end wrote to the stream under the layer.
    pub fn wire_out(&self) -> u64 {
        self.wire_out.load(Ordering::SeqCst)
    }
    /// The bytes that this end read from the stream under the layer.
    pub fn wire_in(&self) -> u64 {
        self.wire_in.load(Ordering::SeqCst)
    }
}

/// A command to the sending task.
pub(crate) enum Command {
    /// Send the RESUME record.
    Resume,
    /// The peer sent a RESUME that waits for the record `next`: send again.
    Resend(u64),
    /// The receiving task failed: end the stream.
    Stop,
    /// A new carrier: send the RESUME record on it, then wait for the
    /// RESUME of the peer before new records.
    Carry(Writer),
}

/// The read half of a carrier.
pub(crate) type Reader = Box<dyn AsyncRead + Unpin + Send>;
/// The write half of a carrier.
pub(crate) type Writer = Box<dyn AsyncWrite + Unpin + Send>;
/// A new carrier. The receiver stops to read the old one first, and then
/// gives the write half to the sender: the counter that the RESUME names
/// is then final.
pub(crate) type NewCarrier = (Reader, Writer);

pub(crate) struct Shared {
    pub(crate) records: Mutex<Records>,
    pub(crate) stats: Stats,
    pub(crate) options: Options,
}

/// A byte stream over the records of one stream of a link.
pub struct Sealed {
    write: DuplexStream,
    read: mpsc::Receiver<io::Result<Vec<u8>>>,
    rest: Vec<u8>,
    at: usize,
    commands: mpsc::UnboundedSender<Command>,
    carriers: mpsc::UnboundedSender<NewCarrier>,
    shared: Arc<Shared>,
    peer: PublicKey,
    suite: Suite,
}

impl Sealed {
    /// The key of the peer, which the handshake proved.
    pub fn peer(&self) -> PublicKey {
        self.peer
    }

    /// The suite of the two ends.
    pub fn suite(&self) -> Suite {
        self.suite
    }

    /// What this stream counted.
    pub fn stats(&self) -> &Stats {
        &self.shared.stats
    }

    /// The epoch of the key that this end seals with, and of the one that it
    /// opens with.
    pub fn epochs(&self) -> (u64, u64) {
        lock(&self.shared.records).epochs()
    }

    /// Send the RESUME record: the peer sends again the payloads that this
    /// end did not get. Exit 69 when the stream has ended.
    pub fn resume(&self) -> anyhow::Result<()> {
        self.commands.send(Command::Resume).map_err(|_| gone())
    }

    /// Move the stream to the new carrier `io` (T-301). Each end calls it
    /// with its end of the new carrier, after the loss of the old one. This
    /// end sends its RESUME record first on `io`, and sends no new record
    /// until the RESUME of the peer comes; then it sends again the records
    /// that the peer names. Use it with [`Options::carry_over`]. Exit 69
    /// when the stream has ended.
    pub fn rebind<S>(&self, io: S) -> anyhow::Result<()>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        self.rebinder().rebind(io)
    }

    /// A handle that calls [`Sealed::rebind`] while a task reads and writes
    /// the stream. After the drop of the [`Sealed`], a rebind has no effect.
    pub fn rebinder(&self) -> Rebinder {
        Rebinder {
            carriers: self.carriers.clone(),
        }
    }
}

/// The handle of [`Sealed::rebinder`].
#[derive(Clone)]
pub struct Rebinder {
    carriers: mpsc::UnboundedSender<NewCarrier>,
}

impl Rebinder {
    /// See [`Sealed::rebind`].
    pub fn rebind<S>(&self, io: S) -> anyhow::Result<()>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let (read, write) = tokio::io::split(io);
        self.carriers
            .send((Box::new(read), Box::new(write)))
            .map_err(|_| gone())
    }
}

fn gone() -> anyhow::Error {
    exit::fault(Fault::Unavailable, anyhow!("the sealed stream has ended"))
}

/// The error of a stream that the resume window ended: three failed RESUME
/// records, or no new carrier in `window`. [`ended`] gives exit 69.
pub(crate) fn expired(window: Duration, reason: impl Into<String>) -> io::Error {
    io::Error::new(
        io::ErrorKind::ConnectionAborted,
        ResumeExpired {
            window,
            reason: reason.into(),
        },
    )
}

/// The exit error of a read of a [`Sealed`] that failed: exit 69, with
/// [`ResumeExpired`] in the chain when the resume window ended the stream.
pub fn ended(error: io::Error) -> anyhow::Error {
    let expired = error
        .get_ref()
        .and_then(|e| e.downcast_ref::<ResumeExpired>())
        .cloned();
    let error = match expired {
        Some(e) => anyhow::Error::new(e),
        None => anyhow::Error::new(error),
    };
    exit::fault(Fault::Unavailable, error.context("the sealed stream ended"))
}

pub(crate) fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl AsyncRead for Sealed {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        loop {
            if self.at < self.rest.len() {
                let n = buf.remaining().min(self.rest.len() - self.at);
                let at = self.at;
                buf.put_slice(&self.rest[at..at + n]);
                self.at += n;
                return Poll::Ready(Ok(()));
            }
            match self.read.poll_recv(cx) {
                Poll::Ready(Some(Ok(bytes))) => {
                    self.rest = bytes;
                    self.at = 0;
                }
                Poll::Ready(Some(Err(e))) => return Poll::Ready(Err(e)),
                Poll::Ready(None) => return Poll::Ready(Ok(())),
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

impl AsyncWrite for Sealed {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.write).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.write).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.write).poll_shutdown(cx)
    }
}

/// The exit class of a failure of the channel: 78 for a suite that does
/// not match, 77 for a refused key, 124 for a timeout, 69 for the others.
fn fault(error: e2e::Error) -> anyhow::Error {
    use pod_core::Class;
    let failure: pod_core::Failure = error.into();
    let how = match failure.class {
        Class::Config => Fault::Config,
        Class::Refused => Fault::NoPermission,
        Class::Timeout => Fault::Timeout,
        _ => Fault::Unavailable,
    };
    exit::fault(how, anyhow!("{failure}"))
}

fn not_allowed(peer: &PublicKey) -> anyhow::Error {
    exit::fault(
        Fault::NoPermission,
        anyhow!(
            "the peer {} of the sealed stream is not allowed",
            peer.display_fingerprint()
        ),
    )
}

/// Seal the stream `io` as the end that opened it. The key of the peer must
/// be `expect`, when it is given. The handshake, the agreement on a suite
/// and the start of the records run before this returns. Two tasks run the
/// records; a dropped [`Sealed`] ends them.
pub async fn dial<S>(
    io: S,
    me: &Identity,
    expect: Option<&PublicKey>,
    options: Options,
) -> anyhow::Result<Sealed>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (sealed, send, receive) = dialed(io, me, expect, options).await?;
    tokio::spawn(send);
    tokio::spawn(receive);
    Ok(sealed)
}

/// Seal the stream `io` as the end that serves it. The key of the peer
/// must be in `allow`. See [`dial`].
pub async fn accept<S>(
    io: S,
    me: &Identity,
    allow: &AllowList,
    options: Options,
) -> anyhow::Result<Sealed>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (sealed, send, receive) = accepted(io, me, allow, options).await?;
    tokio::spawn(send);
    tokio::spawn(receive);
    Ok(sealed)
}

/// [`accept`] for a stream that this end only borrows: the records run
/// inside this call, together with `serve`, which gets the [`Sealed`]. The
/// call ends when `serve` and both directions of the records have ended.
pub async fn accept_with<S, F, Fut>(
    io: &mut S,
    me: &Identity,
    allow: &AllowList,
    options: Options,
    serve: F,
) -> anyhow::Result<Fut::Output>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
    F: FnOnce(Sealed) -> Fut,
    Fut: std::future::Future,
{
    let (sealed, send, receive) = accepted(io, me, allow, options).await?;
    let (_, _, out) = tokio::join!(send, receive, serve(sealed));
    Ok(out)
}

type Pumps<F, G> = (Sealed, F, G);

async fn dialed<S>(
    io: S,
    me: &Identity,
    expect: Option<&PublicKey>,
    options: Options,
) -> anyhow::Result<
    Pumps<impl std::future::Future<Output = ()>, impl std::future::Future<Output = ()>>,
>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    let offered = e2e::initiate(io, me.key()).await.map_err(fault)?;
    let peer = offered.peer();
    if expect.is_some_and(|key| *key != peer) {
        offered.reject().await;
        return Err(not_allowed(&peer));
    }
    let channel = offered.accept(None).await.map_err(fault)?;
    parts(channel, peer, options).await
}

async fn accepted<S>(
    io: S,
    me: &Identity,
    allow: &AllowList,
    options: Options,
) -> anyhow::Result<
    Pumps<impl std::future::Future<Output = ()>, impl std::future::Future<Output = ()>>,
>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    let asked = e2e::respond(io, me.key()).await.map_err(fault)?;
    let peer = asked.peer();
    if !allow.admits(&peer) {
        asked.refuse(Refusal::NotAllowed).await;
        return Err(not_allowed(&peer));
    }
    let channel = asked.admit().await.map_err(fault)?;
    parts(channel, peer, options).await
}

/// Agree on a suite over a channel that the caller made (`pod_e2e`), and
/// start the records on its stream. [`dial`] and [`accept`] call it.
pub async fn from_channel<S>(
    channel: e2e::Channel<S>,
    peer: PublicKey,
    options: Options,
) -> anyhow::Result<Sealed>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (sealed, send, receive) = parts(channel, peer, options).await?;
    tokio::spawn(send);
    tokio::spawn(receive);
    Ok(sealed)
}

async fn parts<S>(
    mut channel: e2e::Channel<S>,
    peer: PublicKey,
    options: Options,
) -> anyhow::Result<
    Pumps<impl std::future::Future<Output = ()>, impl std::future::Future<Output = ()>>,
>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    let suite = channel.agree(&options.suites).await.map_err(fault)?;
    let records = channel
        .exporter()
        .ok_or_else(|| exit::fault(Fault::Unavailable, anyhow!("the channel has no exporter")))?
        .records(suite, options.settings(), Instant::now())
        .map_err(fault)?;
    let io = channel.into_transport();
    let shared = Arc::new(Shared {
        records: Mutex::new(records),
        stats: Stats::default(),
        options,
    });
    let (write, sender_read) = tokio::io::duplex(pump::PIPE);
    let (tx, read) = mpsc::channel(8);
    let (commands, command_rx) = mpsc::unbounded_channel();
    let (carriers, carrier_rx) = mpsc::unbounded_channel();
    let (io_read, io_write) = tokio::io::split(io);
    let send = pump::send(shared.clone(), sender_read, io_write, command_rx);
    let receive = pump::receive(
        shared.clone(),
        io_read,
        tx,
        commands.downgrade(),
        carrier_rx,
    );
    let sealed = Sealed {
        write,
        read,
        rest: Vec::new(),
        at: 0,
        commands,
        carriers,
        shared,
        peer,
        suite,
    };
    Ok((sealed, send, receive))
}
