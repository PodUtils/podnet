//! The flow record of `--flow-log FILE` (T-157): one JSON line for each
//! decision (at dial time) and one at the close of each relayed flow.
//!
//! ```text
//! {"time":1731234567.123,"event":"dial","flow":1,"proto":"tcp","target":"example.com:443","rule":"default","chain":["unix:/run/p.sock"],"verdict":"allow"}
//! {"time":1731234568.456,"event":"close","flow":1,"proto":"tcp","target":"example.com:443","bytes_out":517,"bytes_in":4096}
//! ```
//!
//! `bytes_out` goes from the program to the upstream; `bytes_in` comes
//! back. `-v` stays the log for humans.

use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::task::{Context as TaskContext, Poll};

use anyhow::{Context, Result};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use crate::upstream::{Proxy, Selected};

/// The file of the record, when `--flow-log` is given.
static LOG: OnceLock<Mutex<std::fs::File>> = OnceLock::new();
/// The number of the last flow.
static FLOWS: AtomicU64 = AtomicU64::new(0);
/// The datagram destinations of each socket that have a line already.
static SEEN: OnceLock<Mutex<std::collections::HashSet<(u64, String)>>> = OnceLock::new();
/// Above this many entries, [`SEEN`] starts again.
const MAX_SEEN: usize = 16_384;

/// Open `path` for the record: append, create with mode 0600.
pub(crate) fn open(path: &std::path::Path) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("open the flow log {}", path.display()))?;
    let _ = LOG.set(Mutex::new(file));
    Ok(())
}

/// True when the record is on.
pub(crate) fn on() -> bool {
    LOG.get().is_some()
}

fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| (d.as_millis() as f64) / 1000.0)
        .unwrap_or(0.0)
}

fn write(line: serde_json::Value) {
    use std::io::Write;
    if let Some(file) = LOG.get() {
        let mut text = line.to_string();
        text.push('\n');
        if let Ok(mut file) = file.lock() {
            let _ = file.write_all(text.as_bytes());
        }
    }
}

/// The verdict of a decision: `allow` (through the chain), `direct` (no
/// proxy) or `reject`.
fn verdict(chain: Option<&[Proxy]>) -> &'static str {
    match chain {
        None => "reject",
        Some(chain)
            if chain
                .iter()
                .all(|p| p.kind == crate::upstream::ProxyKind::Direct) =>
        {
            "direct"
        }
        Some(_) => "allow",
    }
}

/// Write the line of a decision for `target`, with the rule that made it
/// (`rule`: a rule text, `default` or `loopback`). Return the number of
/// the flow, for its close line. `None` when the record is off.
pub(crate) fn dial(proto: &str, target: &str, rule: &str, chain: Option<&[Proxy]>) -> Option<u64> {
    if !on() {
        return None;
    }
    let flow = FLOWS.fetch_add(1, Ordering::Relaxed) + 1;
    let hops: Vec<String> = chain
        .unwrap_or_default()
        .iter()
        .map(Proxy::endpoint)
        .collect();
    write(serde_json::json!({
        "time": now(),
        "event": "dial",
        "flow": flow,
        "proto": proto,
        "target": target,
        "rule": rule,
        "chain": hops,
        "verdict": verdict(chain),
    }));
    Some(flow)
}

/// [`dial`] with the decision of the rules.
pub(crate) fn decided(proto: &str, target: &str, picked: &Selected<'_>) -> Option<u64> {
    dial(proto, target, picked.rule_name(), picked.chain())
}

/// [`decided`] for a datagram of the socket `key`: one line for each
/// destination of each socket.
pub(crate) fn datagram(key: u64, target: &str, picked: &Selected<'_>) {
    if !on() {
        return;
    }
    let seen = SEEN.get_or_init(Default::default);
    let first = match seen.lock() {
        Ok(mut seen) => {
            if seen.len() >= MAX_SEEN {
                seen.clear();
            }
            seen.insert((key, target.to_string()))
        }
        Err(_) => true,
    };
    if first {
        decided("udp", target, picked);
    }
}

/// The byte counts of a relayed flow.
#[derive(Clone, Default)]
pub(crate) struct Counts {
    out: Arc<AtomicU64>,
    back: Arc<AtomicU64>,
}

