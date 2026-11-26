//! The socket of the inspector: `NAME.inspect.sock` in the run directory,
//! mode 0600 (`bind_private`), and the check of the user of each peer
//! (`SO_PEERCRED`): the user of the tunnel, or root.
//!
//! The wire is one JSON object for each line. A request is
//! `{"verb":"list"}`, `{"verb":"show","id":N}`, `{"verb":"replay","id":N}`
//! or `{"verb":"clear"}` of 4 KiB at most. The answer is `{"ok":true,...}` or
//! `{"ok":false,"code":EXIT,"error":TEXT}`, and then the server closes the
//! connection. There is no TCP listener.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context as _, Result};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;

use super::Inspector;
use crate::exit::{self, Fault};

/// The end of the name of the socket of an instance.
pub const SUFFIX: &str = ".inspect.sock";

/// The largest request line.
const REQUEST_LIMIT: usize = 4096;
/// The clients that are served at one time.
const MAX_CLIENTS: usize = 8;
/// The time that a client has to send its request.
const READ_TIME: Duration = Duration::from_secs(10);

/// The socket path of the inspector of this process: `NAME.inspect.sock`
/// beside the record of the instance (`--detach`, `--name`), else
/// `tunnel-HEX.inspect.sock` in the run directory, with 8 random hex
/// digits (T-545: a pid is no unique name).
pub fn socket_path() -> Result<PathBuf> {
    if let Some((dir, name)) = crate::detach::instance() {
        return Ok(dir.join(format!("{name}{SUFFIX}")));
    }
    let dir = crate::instances::run_dir()?;
    let nonce = crate::state::temp::nonce();
    Ok(dir.join(format!("tunnel-{}{SUFFIX}", &nonce[..8])))
}

