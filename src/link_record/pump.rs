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

use super::pace::Paced;
use super::pack::{self, Refused};
use super::{lock, Command, NewCarrier, Reader, Shared, COMPRESSED, DATA, ENTROPY, FIN};

/// The bytes between the application and the sender.
pub(super) const PIPE: usize = 256 * 1024;

/// The most payload bytes of one record.
const CHUNK: usize = 32 * 1024;

/// How long the opening read waits for more bytes before it sends the
/// first record (FLY-1). The coalescing is the one write that PACE-2
/// allows, and only at the start of a stream.
const OPENING_GAP: std::time::Duration = std::time::Duration::from_millis(5);

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
    paced: &mut Option<Paced>,
    kind: u8,
    flags: u16,
    payload: &[u8],
) -> io::Result<()> {
    let (record, body) = seal(shared, kind, flags, payload)?;
    if let Some(paced) = paced {
        paced.wait(record.len()).await;
    }
    if let Some(log) = log {
        log.push(counter_of(&record), kind, flags, payload);
    }
    shared.shape.record(payload.len(), body);
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

/// The record of one payload, with the body that the profile chooses when
/// the stream has a profile (T-370), else the one of `pod-e2e`.
fn seal(shared: &Shared, kind: u8, flags: u16, payload: &[u8]) -> io::Result<(Vec<u8>, usize)> {
    let mut records = lock(&shared.records);
    let before = records.stats();
    let record = if let Some(shaper) = &shared.shaper {
        let mut shaper = lock(shaper);
        // PAD-3: the bytes between the payload and the end of the body come
        // from the band of the profile, not from a zero fill. The body
        // starts with the payload length, so the peer keeps the plaintext
        // and drops the tail.
        let room = payload.len().saturating_add(4);
        let shaped = shaper.shape(room);
        let mut framed = None;
        if kind == DATA && !payload.is_empty() && shaped.body > room {
            if let Some(entropy) = &shared.entropy {
                let tail = shaped.body - room;
                let bytes = lock(entropy).fill(tail);
                let mut body = Vec::with_capacity(shaped.body);
                body.extend_from_slice(&(payload.len() as u32).to_be_bytes());
                body.extend_from_slice(payload);
                body.extend_from_slice(&bytes);
                shared.shape.entropy(&bytes);
                framed = Some((body, flags | ENTROPY));
            }
        }
        match framed {
            Some((body, flags)) => records
                .seal_with(Instant::now(), kind, flags, 0, &body, body.len())
                .map_err(bad)?,
            None => records
                .seal_with(Instant::now(), kind, flags, 0, payload, shaped.body)
                .map_err(bad)?,
        }
    } else {
        records
            .seal(Instant::now(), kind, flags, 0, payload)
            .map_err(bad)?
    };
    let after = records.stats();
    Ok((record, (after.body_out - before.body_out) as usize))
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
    let mut paced = shared
        .options
        .pace
        .then_some(shared.options.profile.as_ref())
        .flatten()
        .map(|profile| Paced::new(profile, shared.options.measured_bps, Instant::now()));
    // The opening write, coalesced to the target of the profile (FLY-1).
    if open && !held && io.is_some() {
        let target = shared.options.opening_write();
        if target > 0 {
            let (first, eof) = opening(app, target).await?;
            if !first.is_empty() {
                if let Some(w) = io.as_mut() {
                    let sent = put(shared, Some(&mut log), w, &mut paced, DATA, 0, &first).await;
                    lost(io, carry, sent)?;
                }
            }
            if eof {
                open = false;
                if let Some(w) = io.as_mut() {
                    let sent = put(shared, Some(&mut log), w, &mut paced, FIN, 0, &[]).await;
                    lost(io, carry, sent)?;
                }
                if !carry {
                    return Ok(());
                }
            }
        }
    }
    let mut keepalive = paced
        .as_mut()
        .map(|paced| Instant::now() + paced.keepalive());
    loop {
        let reading = open && !held && io.is_some();
        if !reading && !commanded {
            return Ok(());
        }
        let beat =
            keepalive.unwrap_or_else(|| Instant::now() + std::time::Duration::from_secs(3600));
        let idle = paced.is_some() && reading && !held;
        tokio::select! {
            read = app.read(&mut buf), if reading => {
                let n = read?;
                let Some(w) = io.as_mut() else { continue };
                let sent = if n == 0 {
                    open = false;
                    put(shared, Some(&mut log), w, &mut paced, FIN, 0, &[]).await
                } else {
                    let packed = shared.options.compress.then(|| pack::pack(&buf[..n])).flatten();
                    match &packed {
                        Some(p) => put(shared, Some(&mut log), w, &mut paced, DATA, COMPRESSED, p).await,
                        None => put(shared, Some(&mut log), w, &mut paced, DATA, 0, &buf[..n]).await,
                    }
                };
                keepalive = paced.as_mut().map(|paced| Instant::now() + paced.keepalive());
                lost(io, carry, sent)?;
                // Without a new carrier to come, the FIN ends the sender.
                if !open && !carry {
                    return Ok(());
                }
            }
            // PACE-1: an idle channel sends an empty record, at a jittered
            // interval. The peer drops an empty payload.
            () = tokio::time::sleep_until(tokio::time::Instant::from_std(beat)), if idle => {
                if let Some(w) = io.as_mut() {
                    let sent = put(shared, Some(&mut log), w, &mut paced, DATA, 0, &[]).await;
                    lost(io, carry, sent)?;
                }
                keepalive = paced.as_mut().map(|paced| Instant::now() + paced.keepalive());
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
                        let sent = put(shared, Some(&mut log), w, &mut paced, kind, flags, &payload).await;
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

/// The opening write of a stream: up to `target` bytes, with a short pause
/// for each more byte, so that a page load's first records answer as one
/// write. Gives the bytes and whether the application ended.
async fn opening<W>(app: &mut W, target: usize) -> io::Result<(Vec<u8>, bool)>
where
    W: AsyncRead + Unpin,
{
    let mut out = Vec::with_capacity(target.min(CHUNK));
    let mut first = true;
    while out.len() < target {
        let want = (target - out.len()).min(CHUNK);
        let mut buf = vec![0u8; want];
        let read = if first {
            app.read(&mut buf).await
        } else {
            match tokio::time::timeout(OPENING_GAP, app.read(&mut buf)).await {
                Ok(read) => read,
                // No more byte in the gap: the opening write is complete.
                Err(_) => return Ok((out, false)),
            }
        };
        let n = read?;
        if n == 0 {
            return Ok((out, true));
        }
        out.extend_from_slice(&buf[..n]);
        first = false;
    }
    Ok((out, false))
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

/// The most bytes of a body that the buffer of a record holds before they
/// come: the buffer grows with the bytes, so a header that says 16 MiB
/// with no bytes after it does not take 16 MiB (T-343).
const BODY_STEP: usize = 64 * 1024;

/// One whole record, or `None` when the stream ended.
async fn read_record<R: AsyncRead + Unpin>(io: &mut R) -> io::Result<Option<Vec<u8>>> {
    let mut record = vec![0u8; HEADER];
    if io.read_exact(&mut record).await.is_err() {
        return Ok(None);
    }
    let len = record_len(&record).map_err(bad)?;
    let body = len - HEADER;
    record.reserve_exact(body.min(BODY_STEP));
    let read = (&mut *io).take(body as u64).read_to_end(&mut record).await;
    if read.is_err() || record.len() != len {
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
                        let known = COMPRESSED | ENTROPY;
                        if r.flags & !known != 0 {
                            return Err(bad(format!("a record has the flags {:#x}", r.flags)));
                        }
                        let mut body = r.payload;
                        if r.flags & ENTROPY != 0 {
                            if body.len() < 4 {
                                return Err(bad("an entropy record has no length"));
                            }
                            let len = u32::from_be_bytes(body[..4].try_into().expect("four bytes"))
                                as usize;
                            if len > body.len() - 4 {
                                return Err(bad("an entropy record is shorter than its payload"));
                            }
                            body = body[4..4 + len].to_vec();
                        }
                        let payload = if r.flags & COMPRESSED != 0 {
                            pack::unpack(&body).map_err(|e| match e {
                                Refused::Limit => bad("decompress-limit"),
                                Refused::Bad => bad("a compressed record is not snappy"),
                            })?
                        } else {
                            body
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

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use pod_e2e::record::{Header, MAX_LENGTH, VERSION};

    use super::*;

    /// A middle node: a header that says [`MAX_LENGTH`], then no byte. It
    /// keeps the largest buffer that a read offered it.
    struct Hostile {
        header: Option<Vec<u8>>,
        offered: Arc<AtomicUsize>,
    }

    impl AsyncRead for Hostile {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            if let Some(header) = self.header.take() {
                buf.put_slice(&header);
                return Poll::Ready(Ok(()));
            }
            self.offered.fetch_max(buf.remaining(), Ordering::SeqCst);
            Poll::Pending
        }
    }

    #[tokio::test]
    async fn link_record_a_long_header_with_no_body_takes_no_buffer_of_its_length() {
        let header = Header {
            version: VERSION,
            kind: DATA,
            flags: 0,
            stream: 0,
            counter: 0,
            length: MAX_LENGTH as u32,
            pad: 0,
            reserved: 0,
        };
        let offered = Arc::new(AtomicUsize::new(0));
        let mut io = Hostile {
            header: Some(header.encode().to_vec()),
            offered: offered.clone(),
        };
        let read =
            tokio::time::timeout(std::time::Duration::from_millis(100), read_record(&mut io)).await;
        assert!(read.is_err(), "a record with no body came: {read:?}");
        let most = offered.load(Ordering::SeqCst);
        assert!(
            most > 0 && most <= BODY_STEP,
            "a read offered {most} bytes for a body that did not come"
        );
    }
}
