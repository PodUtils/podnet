//! The control socket of a link instance (T-267, T-266).
//!
//! An instance of `podnet link listen` or `podnet link connect` holds the
//! links that it runs in a [`Hub`]. [`Control`] serves the hub on a unix
//! socket. The socket follows the discipline of the router
//! (`router::control`):
//!
//! * the socket has the mode 0600 ([`bind_private`]), and so has the token
//!   file beside it (`PATH.token`);
//! * the token is 32 random bytes in hex, new for each run, and every
//!   request must carry it (a bad token exits 77 at the client);
//! * the user of the peer (`SO_PEERCRED`) is the user of the instance, or
//!   root ([`peer_allowed`]).
//!
//! The wire is one JSON object for each line, not the HTTP of the router.
//! A request is `{"token":T,"verb":V,...}` of 4 KiB at most; a response is
//! `{"ok":true,...}` or `{"ok":false,"code":EXIT,"error":TEXT}` of 64 KiB
//! at most. 16 clients are served at one time, and a client has one
//! command at a time. There is no TCP listener and no HTTP endpoint.
//!
//! The verbs:
//!
//! * `status`: the fingerprint of the instance and one row for each held
//!   link (carrier, address, role, peer fingerprint, streams, counters);
//! * `peer` (`"peer":FP`): `ok` when the instance holds a link to a peer
//!   with this fingerprint, else 77 (the links go to other peers) or 69
//!   (it holds no link);
//! * `open` (`"peer":FP,"service":NAME`): open one stream of the link to
//!   that peer, ask it for the service, and on `ok` turn the connection
//!   into the bytes of the stream. This is how `peer:FP/SERVICE` of the
//!   network registry reaches a link of another process.
//! * `accept` (`"peer":FP,"service":NAME,"target":"unix:/ABS/PATH"`): serve
//!   the streams for NAME that the peer opens on the socket at PATH, while
//!   the client keeps the connection (`accept.rs`). This is how `podnet
//!   serve --ingress peer:FP/SERVICE` takes a stream that the far end opens.
//! * `close-link` (`"link":ID`), `close-stream` (`"link":ID,"stream":ID`)
//!   and `select` (`"link":ID|null`) change a link (T-307, `verbs.rs`).
//!   Each one answers the status document after the change. `reconnect`
//!   and `rotate` (`"link":ID`) make a new link to the same peer; a listener
//!   answers 69 (`redial.rs`).
//! * `routes`: the best route of each origin that the announcements
//!   (`--announce`, T-285) taught this instance; 69 without `--announce`;
//! * `mesh`: the nodes and the edges of the mesh and its counters
//!   (T-289, `link_announce/status.rs`). The request may hold `max_nodes`
//!   and `max_edges` (lower limits). The answer may be 256 KiB; 69 without
//!   `--announce`.
//!
//! * `dump`: the state of the instance for a bug report (T-309,
//!   `dump.rs`). Its answer may be 256 KiB; the other answers are 64 KiB at
//!   most.

use std::collections::BTreeMap;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, Context};
use pod_core::entropy::{Entropy, OsEntropy};
use pod_identity::PublicKey;
use podnet_netns::listen::bind_private;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::sync::{Notify, Semaphore};
use tokio::task::JoinHandle;

use crate::exit::{self, Fault};
use crate::identity::AllowList;
use crate::link::{Link, ResetCode, Stream};

mod accept;
mod dump;
mod redial;
mod verbs;
pub use redial::{Dialing, Redial};

/// The most bytes of a request line.
pub const REQUEST_LIMIT: usize = 4096;
/// The most bytes of a response line.
pub const RESPONSE_LIMIT: usize = 64 * 1024;
/// The most bytes of the answer to `dump` (T-309).
pub const DUMP_LIMIT: usize = 256 * 1024;
/// The most clients at one time.
pub const MAX_CLIENTS: usize = 16;
/// The time that a client can take to send a line, and the time that an
/// `open` can take to reach the service.
const LIMIT: Duration = Duration::from_secs(10);
/// The name of the variable that names one socket for `peer:FP/SERVICE`.
pub const CONTROL_VARIABLE: &str = "PODNET_LINK_CONTROL";

/// True when a peer of the socket may use it: the user of the instance, or
/// root. The mode of the socket (0600) is the first gate; this is the
/// second. It is the rule of the router.
pub fn peer_allowed(peer: u32, own: u32) -> bool {
    crate::router::control::peer_allowed(peer, own)
}

