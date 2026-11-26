//! The inspector of `podnet tunnel --inspect` (T-244, GitHub #435).
//!
//! pod-cloudflare calls a capture sink ([`pod_cloudflare::capture::Capture`],
//! `Host::capture`) after each write of an HTTP exchange of an origin: the
//! request head that went to the origin, the payload of each body, the
//! answer head and the end. The [`Inspector`] keeps the last `keep`
//! exchanges in a ring in memory. The ring is lost at exit and is never
//! written to a file.
//!
//! * Each call of the sink takes one short lock and copies at most the cap
//!   of body bytes: it never holds a byte back from the visitor or the
//!   origin. A full ring drops the oldest exchange. An exchange that the
//!   ring dropped while it was open gets [`Refused`]: the crate stops the
//!   capture of it, and the copy goes on.
//! * Each body keeps at most `body` bytes, and the count of the bytes that
//!   it cut.
//! * The sink redacts by default ([`secret_name`]): the value of
//!   `Authorization`, `Proxy-Authorization`, `Cookie`, `Set-Cookie` and of
//!   each header or query parameter whose name names a secret is
//!   `[redacted]` in the stored copy. The forward keeps the real value.
//!   `--inspect-raw` keeps the values.
//! * `podnet tunnel inspect list|show|replay|clear` reads the ring over a
//!   unix socket of mode 0600 (`server.rs`), `replay` sends a stored
//!   request to its origin again (`replay.rs`), and `clear` empties the
//!   ring.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::UNIX_EPOCH;

use pod_cloudflare::capture::{Capture, Direction, End, Head, Line, Refused};
use serde_json::{json, Value};

use super::Host;
use crate::ingress::Ingress;

pub mod client;
mod replay;
mod server;

pub use server::{find_socket, peer_allowed, serve, served_path, socket_path, Served, SUFFIX};

/// The default count of exchanges in the ring.
pub const DEFAULT_KEEP: usize = 100;
/// The default cap of the bytes of one body.
pub const DEFAULT_BODY: usize = 64 * 1024;
/// The largest ring.
pub const MAX_KEEP: usize = 10_000;
/// The largest cap of one body.
pub const MAX_BODY: usize = 16 << 20;
/// The stored value of a secret.
pub const REDACTED: &str = "[redacted]";

/// The options of `--inspect`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    /// The count of exchanges in the ring (1 to [`MAX_KEEP`]).
    pub keep: usize,
    /// The cap of the bytes of each body (1 to [`MAX_BODY`]).
    pub body: usize,
    /// Keep the values of the secrets.
    pub raw: bool,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            keep: DEFAULT_KEEP,
            body: DEFAULT_BODY,
            raw: false,
        }
    }
}

/// The state of an exchange in the ring.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// The copy goes on.
    Open,
    /// The copy ended with no error.
    Complete,
    /// The copy ended with an error, or the sink stopped.
    Incomplete,
}

impl State {
    fn word(self) -> &'static str {
        match self {
            State::Open => "open",
            State::Complete => "complete",
            State::Incomplete => "incomplete",
        }
    }
}

/// The stored part of a body: at most the cap of bytes, and the count of
/// all bytes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Body {
    pub bytes: Vec<u8>,
    pub total: u64,
}

impl Body {
    /// Keep the part of `part` that fits under `cap`, and count all of it.
    pub fn push(&mut self, part: &[u8], cap: usize) {
        let room = cap.saturating_sub(self.bytes.len());
        self.bytes.extend_from_slice(&part[..part.len().min(room)]);
        self.total = self.total.saturating_add(part.len() as u64);
    }

    /// The count of the bytes that the cap cut.
    pub fn cut(&self) -> u64 {
        self.total.saturating_sub(self.bytes.len() as u64)
    }

    fn json(&self, headers: &[(String, String)]) -> Value {
        let mut doc = json!({
            "headers": headers.iter().map(|(n, v)| json!([n, v])).collect::<Vec<_>>(),
            "bytes": self.total,
            "cut": self.cut(),
        });
        match std::str::from_utf8(&self.bytes) {
            Ok(text) => doc["body"] = text.into(),
            Err(_) => {
                use base64::Engine as _;
                doc["body_base64"] = base64::engine::general_purpose::STANDARD
                    .encode(&self.bytes)
                    .into();
            }
        }
        doc
    }
}