/// The socket of the inspector of the instance `name`, or, with no name,
/// of the one tunnel with `--inspect` that runs. None that runs gives exit
/// 69; more than one gives exit 64 with their names.
pub fn find_socket(name: Option<&str>) -> Result<PathBuf> {
    let dir = crate::instances::lookup_dir();
    if let Some(name) = name {
        crate::instances::check_name(name).map_err(|e| exit::fault(Fault::Usage, e))?;
        let path = dir.join(format!("{name}{SUFFIX}"));
        if !path.exists() {
            return Err(exit::fault(
                Fault::Unavailable,
                anyhow!(
                    "no tunnel with --inspect runs under the name {name} (no {})",
                    path.display()
                ),
            ));
        }
        return Ok(path);
    }
    let mut found: Vec<(String, PathBuf)> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let file = entry.file_name().to_string_lossy().into_owned();
            let name = file.strip_suffix(SUFFIX)?.to_string();
            Some((name, entry.path()))
        })
        .filter(|(_, path)| crate::net::socket_answers(path))
        .collect();
    found.sort();
    match found.len() {
        0 => Err(exit::fault(
            Fault::Unavailable,
            anyhow!(
                "no tunnel with --inspect runs (no *{SUFFIX} in {} answers)",
                dir.display()
            ),
        )),
        1 => Ok(found.remove(0).1),
        _ => Err(exit::fault(
            Fault::Usage,
            anyhow!(
                "more than one tunnel with --inspect runs; give --name: {}",
                found
                    .iter()
                    .map(|(n, _)| n.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )),
    }
}

/// The served socket. Its drop stops the server and removes the socket.
pub struct Served {
    path: PathBuf,
    task: JoinHandle<()>,
}

impl Served {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Served {
    fn drop(&mut self) {
        self.task.abort();
        let _ = std::fs::remove_file(&self.path);
        let mut served = SERVED.lock().unwrap_or_else(|e| e.into_inner());
        if served.as_deref() == Some(self.path.as_path()) {
            *served = None;
        }
    }
}

/// The socket that this process serves, for the ready document.
static SERVED: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

/// The socket of the inspector that this process serves.
pub fn served_path() -> Option<PathBuf> {
    SERVED.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Serve `inspector` on a new socket at `path` (mode 0600). A socket that
/// another process serves is refused (exit 69); a stale socket file is
/// replaced. Needs a Tokio runtime.
pub fn serve(inspector: Arc<Inspector>, path: &Path) -> Result<Served> {
    crate::net::claim_socket_path(path, false)?;
    let listener = podnet_netns::listen::bind_private(path)
        .with_context(|| format!("the socket of the inspector unix:{}", path.display()))
        .map_err(|e| exit::fault(Fault::OsError, e))?;
    let task = tokio::spawn(accept(listener, inspector));
    *SERVED.lock().unwrap_or_else(|e| e.into_inner()) = Some(path.to_path_buf());
    Ok(Served {
        path: path.to_path_buf(),
        task,
    })
}

/// True when a peer of the socket may use it: the user of the tunnel, or
/// root. The mode 0600 of the socket is the first gate; this is the
/// second.
pub fn peer_allowed(peer: u32, own: u32) -> bool {
    peer == own || peer == 0
}

fn own_uid() -> u32 {
    // SAFETY: geteuid() has no precondition.
    unsafe { libc::geteuid() }
}

async fn accept(listener: UnixListener, inspector: Arc<Inspector>) {
    let slots = Arc::new(Semaphore::new(MAX_CLIENTS));
    let own = own_uid();
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            tokio::time::sleep(Duration::from_millis(50)).await;
            continue;
        };
        let permitted = stream
            .peer_cred()
            .map(|c| peer_allowed(c.uid(), own))
            .unwrap_or(false);
        let Ok(permit) = slots.clone().try_acquire_owned() else {
            tokio::spawn(answer(
                stream,
                refusal(69, "the inspector is busy; try again"),
            ));
            continue;
        };
        let inspector = inspector.clone();
        tokio::spawn(async move {
            if permitted {
                session(stream, &inspector).await;
            } else {
                let text = "the inspector serves only the user of the tunnel";
                answer(stream, refusal(77, text)).await;
            }
            drop(permit);
        });
    }
}

/// Read one request of `stream`, and write its answer.
async fn session(mut stream: UnixStream, inspector: &Arc<Inspector>) {
    let doc = match tokio::time::timeout(READ_TIME, read_line(&mut stream)).await {
        Err(_) => refusal(124, "no request in 10 s"),
        Ok(Err(e)) => refusal(64, &e.to_string()),
        Ok(Ok(line)) => match serde_json::from_slice::<Value>(&line) {
            Ok(request) => handle(inspector, &request).await,
            Err(e) => refusal(64, &format!("the request is not JSON: {e}")),
        },
    };
    answer(stream, doc).await;
}

async fn answer(mut stream: UnixStream, doc: Value) {
    let mut line = doc.to_string();
    line.push('\n');
    let _ = stream.write_all(line.as_bytes()).await;
    let _ = stream.shutdown().await;
}

fn refusal(code: i32, text: &str) -> Value {
    json!({"ok": false, "code": code, "error": text})
}

/// One line of at most [`REQUEST_LIMIT`] bytes.
async fn read_line(stream: &mut UnixStream) -> std::io::Result<Vec<u8>> {
    let mut got = Vec::new();
    let mut buf = [0u8; 512];
    loop {
        if let Some(at) = got.iter().position(|b| *b == b'\n') {
            got.truncate(at);
            return Ok(got);
        }
        if got.len() > REQUEST_LIMIT {
            return Err(std::io::Error::other("the request is longer than 4 KiB"));
        }
        let n = stream.read(&mut buf).await?;
        if n == 0 {
            return Ok(got);
        }
        got.extend_from_slice(&buf[..n]);
    }
}

/// The answer to one request.
pub(super) async fn handle(inspector: &Arc<Inspector>, request: &Value) -> Value {
    let id = request["id"].as_u64();
    let missing = |id: u64| {
        refusal(
            66,
            &format!(
                "the ring has no exchange {id} (it holds the last {} only)",
                inspector.options().keep
            ),
        )
    };
    match (request["verb"].as_str(), id) {
        (Some("list"), _) => {
            let mut doc = inspector.list_doc();
            doc["ok"] = true.into();
            doc
        }
        (Some("clear"), _) => json!({"ok": true, "cleared": inspector.clear()}),
        (Some("show"), Some(id)) => match inspector.get(id) {
            Some(exchange) => json!({"ok": true, "exchange": exchange.detail()}),
            None => missing(id),
        },
        (Some("replay"), Some(id)) => match inspector.replay(id).await {
            Ok(mut doc) => {
                doc["ok"] = true.into();
                doc
            }
            Err(err) => refusal(exit::code_of(&err), &exit::text(&err)),
        },
        (Some("show" | "replay"), None) => refusal(64, "the request needs an id"),
        (verb, _) => refusal(
            64,
            &format!("unknown verb {verb:?}; give list, show, replay or clear"),
        ),
    }
}