/// An error answer: the exit code that the client takes, and one line.
#[derive(Debug, Clone)]
struct Answer {
    code: i32,
    text: String,
}

impl Answer {
    fn new(code: i32, text: impl Into<String>) -> Answer {
        Answer {
            code,
            text: text.into(),
        }
    }

    fn of(err: &anyhow::Error) -> Answer {
        let code = exit::code_of(err);
        let code = if code == exit::FAILURE {
            exit::UNAVAILABLE
        } else {
            code
        };
        Answer::new(code, format!("{err:#}"))
    }

    fn json(&self) -> Value {
        json!({"ok": false, "code": self.code, "error": self.text})
    }
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// One link that an instance holds.
pub struct Held {
    id: u64,
    link: Arc<Link>,
    peer: PublicKey,
    carrier: String,
    address: String,
    role: &'static str,
    since: u64,
    opened: AtomicU64,
    accepted: AtomicU64,
    /// The streams that the allow list refused (T-290): a service the peer
    /// has no grant for, and a forwarding the peer has no grant for.
    refused_service: AtomicU64,
    refused_forward: AtomicU64,
    /// The streams that a task serves and a verb can end: the id, and the
    /// signal that the task waits for.
    streams: Mutex<BTreeMap<u32, Arc<Notify>>>,
}

/// The most stream ids that a row of `status` lists.
const STREAM_IDS: usize = 64;

/// A stream that a verb can end. The task that serves the stream waits on
/// [`Tracked::cancelled`], and resets the stream when it returns. The drop
/// takes the stream out of the table.
pub struct Tracked {
    held: Arc<Held>,
    id: u32,
    cancel: Arc<Notify>,
}

impl Tracked {
    /// Return when `close-stream` names this stream. A signal that came
    /// before the call is not lost.
    pub async fn cancelled(&self) {
        self.cancel.notified().await;
    }
}

impl Drop for Tracked {
    fn drop(&mut self) {
        self.held
            .streams
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.id);
    }
}

impl Held {
    fn track(self: &Arc<Held>, id: u32) -> Tracked {
        let cancel = Arc::new(Notify::new());
        self.streams
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, cancel.clone());
        Tracked {
            held: self.clone(),
            id,
            cancel,
        }
    }

    fn stream_ids(&self) -> Vec<u32> {
        let streams = self.streams.lock().unwrap_or_else(|e| e.into_inner());
        streams.keys().copied().take(STREAM_IDS).collect()
    }

    fn row(&self) -> Value {
        json!({
            "stream_ids": self.stream_ids(),
            "id": self.id,
            "carrier": self.carrier,
            "address": self.address,
            "role": self.role,
            "peer": self.peer.display_fingerprint(),
            "state": if self.link.ended().is_none() { "up" } else { "ended" },
            "streams": self.link.streams(),
            "rtt_ms": self.link.rtt().map(|d| d.as_millis() as u64),
            "opened": self.opened.load(Ordering::Relaxed),
            "accepted": self.accepted.load(Ordering::Relaxed),
            "refused_service": self.refused_service.load(Ordering::Relaxed),
            "refused_forward": self.refused_forward.load(Ordering::Relaxed),
            "since": self.since,
        })
    }
}

/// The links of one instance.
pub struct Hub {
    fingerprint: String,
    held: Mutex<Vec<Arc<Held>>>,
    next: AtomicU64,
    /// The link that serves new streams (`select`), when one is pinned.
    selected: Mutex<Option<u64>>,
    /// The last errors, for the verb `dump`.
    errors: Mutex<dump::ErrorRing>,
    /// The route announcements, when the instance runs `--announce`.
    mesh: std::sync::OnceLock<Arc<crate::link_announce::Mesh>>,
    /// The services that clients of the verb `accept` registered.
    accepted: Mutex<accept::Table>,
    /// The way to dial again, for `reconnect` and `rotate` (T-307).
    redial: redial::State,
}

/// The place of a link in a [`Hub`]. Its drop takes the link out.
pub struct Slot {
    hub: Arc<Hub>,
    held: Arc<Held>,
}

impl Slot {
    /// Count a stream that the peer opened.
    pub fn count_accepted(&self) {
        self.held.accepted.fetch_add(1, Ordering::Relaxed);
    }

    /// The counters of the streams that the allow list refuses.
    pub fn refusals(&self) -> Refusals {
        Refusals(self.held.clone())
    }