/// One exchange in the ring.
#[derive(Clone, Debug)]
pub struct Exchange {
    /// The id in the ring: 1, 2, ... in the order of the first head.
    pub id: u64,
    /// The start, in milliseconds from the Unix epoch.
    pub started_ms: u64,
    pub method: String,
    /// The path and the query, with the secret parameters redacted.
    pub target: String,
    pub request_headers: Vec<(String, String)>,
    pub request: Body,
    /// The status of the answer, when an answer head came.
    pub status: Option<u16>,
    pub response_headers: Vec<(String, String)>,
    pub response: Body,
    /// The time to the end of both copies.
    pub elapsed_ms: Option<u64>,
    pub state: State,
    /// The exchange that this one replays.
    pub replayed_from: Option<u64>,
    /// The names of the headers and the query parameters that are redacted.
    pub redacted: Vec<String>,
    /// The key of the crate: the source of the sink and the exchange.
    key: (u64, u64),
}

impl Exchange {
    /// An upgrade (a WebSocket): a request with `Upgrade`, or the status 101.
    pub fn upgrade(&self) -> bool {
        self.status == Some(101) || header(&self.request_headers, "upgrade").is_some()
    }

    /// The line of `list`.
    pub fn summary(&self) -> Value {
        json!({
            "id": self.id,
            "started_ms": self.started_ms,
            "method": self.method,
            "target": self.target,
            "status": self.status,
            "elapsed_ms": self.elapsed_ms,
            "state": self.state.word(),
            "request_bytes": self.request.total,
            "response_bytes": self.response.total,
            "cut": self.request.cut() + self.response.cut(),
            "replayed_from": self.replayed_from,
            "upgrade": self.upgrade(),
        })
    }

    /// The document of `show`.
    pub fn detail(&self) -> Value {
        let mut doc = self.summary();
        doc["request"] = self.request.json(&self.request_headers);
        doc["response"] = self.response.json(&self.response_headers);
        doc["redacted"] = self.redacted.clone().into();
        doc
    }
}

/// The first value of the header `name` (in any case).
pub fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

/// The headers whose value is a secret by their full name.
const SECRET_NAMES: &[&str] = &[
    "authorization",
    "proxy-authorization",
    "cookie",
    "set-cookie",
];

/// The words that make a header or a query parameter a secret when its
/// name holds one of them (`X-Api-Key`, `X-Auth-Token`, `access_token`).
const SECRET_WORDS: &[&str] = &[
    "token",
    "secret",
    "key",
    "passw",
    "credential",
    "session",
    "auth",
    "signature",
    "cookie",
    "jwt",
];

/// True when the value of the header or the query parameter `name` is a
/// secret.
pub fn secret_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    SECRET_NAMES.contains(&lower.as_str()) || SECRET_WORDS.iter().any(|w| lower.contains(w))
}

/// The headers with each secret value redacted, and the names of the
/// redacted ones. With `raw`, the headers as they are.
pub fn redact_headers(
    headers: &[(String, String)],
    raw: bool,
    names: &mut Vec<String>,
) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| {
            if raw || !secret_name(name) {
                return (name.clone(), value.clone());
            }
            if !names.iter().any(|n| n.eq_ignore_ascii_case(name)) {
                names.push(name.clone());
            }
            (name.clone(), REDACTED.to_string())
        })
        .collect()
}

/// The target with the value of each secret query parameter redacted, and
/// the names of the redacted ones. With `raw`, the target as it is.
pub fn redact_target(target: &str, raw: bool, names: &mut Vec<String>) -> String {
    let Some((path, query)) = target.split_once('?').filter(|_| !raw) else {
        return target.to_string();
    };
    let parts: Vec<String> = query
        .split('&')
        .map(|pair| match pair.split_once('=') {
            Some((name, _)) if secret_name(name) => {
                if !names.iter().any(|n| n == name) {
                    names.push(name.to_string());
                }
                format!("{name}={REDACTED}")
            }
            _ => pair.to_string(),
        })
        .collect();
    format!("{path}?{}", parts.join("&"))
}

/// The ring of exchanges.
struct Ring {
    keep: usize,
    list: VecDeque<Exchange>,
    /// The open exchanges: the key of the crate and the id in the ring.
    open: HashMap<(u64, u64), u64>,
    next: u64,
}

