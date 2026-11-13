//! The client side of the control socket (T-267, T-266): one call, the
//! verb `open`, the token file, and the search for a running instance.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

use super::{peer_list, token_path, CONTROL_VARIABLE, DUMP_LIMIT, RESPONSE_LIMIT};
use crate::exit::{self, Fault};

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
        // The dump may be longer than the answer of any other verb.
        let limit = if verb == "dump" || verb == "mesh" {
            DUMP_LIMIT
        } else {
            RESPONSE_LIMIT
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
            if got.len() > limit {
                return Err(exit::fault(
                    Fault::Unavailable,
                    anyhow!("the answer is longer than {limit} bytes"),
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

/// A service that a running instance serves for the streams that the peer
/// opens (the verb `accept`). The drop ends it.
pub struct Accepting {
    stream: UnixStream,
}

impl Accepting {
    /// Wait until the instance ends the connection (it exited, or its
    /// link ended and it closed the registration).
    pub async fn lost(&mut self) {
        let mut sink = [0u8; 64];
        while matches!(self.stream.read(&mut sink).await, Ok(n) if n > 0) {}
    }
}

/// Ask a running instance that holds a link to `fingerprint` to send the
/// streams of the service `service` that the peer opens to the unix
/// socket `socket` (an absolute path).
pub async fn accept_peer(
    fingerprint: &str,
    service: &str,
    socket: &Path,
) -> anyhow::Result<Accepting> {
    peer_list(fingerprint)?;
    let target = format!("unix:{}", socket.display());
    first(fingerprint, |control, token| {
        let target = target.clone();
        async move {
            let mut client = Client::connect(&control).await?;
            let args = json!({"peer": fingerprint, "service": service, "target": target});
            client.call(&token, "accept", args).await?;
            Ok(Accepting {
                stream: client.stream,
            })
        }
    })
    .await
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
                // No token file: no instance runs there any more.
                let e = if exit::code_of(&e) == exit::NO_INPUT {
                    exit::fault(
                        Fault::Unavailable,
                        anyhow!("no link instance runs at {}: {e:#}", socket.display()),
                    )
                } else {
                    e
                };
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

/// The time that `dump_all` waits for one socket.
const DUMP_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// The dump of each control socket that [`discover`] finds, for
/// `bug-report --include-link` (T-309). A socket that does not answer is
/// `{"control":PATH,"error":TEXT}`: a report is useful with one instance
/// down.
pub async fn dump_all() -> Vec<Value> {
    let mut all = Vec::new();
    for socket in discover() {
        let ask = async {
            let token = read_token(&token_path(&socket))?;
            Client::connect(&socket)
                .await?
                .call(&token, "dump", json!({}))
                .await
        };
        let doc = match tokio::time::timeout(DUMP_WAIT, ask).await {
            Ok(Ok(doc)) => doc,
            Ok(Err(e)) => json!({"error": format!("{e:#}")}),
            Err(_) => json!({"error": format!("no answer in {} s", DUMP_WAIT.as_secs())}),
        };
        let mut doc = doc;
        doc["control"] = socket.display().to_string().into();
        all.push(doc);
    }
    all
}
