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
//!
//! The verbs that change a link (T-307) and the state dump (T-309) plug in
//! at [`Hub::verb`].

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
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;

use crate::exit::{self, Fault};
use crate::identity::AllowList;
use crate::link::{Link, Stream};

/// The most bytes of a request line.
pub const REQUEST_LIMIT: usize = 4096;
/// The most bytes of a response line.
pub const RESPONSE_LIMIT: usize = 64 * 1024;
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
}

impl Held {
    fn row(&self) -> Value {
        json!({
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
            "since": self.since,
        })
    }
}

/// The links of one instance.
pub struct Hub {
    fingerprint: String,
    held: Mutex<Vec<Arc<Held>>>,
    next: AtomicU64,
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
}

impl Drop for Slot {
    fn drop(&mut self) {
        let mut held = self.hub.held.lock().unwrap_or_else(|e| e.into_inner());
        held.retain(|h| h.id != self.held.id);
    }
}

impl Hub {
    /// A hub for the install with this fingerprint.
    pub fn new(fingerprint: &str) -> Arc<Hub> {
        Arc::new(Hub {
            fingerprint: fingerprint.to_string(),
            held: Mutex::new(Vec::new()),
            next: AtomicU64::new(1),
        })
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
        up.into_iter()
            .find(|h| peer.admits(&h.peer))
            .ok_or_else(|| {
                Answer::new(
                    exit::NO_PERM,
                    "no link of this instance goes to a peer with this fingerprint",
                )
            })
    }

    async fn open(&self, peer: &AllowList, service: &str) -> Result<(Stream, String), Answer> {
        crate::link_service::check_name(service).map_err(|e| Answer::of(&e))?;
        let held = self.find(peer)?;
        let mut stream = held
            .link
            .open()
            .map_err(|e| Answer::of(&crate::link::fault(e)))?;
        held.opened.fetch_add(1, Ordering::Relaxed);
        match tokio::time::timeout(LIMIT, crate::link_service::request(&mut stream, service)).await
        {
            Ok(Ok(())) => Ok((stream, held.peer.display_fingerprint())),
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
    async fn verb(&self, request: &Value) -> Result<Reply, Answer> {
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
                let (stream, fp) = self.open(&peer, service).await?;
                Ok(Reply::Pipe(
                    stream,
                    json!({"ok": true, "verb": "open", "peer": fp}),
                ))
            }
            other => Err(Answer::new(
                exit::USAGE,
                format!("{other:?} is no verb of the control socket: use status, peer or open"),
            )),
        }
    }
}

/// The answer to a request: a document, or a document and then the bytes
/// of a stream.
enum Reply {
    Json(Value),
    Pipe(Stream, Value),
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
    let mut text = doc.to_string();
    if text.len() >= RESPONSE_LIMIT {
        text = Answer::new(
            exit::UNAVAILABLE,
            format!("the answer is longer than {RESPONSE_LIMIT} bytes"),
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
            Ok(Reply::Pipe(mut far, doc)) => {
                // The connection is now the bytes of the stream: it no
                // longer counts as a client.
                drop(permit);
                if write_line(&mut stream, &doc).await.is_err() {
                    return;
                }
                if !pending.is_empty() && far.write_all(&pending).await.is_err() {
                    return;
                }
                let _ = tokio::io::copy_bidirectional(&mut stream, &mut far).await;
                return;
            }
            Err(answer) => {
                let _ = write_line(&mut stream, &answer.json()).await;
                return finish(stream).await;
            }
        }
    }
}

/// The exit class of an error answer.
fn fault_of(code: i64) -> Fault {
    match code {
        64 => Fault::Usage,
        66 => Fault::NoInput,
        71 => Fault::OsError,
        77 => Fault::NoPermission,
        124 => Fault::Timeout,
        78 => Fault::Config,
        _ => Fault::Unavailable,
    }
}

/// A client of a control socket.
pub struct Client {
    stream: UnixStream,
    path: PathBuf,
}

impl Client {
    /// Connect to the socket at `path`. A socket that nobody serves exits
    /// 69.
    pub async fn connect(path: &Path) -> anyhow::Result<Client> {
        let stream = UnixStream::connect(path)
            .await
            .with_context(|| format!("the control socket {}", path.display()))
            .map_err(|e| exit::fault(Fault::Unavailable, e))?;
        Ok(Client {
            stream,
            path: path.to_path_buf(),
        })
    }