impl Ring {
    fn get_mut(&mut self, id: u64) -> Option<&mut Exchange> {
        let first = self.list.front()?.id;
        let at = usize::try_from(id.checked_sub(first)?).ok()?;
        self.list.get_mut(at).filter(|e| e.id == id)
    }

    fn get(&self, id: u64) -> Option<&Exchange> {
        let first = self.list.front()?.id;
        let at = usize::try_from(id.checked_sub(first)?).ok()?;
        self.list.get(at).filter(|e| e.id == id)
    }

    /// The open exchange of `key`.
    fn of(&mut self, key: (u64, u64)) -> Option<&mut Exchange> {
        let id = *self.open.get(&key)?;
        self.get_mut(id)
    }

    /// Add `exchange` with the next id. A full ring drops the oldest.
    fn push(&mut self, mut exchange: Exchange) -> u64 {
        while self.list.len() >= self.keep {
            match self.list.pop_front() {
                Some(old) => {
                    self.open.remove(&old.key);
                }
                None => break,
            }
        }
        self.next += 1;
        exchange.id = self.next;
        self.open.insert(exchange.key, exchange.id);
        self.list.push_back(exchange);
        self.next
    }
}

/// The inspector of one tunnel: the ring, the options and what a replay
/// needs (the host of the tunnel and its ingress).
pub struct Inspector {
    options: Options,
    ring: Mutex<Ring>,
    sources: AtomicU64,
    /// The host of the tunnel (weak: the host holds the sink).
    host: Mutex<Weak<Host>>,
    ingress: Ingress,
}

impl Inspector {
    /// An inspector with an empty ring for the rules of `ingress`. A value
    /// of 0 in `options` is taken as 1.
    pub fn new(options: Options, ingress: Ingress) -> Arc<Inspector> {
        let options = Options {
            keep: options.keep.clamp(1, MAX_KEEP),
            body: options.body.clamp(1, MAX_BODY),
            raw: options.raw,
        };
        Arc::new(Inspector {
            ring: Mutex::new(Ring {
                keep: options.keep,
                list: VecDeque::new(),
                open: HashMap::new(),
                next: 0,
            }),
            options,
            sources: AtomicU64::new(0),
            host: Mutex::new(Weak::new()),
            ingress,
        })
    }

    pub fn options(&self) -> Options {
        self.options
    }

    fn ring(&self) -> std::sync::MutexGuard<'_, Ring> {
        self.ring.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A new sink for `Host::capture`. Each sink has its own source, so the
    /// ids of the exchanges of two runtimes do not mix. A sink of a replay
    /// names the exchange that it replays.
    pub fn sink(self: &Arc<Self>, replayed_from: Option<u64>) -> Arc<Sink> {
        Arc::new(Sink {
            inspector: self.clone(),
            source: self.sources.fetch_add(1, Ordering::Relaxed),
            replayed_from,
            last: Mutex::new(None),
        })
    }

    /// The host of the tunnel, for a replay. Each start of the tunnel
    /// gives its host.
    pub fn attach(&self, host: &Arc<Host>) {
        *self.host.lock().unwrap_or_else(|e| e.into_inner()) = Arc::downgrade(host);
    }

    fn host(&self) -> Option<Arc<Host>> {
        self.host
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .upgrade()
    }

    /// The summary of each exchange, the oldest first.
    pub fn list(&self) -> Vec<Value> {
        self.ring().list.iter().map(Exchange::summary).collect()
    }

    /// A copy of the exchange `id`, while the ring has it.
    pub fn get(&self, id: u64) -> Option<Exchange> {
        self.ring().get(id).cloned()
    }

    /// Empty the ring, and give the count of the exchanges that it held.
    /// The ids go on from the last one: a new exchange never gets the id
    /// of a cleared one. The crate stops the capture of an exchange that
    /// was open ([`Refused`]); the copy goes on.
    pub fn clear(&self) -> usize {
        let mut ring = self.ring();
        ring.open.clear();
        let count = ring.list.len();
        ring.list.clear();
        count
    }

    /// The document of `list`.
    pub fn list_doc(&self) -> Value {
        json!({
            "keep": self.options.keep,
            "body_cap": self.options.body,
            "raw": self.options.raw,
            "exchanges": self.list(),
        })
    }