    /// Put a stream of this link in the table that `close-stream` reads.
    /// The task that serves the stream waits on the result.
    pub fn track(&self, stream: &Stream) -> Tracked {
        self.held.track(stream.id())
    }
}

/// A handle to count the streams of a link that its allow list refused.
#[derive(Clone)]
pub struct Refusals(Arc<Held>);

impl Refusals {
    /// Count a stream for a service that the peer has no grant for.
    pub fn service(&self) {
        self.0.refused_service.fetch_add(1, Ordering::Relaxed);
    }

    /// Count a stream that the peer wanted relayed without a grant.
    pub fn forward(&self) {
        self.0.refused_forward.fetch_add(1, Ordering::Relaxed);
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        let mut held = self.hub.held.lock().unwrap_or_else(|e| e.into_inner());
        held.retain(|h| h.id != self.held.id);
        let mut pinned = self.hub.selected.lock().unwrap_or_else(|e| e.into_inner());
        if *pinned == Some(self.held.id) {
            *pinned = None;
        }
    }
}

impl Hub {
    /// A hub for the install with this fingerprint.
    pub fn new(fingerprint: &str) -> Arc<Hub> {
        Arc::new(Hub {
            fingerprint: fingerprint.to_string(),
            held: Mutex::new(Vec::new()),
            next: AtomicU64::new(1),
            selected: Mutex::new(None),
            errors: Mutex::new(Default::default()),
            mesh: std::sync::OnceLock::new(),
            accepted: Mutex::new(Default::default()),
            redial: Default::default(),
        })
    }

    /// Give the verb `routes` the mesh of an instance that runs `--announce`.
    pub fn attach_mesh(&self, mesh: Arc<crate::link_announce::Mesh>) {
        let _ = self.mesh.set(mesh);
    }

    /// Hold `link`, whose peer proved `peer`, until the slot drops.
    pub fn hold(
        self: &Arc<Hub>,
        link: Arc<Link>,
        peer: PublicKey,
        carrier: &str,
        address: &str,
        role: &'static str,
    ) -> Slot {
        let held = Arc::new(Held {
            id: self.next.fetch_add(1, Ordering::Relaxed),
            link,
            peer,
            carrier: carrier.to_string(),
            address: address.to_string(),
            role,
            since: now(),
            opened: AtomicU64::new(0),
            accepted: AtomicU64::new(0),
            refused_service: AtomicU64::new(0),
            refused_forward: AtomicU64::new(0),
            streams: Mutex::new(BTreeMap::new()),
        });
        self.held
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(held.clone());
        Slot {
            hub: self.clone(),
            held,
        }
    }