impl Counts {
    /// Write the close line of `flow`.
    pub(crate) fn close(&self, flow: Option<u64>, proto: &str, target: &str) {
        let Some(flow) = flow else { return };
        write(serde_json::json!({
            "time": now(),
            "event": "close",
            "flow": flow,
            "proto": proto,
            "target": target,
            "bytes_out": self.out.load(Ordering::Relaxed),
            "bytes_in": self.back.load(Ordering::Relaxed),
        }));
    }

    /// Add `n` bytes from the program.
    pub(crate) fn add_out(&self, n: usize) {
        self.out.fetch_add(n as u64, Ordering::Relaxed);
    }

    /// Add `n` bytes to the program.
    pub(crate) fn add_in(&self, n: usize) {
        self.back.fetch_add(n as u64, Ordering::Relaxed);
    }

    /// The side of the program: the bytes that it reads from this stream go
    /// out.
    pub(crate) fn program<S>(&self, inner: S) -> Counted<S> {
        Counted {
            inner,
            read: self.out.clone(),
        }
    }
}

/// A stream that counts the bytes that are read from it.
pub(crate) struct Counted<S> {
    inner: S,
    read: Arc<AtomicU64>,
}

impl<S: AsyncRead + Unpin> AsyncRead for Counted<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let poll = Pin::new(&mut self.inner).poll_read(cx, buf);
        let n = buf.filled().len() - before;
        self.read.fetch_add(n as u64, Ordering::Relaxed);
        poll
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Counted<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

/// The buffer of each direction of a relay (T-151): 64 KiB moves about
/// 1.5 times the bytes of the 8 KiB default of tokio on a loopback test.
pub(crate) const RELAY_BUFFER: usize = 64 * 1024;

/// Copy both ways between the program side `program` and `upstream` until
/// both end, and write the close line of `flow`.
pub(crate) async fn relay<A, B>(program: A, upstream: B, flow: Option<u64>, target: &str)
where
    A: AsyncRead + AsyncWrite + Unpin,
    B: AsyncRead + AsyncWrite + Unpin,
{
    let counts = Counts::default();
    let _ = copy_counted(program, upstream, &counts).await;
    counts.close(flow, "tcp", target);
}

/// Copy both ways with [`RELAY_BUFFER`], and add the bytes to `counts`.
async fn copy_counted<A, B>(program: A, upstream: B, counts: &Counts) -> std::io::Result<(u64, u64)>
where
    A: AsyncRead + AsyncWrite + Unpin,
    B: AsyncRead + AsyncWrite + Unpin,
{
    let mut program = counts.program(program);
    let mut upstream = Counted {
        inner: upstream,
        read: counts.back.clone(),
    };
    tokio::io::copy_bidirectional_with_sizes(
        &mut program,
        &mut upstream,
        RELAY_BUFFER,
        RELAY_BUFFER,
    )
    .await
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    // T-151: a relay with the large buffers keeps the byte totals.
    #[test]
    fn the_relay_keeps_the_byte_totals() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let (program, mut client) = tokio::io::duplex(1 << 20);
            let (upstream, mut server) = tokio::io::duplex(1 << 20);
            let counts = super::Counts::default();
            let relay = super::copy_counted(program, upstream, &counts);
            let peers = async move {
                let out = vec![7u8; 300_000];
                client.write_all(&out).await.unwrap();
                client.shutdown().await.unwrap();
                let mut got = Vec::new();
                server.read_to_end(&mut got).await.unwrap();
                assert_eq!(got.len(), out.len());
                server.write_all(&vec![1u8; 200_001]).await.unwrap();
                server.shutdown().await.unwrap();
                let mut back = Vec::new();
                client.read_to_end(&mut back).await.unwrap();
                assert_eq!(back.len(), 200_001);
            };
            let (copied, ()) = tokio::join!(relay, peers);
            assert_eq!(copied.unwrap(), (300_000, 200_001));
            let load =
                |a: &std::sync::atomic::AtomicU64| a.load(std::sync::atomic::Ordering::Relaxed);
            assert_eq!(load(&counts.out), 300_000);
            assert_eq!(load(&counts.back), 200_001);
        });
    }
}