    /// Send the stored request `id` to its origin again, and give the
    /// document of the new exchange (`replay.rs`).
    pub async fn replay(self: &Arc<Self>, id: u64) -> anyhow::Result<Value> {
        replay::replay(self, id).await
    }
}

/// The sink of one runtime of the tunnel (or of one replay) for
/// `Host::capture`.
pub struct Sink {
    inspector: Arc<Inspector>,
    source: u64,
    replayed_from: Option<u64>,
    /// The id in the ring of the last exchange that this sink began.
    last: Mutex<Option<u64>>,
}

impl Sink {
    /// The id in the ring of the last exchange that this sink began.
    pub fn last(&self) -> Option<u64> {
        *self.last.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn begin(&self, exchange: u64, method: &str, target: &str, headers: &[(String, String)]) {
        let raw = self.inspector.options.raw;
        let mut redacted = Vec::new();
        let target = redact_target(target, raw, &mut redacted);
        let request_headers = redact_headers(headers, raw, &mut redacted);
        let started_ms = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or_default();
        let id = self.inspector.ring().push(Exchange {
            id: 0,
            started_ms,
            method: method.to_string(),
            target,
            request_headers,
            request: Body::default(),
            status: None,
            response_headers: Vec::new(),
            response: Body::default(),
            elapsed_ms: None,
            state: State::Open,
            replayed_from: self.replayed_from,
            redacted,
            key: (self.source, exchange),
        });
        *self.last.lock().unwrap_or_else(|e| e.into_inner()) = Some(id);
    }
}

impl Capture for Sink {
    fn head(&self, exchange: u64, head: &Head) -> Result<(), Refused> {
        match &head.line {
            Line::Request { method, target } => {
                self.begin(exchange, method, target, &head.headers);
                Ok(())
            }
            Line::Response { status } => {
                let raw = self.inspector.options.raw;
                let mut ring = self.inspector.ring();
                let record = ring.of((self.source, exchange)).ok_or(Refused)?;
                let mut names = std::mem::take(&mut record.redacted);
                record.response_headers = redact_headers(&head.headers, raw, &mut names);
                record.redacted = names;
                record.status = Some(*status);
                Ok(())
            }
        }
    }

    fn body(&self, exchange: u64, direction: Direction, bytes: &[u8]) -> Result<(), Refused> {
        let cap = self.inspector.options.body;
        let mut ring = self.inspector.ring();
        let record = ring.of((self.source, exchange)).ok_or(Refused)?;
        match direction {
            Direction::Request => record.request.push(bytes, cap),
            Direction::Response => record.response.push(bytes, cap),
        }
        Ok(())
    }

    fn end(&self, exchange: u64, end: &End) {
        let mut ring = self.inspector.ring();
        let key = (self.source, exchange);
        if let Some(record) = ring.of(key) {
            record.status = end.status.or(record.status);
            record.elapsed_ms = Some(end.elapsed.as_millis() as u64);
            record.state = if end.complete {
                State::Complete
            } else {
                State::Incomplete
            };
        }
        ring.open.remove(&key);
    }
}

impl Drop for Sink {
    /// An exchange of this sink that never got its end (the crate dropped
    /// the sink for it) is incomplete.
    fn drop(&mut self) {
        let mut ring = self.inspector.ring();
        let keys: Vec<(u64, u64)> = ring
            .open
            .keys()
            .filter(|(source, _)| *source == self.source)
            .copied()
            .collect();
        for key in keys {
            if let Some(record) = ring.of(key) {
                record.state = State::Incomplete;
            }
            ring.open.remove(&key);
        }
    }
}

/// The inspector of this process, for the host of the next tunnel
/// ([`super::hosted`]). `podnet tunnel --inspect` sets it.
static INSTALLED: Mutex<Option<Arc<Inspector>>> = Mutex::new(None);

/// Give `inspector` to each tunnel that this process starts after this
/// call.
pub fn install(inspector: Arc<Inspector>) {
    *INSTALLED.lock().unwrap_or_else(|e| e.into_inner()) = Some(inspector);
}

/// The inspector of this process, when `--inspect` set one.
pub fn installed() -> Option<Arc<Inspector>> {
    INSTALLED.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

#[cfg(test)]
mod tests;