    fn rows(&self) -> Vec<Arc<Held>> {
        self.held.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// The document of the verb `status`. It holds fingerprints, never a
    /// seed and never the token.
    pub fn status(&self) -> Value {
        json!({
            "ok": true,
            "verb": "status",
            "fingerprint": self.fingerprint,
            "pid": std::process::id(),
            "selected": *self.selected.lock().unwrap_or_else(|e| e.into_inner()),
            "links": self.rows().iter().map(|h| h.row()).collect::<Vec<_>>(),
        })
    }

    /// The link to a peer that `peer` admits. No link up: 69. Links up to
    /// other peers only: 77, before any service opens.
    fn find(&self, peer: &AllowList) -> Result<Arc<Held>, Answer> {
        let up: Vec<Arc<Held>> = self
            .rows()
            .into_iter()
            .filter(|h| h.link.ended().is_none())
            .collect();
        if up.is_empty() {
            return Err(Answer::new(
                exit::UNAVAILABLE,
                "this instance holds no link that is up",
            ));
        }
        let pinned = *self.selected.lock().unwrap_or_else(|e| e.into_inner());
        let pinned = up
            .iter()
            .find(|h| Some(h.id) == pinned && peer.admits(&h.peer))
            .cloned();
        pinned
            .into_iter()
            .chain(up)
            .find(|h| peer.admits(&h.peer))
            .ok_or_else(|| {
                Answer::new(
                    exit::NO_PERM,
                    "no link of this instance goes to a peer with this fingerprint",
                )
            })
    }

    async fn open(
        &self,
        peer: &AllowList,
        service: &str,
    ) -> Result<(Stream, String, Tracked), Answer> {
        crate::link_service::check_name(service).map_err(|e| Answer::of(&e))?;
        let held = self.find(peer)?;
        let mut stream = held
            .link
            .open()
            .map_err(|e| Answer::of(&crate::link::fault(e)))?;
        held.opened.fetch_add(1, Ordering::Relaxed);
        let tracked = held.track(stream.id());
        match tokio::time::timeout(LIMIT, crate::link_service::request(&mut stream, service)).await
        {
            Ok(Ok(())) => Ok((stream, held.peer.display_fingerprint(), tracked)),
            Ok(Err(e)) => Err(Answer::of(&e)),
            Err(_) => Err(Answer::new(
                exit::TIMEOUT,
                format!(
                    "the peer did not answer the service {service:?} in {} s",
                    LIMIT.as_secs()
                ),
            )),
        }
    }

    /// One request that carries a good token. Add the verbs here.
    async fn verb(self: &Arc<Hub>, request: &Value) -> Result<Reply, Answer> {
        let name = request["verb"].as_str().unwrap_or_default();
        match name {
            "status" => Ok(Reply::Json(self.status())),
            "peer" | "open" => {
                let peer = peer_list(request["peer"].as_str().unwrap_or_default())
                    .map_err(|e| Answer::of(&e))?;
                if name == "peer" {
                    let held = self.find(&peer)?;
                    return Ok(Reply::Json(
                        json!({"ok": true, "verb": "peer", "peer": held.peer.display_fingerprint()}),
                    ));
                }
                let service = request["service"].as_str().unwrap_or_default();
                let (stream, fp, tracked) = self.open(&peer, service).await?;
                Ok(Reply::Pipe(
                    stream,
                    json!({"ok": true, "verb": "open", "peer": fp}),
                    tracked,
                ))
            }
            "accept" => self.accept(request),
            "close-link" => self.close_link(request).await,
            "close-stream" => self.close_stream(request).await,
            "select" => self.select(request),
            "dump" => Ok(self.dump_reply()),
            "routes" => match self.mesh.get() {
                Some(mesh) => Ok(Reply::Json(mesh.routes(crate::link_announce::now_ms()))),
                None => Err(Answer::new(
                    exit::UNAVAILABLE,
                    "routes: this instance runs without --announce",
                )),
            },
            "mesh" => match self.mesh.get() {
                Some(mesh) => {
                    let limits = crate::link_announce::ViewLimits::from_request(request)
                        .map_err(|e| Answer::new(exit::USAGE, format!("mesh: {e}")))?;
                    Ok(Reply::Big(
                        mesh.view(crate::link_announce::now_ms(), limits),
                    ))
                }
                None => Err(Answer::new(
                    exit::UNAVAILABLE,
                    "mesh: this instance runs without --announce",
                )),
            },
            "reconnect" => self.reconnect(request).await,
            "rotate" => self.rotate(request).await,
            other => Err(Answer::new(
                exit::USAGE,
                format!(
                    "{other:?} is no verb of the control socket: use {}",
                    verbs::LIST
                ),
            )),
        }
    }
}

/// The answer to a request: a document, or a document and then the bytes
/// of a stream.
enum Reply {
    Json(Value),
    /// A document that may exceed [`RESPONSE_LIMIT`], up to [`DUMP_LIMIT`].
    Big(Value),
    Pipe(Stream, Value, Tracked),
    /// A document, then the connection stays until the client ends it.
    Hold(Value, accept::Claim),
}

/// The allow list of one fingerprint (the peer of `peer:FP/SERVICE`). A
/// text that is no one fingerprint exits 64.
pub fn peer_list(fingerprint: &str) -> anyhow::Result<AllowList> {
    let bad = |why: String| exit::fault(Fault::Usage, anyhow!("peer {fingerprint:?}: {why}"));
    if fingerprint.is_empty() || fingerprint.contains(char::is_whitespace) {
        return Err(bad("give one fingerprint".into()));
    }
    let list = AllowList::parse(fingerprint).map_err(|e| bad(format!("{e:#}")))?;
    if list.len() != 1 {
        return Err(bad("give one fingerprint".into()));
    }
    Ok(list)
}

/// The path of the token file of a socket.
pub fn token_path(socket: &Path) -> PathBuf {
    let mut name = socket.as_os_str().to_os_string();
    name.push(".token");
    PathBuf::from(name)
}

fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

type Gate = Arc<dyn Fn(u32) -> bool + Send + Sync>;

/// The control socket of a hub, and the task that serves it. The drop
/// stops the task and removes the socket and the token file.
pub struct Control {
    path: PathBuf,
    token_file: PathBuf,
    token: String,
    task: JoinHandle<()>,
}

impl Control {
    /// Serve `hub` at `path`. A path that a live socket holds exits 69.
    pub fn start(path: &Path, hub: Arc<Hub>) -> anyhow::Result<Control> {
        // SAFETY: geteuid has no precondition.
        let own = unsafe { libc::geteuid() };
        Control::start_gated(path, hub, Arc::new(move |peer| peer_allowed(peer, own)))
    }

