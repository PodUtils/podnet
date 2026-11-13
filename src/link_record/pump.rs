//! The two tasks of a sealed stream: the sender seals the bytes that the
//! application wrote, the receiver opens the records that came.

use std::collections::VecDeque;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Instant;

use pod_core::entropy::OsEntropy;
use pod_e2e::record::{record_len, Fault, Opened, HEADER};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, DuplexStream, ReadBuf};
use tokio::sync::mpsc;

use super::pack::{self, Refused};
use super::{lock, Command, NewCarrier, Reader, Shared, COMPRESSED, DATA, FIN};

/// The bytes between the application and the sender.
pub(super) const PIPE: usize = 256 * 1024;

/// The most payload bytes of one record.
const CHUNK: usize = 32 * 1024;

/// The bytes of sent payloads that the sender keeps for a RESUME.
const LOG_LIMIT: usize = 1024 * 1024;

/// The first carrier of the stream, or a later one of
/// [`super::Sealed::rebind`].
enum Carrier<F, N> {
    First(F),
    Next(N),
}

impl<F: AsyncRead + Unpin, N: AsyncRead + Unpin> AsyncRead for Carrier<F, N> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Carrier::First(io) => Pin::new(io).poll_read(cx, buf),
            Carrier::Next(io) => Pin::new(io).poll_read(cx, buf),
        }
    }
}

impl<F: AsyncWrite + Unpin, N: AsyncWrite + Unpin> AsyncWrite for Carrier<F, N> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Carrier::First(io) => Pin::new(io).poll_write(cx, buf),
            Carrier::Next(io) => Pin::new(io).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Carrier::First(io) => Pin::new(io).poll_flush(cx),
            Carrier::Next(io) => Pin::new(io).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Carrier::First(io) => Pin::new(io).poll_shutdown(cx),
            Carrier::Next(io) => Pin::new(io).poll_shutdown(cx),
        }
    }
}

/// A sent record: its counter, type, flags and payload.
type Row = (u64, u8, u16, Vec<u8>);

/// The records that a RESUME can ask for again.
#[derive(Default)]
struct Log {
    rows: VecDeque<Row>,
    bytes: usize,
    /// The records below this counter are gone from the log.
    kept_from: u64,
}

impl Log {
    fn push(&mut self, counter: u64, kind: u8, flags: u16, payload: &[u8]) {
        self.bytes += payload.len();
        self.rows
            .push_back((counter, kind, flags, payload.to_vec()));
        while self.bytes > LOG_LIMIT {
            let Some((c, _, _, p)) = self.rows.pop_front() else {
                break;
            };
            self.bytes -= p.len();
            self.kept_from = c + 1;
        }
    }

    /// The records from `next` on, or `None` when some are gone.
    fn from(&self, next: u64) -> Option<Vec<Row>> {
        if next < self.kept_from {
            return None;
        }
        Some(self.rows.iter().filter(|r| r.0 >= next).cloned().collect())
    }
}

fn counter_of(record: &[u8]) -> u64 {
    u64::from_be_bytes(record[8..16].try_into().expect("a record has a header"))
}

fn bad(why: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, why.to_string())
}

async fn put<W: AsyncWrite + Unpin>(
    shared: &Shared,
    log: Option<&mut Log>,
    io: &mut W,
    kind: u8,
    flags: u16,
    payload: &[u8],
) -> io::Result<()> {
    let record = lock(&shared.records)
        .seal(Instant::now(), kind, flags, 0, payload)
        .map_err(bad)?;
    if let Some(log) = log {
        log.push(counter_of(&record), kind, flags, payload);
    }
    shared
        .stats
        .sent
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    shared
        .stats
        .wire_out
        .fetch_add(record.len() as u64, std::sync::atomic::Ordering::SeqCst);
    io.write_all(&record).await
}

/// Seal the bytes of `app` into records on `io`, and serve the commands.
pub(super) async fn send<W: AsyncWrite + Unpin + Send>(
    shared: Arc<Shared>,
    mut app: DuplexStream,
    io: W,
    mut commands: mpsc::UnboundedReceiver<Command>,
) {
    let mut io: Out<W> = Some(Carrier::First(io));
    let _ = run_send(&shared, &mut app, &mut io, &mut commands).await;
    if let Some(mut io) = io {
        let _ = io.shutdown().await;
    }
}

/// The carrier of the sender. A write that fails drops it when the stream
/// can move to a new carrier ([`super::Options::carry_over`]); the record
/// stays in the log for the RESUME of the peer.
type Out<W> = Option<Carrier<W, super::Writer>>;

