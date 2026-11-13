//! The two tasks of a sealed stream: the sender seals the bytes that the
//! application wrote, the receiver opens the records that came.

use std::collections::VecDeque;
use std::io;
use std::sync::Arc;
use std::time::Instant;

use pod_core::entropy::OsEntropy;
use pod_e2e::record::{record_len, Fault, Opened, HEADER};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, DuplexStream};
use tokio::sync::mpsc;

use super::pack::{self, Refused};
use super::{lock, Command, Shared, COMPRESSED, DATA, FIN};

/// The bytes between the application and the sender.
pub(super) const PIPE: usize = 256 * 1024;

/// The most payload bytes of one record.
const CHUNK: usize = 32 * 1024;

/// The bytes of sent payloads that the sender keeps for a RESUME.
const LOG_LIMIT: usize = 1024 * 1024;

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
pub(super) async fn send<W: AsyncWrite + Unpin>(
    shared: Arc<Shared>,
    mut app: DuplexStream,
    mut io: W,
    mut commands: mpsc::UnboundedReceiver<Command>,
) {
    let _ = run_send(&shared, &mut app, &mut io, &mut commands).await;
    let _ = io.shutdown().await;
}

async fn run_send<W: AsyncWrite + Unpin>(
    shared: &Shared,
    app: &mut DuplexStream,
    io: &mut W,
    commands: &mut mpsc::UnboundedReceiver<Command>,
) -> io::Result<()> {
    let mut log = Log::default();
    let mut buf = vec![0u8; CHUNK];
    let mut commanded = true;
    loop {
        tokio::select! {
            read = app.read(&mut buf) => {
                let n = read?;
                if n == 0 {
                    return put(shared, Some(&mut log), io, FIN, 0, &[]).await;
                }
                let packed = shared.options.compress.then(|| pack::pack(&buf[..n])).flatten();
                match &packed {
                    Some(p) => put(shared, Some(&mut log), io, DATA, COMPRESSED, p).await?,
                    None => put(shared, Some(&mut log), io, DATA, 0, &buf[..n]).await?,
                }
            }
            command = commands.recv(), if commanded => match command {
                // No end can command any more: only the application is left.
                None => commanded = false,
                Some(Command::Stop) => return Ok(()),
                Some(Command::Resume) => {
                    let record = lock(&shared.records).resume(&mut OsEntropy).map_err(bad)?;
                    shared.stats.wire_out.fetch_add(record.len() as u64, std::sync::atomic::Ordering::SeqCst);
                    io.write_all(&record).await?;
                }
                Some(Command::Resend(next)) => {
                    let rows = log.from(next).ok_or_else(|| {
                        bad("the records that the peer misses are no longer kept")
                    })?;
                    for (_, kind, flags, payload) in rows {
                        put(shared, Some(&mut log), io, kind, flags, &payload).await?;
                    }
                }
            },
        }
    }
}

/// Open the records of `io` and give the bytes to the application. A fault
/// ends the stream: the reader gets the error, and the sender stops.
pub(super) async fn receive<R: AsyncRead + Unpin>(
    shared: Arc<Shared>,
    mut io: R,
    out: mpsc::Sender<io::Result<Vec<u8>>>,
    commands: mpsc::UnboundedSender<Command>,
) {
    if let Err(e) = run_receive(&shared, &mut io, &out, &commands).await {
        let _ = commands.send(Command::Stop);
        let _ = out.send(Err(e)).await;
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

async fn run_receive<R: AsyncRead + Unpin>(
    shared: &Shared,
    io: &mut R,
    out: &mpsc::Sender<io::Result<Vec<u8>>>,
    commands: &mpsc::UnboundedSender<Command>,
) -> io::Result<()> {
    use std::sync::atomic::Ordering::SeqCst;
    let cut = || {
        io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "the stream was cut: it ended with no FIN record",
        )
    };
    loop {
        // The reader is gone: nobody wants the bytes.
        let read = tokio::select! {
            () = out.closed() => return Ok(()),
            read = read_record(io) => read,
        };
        let record = read?.ok_or_else(cut)?;
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
                        if !payload.is_empty() && out.send(Ok(payload)).await.is_err() {
                            return Ok(());
                        }
                    }
                    FIN => return Ok(()),
                    other => return Err(bad(format!("a record has the type {other}"))),
                }
            }
            Ok(Opened::Resume { next }) => {
                let _ = commands.send(Command::Resend(next));
            }
            Err(Fault::Replayed(_) | Fault::Stale(_)) => {
                shared.stats.replayed.fetch_add(1, SeqCst);
            }
            Err(Fault::Resume(_)) => {
                shared.stats.refused.fetch_add(1, SeqCst);
            }
            Err(fault) => return Err(bad(fault)),
        }
    }
}