    /// [`Control::start`] with the test `allow` for the user id of a peer.
    pub fn start_gated(path: &Path, hub: Arc<Hub>, allow: Gate) -> anyhow::Result<Control> {
        let mut raw = [0u8; 32];
        OsEntropy.fill(&mut raw).map_err(|e| {
            exit::fault(
                Fault::OsError,
                anyhow!("no random bytes for the token: {e}"),
            )
        })?;
        let token: String = raw.iter().map(|b| format!("{b:02x}")).collect();
        // No report, dump or log line of this process may hold the token.
        crate::report::hide(&token);
        if path.exists() && std::os::unix::net::UnixStream::connect(path).is_err() {
            // A socket that nobody serves: the leftover of a crash.
            let _ = std::fs::remove_file(path);
        }
        let listener = bind_private(path)
            .with_context(|| format!("the control socket {}", path.display()))
            .map_err(crate::net::bind_fault)?;
        let token_file = token_path(path);
        let written = std::fs::remove_file(&token_file)
            .or_else(|e| {
                (e.kind() == std::io::ErrorKind::NotFound)
                    .then_some(())
                    .ok_or(e)
            })
            .and_then(|()| {
                std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(&token_file)
            })
            .and_then(|mut f| std::io::Write::write_all(&mut f, token.as_bytes()));
        if let Err(e) = written {
            let _ = std::fs::remove_file(path);
            return Err(exit::fault(
                Fault::OsError,
                anyhow!("{}: {e}", token_file.display()),
            ));
        }
        let task = tokio::spawn(serve(listener, hub, token.clone(), allow));
        Ok(Control {
            path: path.to_path_buf(),
            token_file,
            token,
            task,
        })
    }

    /// The path of the socket.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The token of this run (the instance keeps it; do not print it).
    pub fn token(&self) -> &str {
        &self.token
    }
}

impl Drop for Control {
    fn drop(&mut self) {
        self.task.abort();
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_file(&self.token_file);
    }
}

async fn serve(listener: tokio::net::UnixListener, hub: Arc<Hub>, token: String, allow: Gate) {
    let slots = Arc::new(Semaphore::new(MAX_CLIENTS));
    let token: Arc<str> = token.into();
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            tokio::time::sleep(Duration::from_millis(50)).await;
            continue;
        };
        let permitted = stream.peer_cred().map(|c| allow(c.uid())).unwrap_or(false);
        let (hub, token) = (hub.clone(), token.clone());
        match slots.clone().try_acquire_owned() {
            Ok(permit) => {
                tokio::spawn(session(stream, hub, token, permitted, permit));
            }
            Err(_) => {
                tokio::spawn(refuse(stream));
            }
        }
    }
}

enum Line {
    Got(Vec<u8>),
    Long,
    Gone,
}

/// Read one line of at most [`REQUEST_LIMIT`] bytes. `pending` keeps the
/// bytes after the line.
async fn read_line(stream: &mut UnixStream, pending: &mut Vec<u8>) -> std::io::Result<Line> {
    let mut buf = [0u8; 1024];
    loop {
        if let Some(at) = pending.iter().position(|b| *b == b'\n') {
            let mut line: Vec<u8> = pending.drain(..=at).collect();
            line.pop();
            return Ok(if line.len() > REQUEST_LIMIT {
                Line::Long
            } else {
                Line::Got(line)
            });
        }
        if pending.len() > REQUEST_LIMIT {
            return Ok(Line::Long);
        }
        let n = stream.read(&mut buf).await?;
        if n == 0 {
            return Ok(Line::Gone);
        }
        pending.extend_from_slice(&buf[..n]);
    }
}

async fn write_line(stream: &mut UnixStream, doc: &Value) -> std::io::Result<()> {
    write_limited(stream, doc, RESPONSE_LIMIT).await
}