fn lost<W>(io: &mut Out<W>, carry: bool, sent: io::Result<()>) -> io::Result<()> {
    match sent {
        Err(_) if carry => {
            *io = None;
            Ok(())
        }
        other => other,
    }
}

async fn run_send<W: AsyncWrite + Unpin>(
    shared: &Shared,
    app: &mut DuplexStream,
    io: &mut Out<W>,
    commands: &mut mpsc::UnboundedReceiver<Command>,
) -> io::Result<()> {
    let carry = shared.options.carry_over;
    let mut log = Log::default();
    let mut buf = vec![0u8; CHUNK];
    let mut commanded = true;
    // The FIN record is not sent yet.
    let mut open = true;
    // A new carrier waits for the RESUME of the peer: no new record.
    let mut held = false;
    loop {
        let reading = open && !held && io.is_some();
        if !reading && !commanded {
            return Ok(());
        }
        tokio::select! {
            read = app.read(&mut buf), if reading => {
                let n = read?;
                let Some(w) = io.as_mut() else { continue };
                let sent = if n == 0 {
                    open = false;
                    put(shared, Some(&mut log), w, FIN, 0, &[]).await
                } else {
                    let packed = shared.options.compress.then(|| pack::pack(&buf[..n])).flatten();
                    match &packed {
                        Some(p) => put(shared, Some(&mut log), w, DATA, COMPRESSED, p).await,
                        None => put(shared, Some(&mut log), w, DATA, 0, &buf[..n]).await,
                    }
                };
                lost(io, carry, sent)?;
                // Without a new carrier to come, the FIN ends the sender.
                if !open && !carry {
                    return Ok(());
                }
            }
            command = commands.recv(), if commanded => match command {
                // The application dropped its end: only its bytes are left.
                None => commanded = false,
                Some(Command::Stop) => return Ok(()),
                Some(Command::Resume) => {
                    let Some(w) = io.as_mut() else { continue };
                    let sent = resume(shared, w).await;
                    lost(io, carry, sent)?;
                }
                Some(Command::Resend(next)) => {
                    held = false;
                    let rows = log.from(next).ok_or_else(|| {
                        bad("the records that the peer misses are no longer kept")
                    })?;
                    for (_, kind, flags, payload) in rows {
                        let Some(w) = io.as_mut() else { break };
                        let sent = put(shared, Some(&mut log), w, kind, flags, &payload).await;
                        lost(io, carry, sent)?;
                    }
                }
                Some(Command::Carry(w)) => {
                    let mut w = Carrier::Next(w);
                    let sent = resume(shared, &mut w).await;
                    *io = Some(w);
                    held = true;
                    lost(io, carry, sent)?;
                }
            },
        }
    }
}

/// Write the RESUME record of this end on `io`.
async fn resume<W: AsyncWrite + Unpin>(shared: &Shared, io: &mut W) -> io::Result<()> {
    let record = lock(&shared.records).resume(&mut OsEntropy).map_err(bad)?;
    shared
        .stats
        .wire_out
        .fetch_add(record.len() as u64, std::sync::atomic::Ordering::SeqCst);
    io.write_all(&record).await
}

/// Open the records of `io` and give the bytes to the application. A fault
/// ends the stream: the reader gets the error, and the sender stops. A new
/// carrier of [`super::Sealed::rebind`] takes the place of `io`.
pub(super) async fn receive<R: AsyncRead + Unpin + Send>(
    shared: Arc<Shared>,
    io: R,
    out: mpsc::Sender<io::Result<Vec<u8>>>,
    commands: mpsc::WeakUnboundedSender<Command>,
    mut carriers: mpsc::UnboundedReceiver<NewCarrier>,
) {
    let mut out = Some(out);
    let io = Carrier::First(io);
    if let Err(e) = run_receive(&shared, io, &mut out, &commands, &mut carriers).await {
        if let Some(commands) = commands.upgrade() {
            let _ = commands.send(Command::Stop);
        }
        if let Some(out) = out {
            let _ = out.send(Err(e)).await;
        }
    }
}

/// One whole record, or `None` when the stream ended.
async fn read_record<R: AsyncRead + Unpin>(io: &mut R) -> io::Result<Option<Vec<u8>>> {
    let mut record = vec![0u8; HEADER];
    if io.read_exact(&mut record).await.is_err() {
        return Ok(None);
    }
    let len = record_len(&record).map_err(bad)?;
    record.resize(len, 0);
    if io.read_exact(&mut record[HEADER..]).await.is_err() {
        return Ok(None);
    }
    Ok(Some(record))
}