    async fn exchange(&mut self, token: &str, verb: &str, args: Value) -> anyhow::Result<Value> {
        let mut request = match args {
            Value::Object(map) => map,
            _ => serde_json::Map::new(),
        };
        request.insert("token".into(), token.into());
        request.insert("verb".into(), verb.into());
        let mut line = Value::Object(request).to_string();
        line.push('\n');
        let gone = |e: std::io::Error| {
            exit::fault(
                Fault::Unavailable,
                anyhow!("the control socket {}: {e}", self.path.display()),
            )
        };
        self.stream.write_all(line.as_bytes()).await.map_err(gone)?;
        let mut got = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            let n = self.stream.read(&mut byte).await.map_err(gone)?;
            if n == 0 {
                return Err(exit::fault(
                    Fault::Unavailable,
                    anyhow!(
                        "the control socket {} closed with no answer",
                        self.path.display()
                    ),
                ));
            }
            if byte[0] == b'\n' {
                break;
            }
            got.push(byte[0]);
            if got.len() > RESPONSE_LIMIT {
                return Err(exit::fault(
                    Fault::Unavailable,
                    anyhow!("the answer is longer than {RESPONSE_LIMIT} bytes"),
                ));
            }
        }
        let doc: Value = serde_json::from_slice(&got)
            .context("the control socket sent no JSON")
            .map_err(|e| exit::fault(Fault::Unavailable, e))?;
        if doc["ok"] == true {
            return Ok(doc);
        }
        let text = doc["error"].as_str().unwrap_or("the request failed");
        Err(exit::fault(
            fault_of(doc["code"].as_i64().unwrap_or(0)),
            anyhow!("{text}"),
        ))
    }

    /// Send one request and read its answer. An error answer exits with
    /// its code (a bad token: 77).
    pub async fn call(&mut self, token: &str, verb: &str, args: Value) -> anyhow::Result<Value> {
        self.exchange(token, verb, args).await
    }

    /// The verb `open`: after the answer the connection carries the bytes
    /// of one stream to the service of the peer.
    pub async fn open(
        mut self,
        token: &str,
        peer: &str,
        service: &str,
    ) -> anyhow::Result<UnixStream> {
        self.exchange(token, "open", json!({"peer": peer, "service": service}))
            .await?;
        Ok(self.stream)
    }
}

/// The token in `file`. A missing file exits 66, and a file that others
/// can read exits 71.
pub fn read_token(file: &Path) -> anyhow::Result<String> {
    use std::os::unix::fs::PermissionsExt as _;
    let meta = std::fs::metadata(file).map_err(|e| {
        let fault = if e.kind() == std::io::ErrorKind::NotFound {
            Fault::NoInput
        } else {
            Fault::OsError
        };
        exit::fault(fault, anyhow!("the token file {}: {e}", file.display()))
    })?;
    if meta.permissions().mode() & 0o077 != 0 {
        return Err(exit::fault(
            Fault::OsError,
            anyhow!(
                "the token file {} can be used by others (mode 0600)",
                file.display()
            ),
        ));
    }
    let text = std::fs::read_to_string(file)
        .map_err(|e| exit::fault(Fault::OsError, anyhow!("{}: {e}", file.display())))?;
    Ok(text.trim().to_string())
}

/// The control sockets of the instances that run: the one socket of
/// [`CONTROL_VARIABLE`], else the `control` of the ready document of each
/// link instance in the background record.
pub fn discover() -> Vec<PathBuf> {
    if let Some(path) = std::env::var_os(CONTROL_VARIABLE).filter(|v| !v.is_empty()) {
        return vec![PathBuf::from(path)];
    }
    let mut found = Vec::new();
    for entry in crate::instances::list(&crate::instances::lookup_dir()) {
        if !entry.record.command.starts_with("link ") {
            continue;
        }
        let control = entry
            .record
            .ready
            .as_ref()
            .and_then(|d| d["control"].as_str());
        if let Some(path) = control.map(PathBuf::from).filter(|p| !found.contains(p)) {
            found.push(path);
        }
    }
    found
}

/// Ask each instance in turn, until one answers `ok`. An instance that
/// holds links to other peers gives 77; none that runs, or none with a
/// link, gives 69. The best error is the one of 77.
async fn first<T, F, Fut>(what: &str, mut ask: F) -> anyhow::Result<T>
where
    F: FnMut(PathBuf, String) -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<T>>,
{
    let mut worst: Option<anyhow::Error> = None;
    for socket in discover() {
        let token = match read_token(&token_path(&socket)) {
            Ok(token) => token,
            Err(e) => {
                worst.get_or_insert(e);
                continue;
            }
        };
        match ask(socket, token).await {
            Ok(done) => return Ok(done),
            Err(e) => {
                let better = exit::code_of(&e) == exit::NO_PERM
                    && worst
                        .as_ref()
                        .is_none_or(|w| exit::code_of(w) != exit::NO_PERM);
                if worst.is_none() || better {
                    worst = Some(e);
                }
            }
        }
    }
    Err(worst.unwrap_or_else(|| {
        exit::fault(
            Fault::Unavailable,
            anyhow!(
                "no link instance runs, so there is no link to {what} (podnet link connect, or set {CONTROL_VARIABLE})"
            ),
        )
    }))
}

/// Check that a running instance holds a link to the peer `fingerprint`.
/// A link to other peers only exits 77.
pub async fn probe_peer(fingerprint: &str) -> anyhow::Result<()> {
    peer_list(fingerprint)?;
    first(fingerprint, |socket, token| async move {
        Client::connect(&socket)
            .await?
            .call(&token, "peer", json!({"peer": fingerprint}))
            .await
            .map(|_| ())
    })
    .await
}

/// Open one stream to the service `service` of the peer `fingerprint`,
/// through a running instance. The bytes of the stream are the bytes of
/// the returned connection.
pub async fn open_peer(fingerprint: &str, service: &str) -> anyhow::Result<UnixStream> {
    peer_list(fingerprint)?;
    first(fingerprint, |socket, token| async move {
        Client::connect(&socket)
            .await?
            .open(&token, fingerprint, service)
            .await
    })
    .await
}