async fn write_limited(stream: &mut UnixStream, doc: &Value, limit: usize) -> std::io::Result<()> {
    let mut text = doc.to_string();
    if text.len() >= limit {
        text = Answer::new(
            exit::UNAVAILABLE,
            format!("the answer is longer than {limit} bytes"),
        )
        .json()
        .to_string();
    }
    text.push('\n');
    stream.write_all(text.as_bytes()).await
}

/// Close after an answer. The bytes that the client still sends are read
/// first: a close with unread bytes resets the connection, and the client
/// would lose the answer.
async fn finish(mut stream: UnixStream) {
    let _ = stream.shutdown().await;
    let mut sink = [0u8; 1024];
    let drain = async {
        let mut total = 0usize;
        while let Ok(n) = stream.read(&mut sink).await {
            total += n;
            if n == 0 || total > 4 * RESPONSE_LIMIT {
                break;
            }
        }
    };
    let _ = tokio::time::timeout(Duration::from_secs(1), drain).await;
}

async fn refuse(mut stream: UnixStream) {
    let mut pending = Vec::new();
    let _ =
        tokio::time::timeout(Duration::from_secs(2), read_line(&mut stream, &mut pending)).await;
    let answer = Answer::new(
        exit::UNAVAILABLE,
        format!("{MAX_CLIENTS} clients are connected: try again"),
    );
    let _ = write_line(&mut stream, &answer.json()).await;
    finish(stream).await;
}

async fn session(
    mut stream: UnixStream,
    hub: Arc<Hub>,
    token: Arc<str>,
    permitted: bool,
    permit: tokio::sync::OwnedSemaphorePermit,
) {
    let mut pending = Vec::new();
    loop {
        let line = match tokio::time::timeout(LIMIT, read_line(&mut stream, &mut pending)).await {
            Ok(Ok(Line::Got(line))) => line,
            Ok(Ok(Line::Long)) => {
                let answer = Answer::new(
                    exit::USAGE,
                    format!("a request is {REQUEST_LIMIT} bytes at most"),
                );
                let _ = write_line(&mut stream, &answer.json()).await;
                return finish(stream).await;
            }
            _ => return,
        };
        let refusal = |code: i32, text: &str| Answer::new(code, text).json();
        let request: Value = match serde_json::from_slice(&line) {
            Ok(v @ Value::Object(_)) => v,
            _ => {
                let _ = write_line(
                    &mut stream,
                    &refusal(exit::USAGE, "a request is one JSON object"),
                )
                .await;
                continue;
            }
        };
        if !permitted {
            let _ = write_line(
                &mut stream,
                &refusal(exit::NO_PERM, "this user may not use the control socket"),
            )
            .await;
            return finish(stream).await;
        }
        let given = request["token"].as_str().unwrap_or_default();
        if !same(given.as_bytes(), token.as_bytes()) {
            let _ = write_line(&mut stream, &refusal(exit::NO_PERM, "the token is wrong")).await;
            return finish(stream).await;
        }
        match hub.verb(&request).await {
            Ok(Reply::Json(doc)) => {
                if write_line(&mut stream, &doc).await.is_err() {
                    return;
                }
            }
            Ok(Reply::Big(doc)) => {
                if write_limited(&mut stream, &doc, DUMP_LIMIT).await.is_err() {
                    return;
                }
            }
            Ok(Reply::Pipe(mut far, doc, tracked)) => {
                // The connection is now the bytes of the stream: it no
                // longer counts as a client.
                drop(permit);
                if write_line(&mut stream, &doc).await.is_err() {
                    return;
                }
                if !pending.is_empty() && far.write_all(&pending).await.is_err() {
                    return;
                }
                tokio::select! {
                    _ = tokio::io::copy_bidirectional(&mut stream, &mut far) => {}
                    // `close-stream` names this stream.
                    _ = tracked.cancelled() => far.reset(ResetCode::CANCEL),
                }
                return;
            }
            Ok(Reply::Hold(doc, claim)) => {
                // The service lives while the client keeps the connection.
                drop(permit);
                if write_line(&mut stream, &doc).await.is_err() {
                    return;
                }
                accept::wait_gone(&mut stream).await;
                drop(claim);
                return;
            }
            Err(answer) => {
                let _ = write_line(&mut stream, &answer.json()).await;
                return finish(stream).await;
            }
        }
    }
}

mod client;
pub use client::{
    accept_peer, discover, dump_all, open_peer, probe_peer, read_token, Accepting, Client,
};