/// What the receiver waits for.
enum Event {
    /// The application dropped its reader.
    Closed,
    /// A new carrier, or `None` when no end can give one.
    Carrier(Option<NewCarrier>),
    Read(io::Result<Option<Vec<u8>>>),
}

/// Read the new carrier from now on, and give its write half to the
/// sender, which sends the RESUME record on it.
fn take<R>(
    (read, write): NewCarrier,
    commands: &mpsc::WeakUnboundedSender<Command>,
) -> Carrier<R, Reader> {
    if let Some(commands) = commands.upgrade() {
        let _ = commands.send(Command::Carry(write));
    }
    Carrier::Next(read)
}

async fn closed(out: &Option<mpsc::Sender<io::Result<Vec<u8>>>>) {
    match out {
        Some(out) => out.closed().await,
        None => std::future::pending().await,
    }
}

async fn run_receive<R: AsyncRead + Unpin>(
    shared: &Shared,
    mut io: Carrier<R, Reader>,
    out: &mut Option<mpsc::Sender<io::Result<Vec<u8>>>>,
    commands: &mpsc::WeakUnboundedSender<Command>,
    carriers: &mut mpsc::UnboundedReceiver<NewCarrier>,
) -> io::Result<()> {
    use std::sync::atomic::Ordering::SeqCst;
    let cut = || {
        io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "the stream was cut: it ended with no FIN record",
        )
    };
    let (carry, window) = (
        shared.options.carry_over,
        shared.options.records.resume_window,
    );
    let mut rebinding = true;
    loop {
        let event = tokio::select! {
            () = closed(out) => Event::Closed,
            new = carriers.recv(), if rebinding => Event::Carrier(new),
            read = read_record(&mut io) => Event::Read(read),
        };
        let record = match event {
            Event::Closed => return Ok(()),
            Event::Carrier(Some(new)) => {
                io = take(new, commands);
                continue;
            }
            Event::Carrier(None) => {
                rebinding = false;
                continue;
            }
            Event::Read(read) => read?,
        };
        let Some(record) = record else {
            if !carry || !rebinding {
                return Err(cut());
            }
            // The carrier is lost: wait for a new one for the window.
            match tokio::time::timeout(window, carriers.recv()).await {
                Ok(Some(new)) => io = take(new, commands),
                Ok(None) => return Err(cut()),
                Err(_) => {
                    return Err(super::expired(
                        window,
                        format!("no new carrier came in {} s", window.as_secs()),
                    ))
                }
            }
            continue;
        };
        let len = record.len();
        shared.stats.wire_in.fetch_add(len as u64, SeqCst);
        let opened = lock(&shared.records).open(Instant::now(), &record);
        match opened {
            Ok(Opened::Record(r)) => {
                shared.stats.received.fetch_add(1, SeqCst);
                match r.kind {
                    DATA => {
                        let payload = match r.flags {
                            0 => r.payload,
                            COMPRESSED => pack::unpack(&r.payload).map_err(|e| match e {
                                Refused::Limit => bad("decompress-limit"),
                                Refused::Bad => bad("a compressed record is not snappy"),
                            })?,
                            other => return Err(bad(format!("a record has the flags {other:#x}"))),
                        };
                        let Some(to) = out.as_ref() else { continue };
                        if !payload.is_empty() && to.send(Ok(payload)).await.is_err() {
                            return Ok(());
                        }
                    }
                    // With a new carrier to come, the receiver stays for a
                    // RESUME of the peer; the application reads the end.
                    FIN if carry => *out = None,
                    FIN => return Ok(()),
                    other => return Err(bad(format!("a record has the type {other}"))),
                }
            }
            Ok(Opened::Resume { next }) => {
                if let Some(commands) = commands.upgrade() {
                    let _ = commands.send(Command::Resend(next));
                }
            }
            Err(Fault::Replayed(_) | Fault::Stale(_)) => {
                shared.stats.replayed.fetch_add(1, SeqCst);
            }
            Err(Fault::Resume(_)) => {
                shared.stats.refused.fetch_add(1, SeqCst);
            }
            // The third failed RESUME, or one after the window (T-301).
            Err(Fault::ResumeExpired) => {
                shared.stats.refused.fetch_add(1, SeqCst);
                return Err(super::expired(window, Fault::ResumeExpired.to_string()));
            }
            Err(fault) => return Err(bad(fault)),
        }
    }
}
