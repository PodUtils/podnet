//! The control socket of the router (T-097, T-180).
//!
//! One unix socket (mode 0600, the peer must be the user of the router or
//! root) answers a small HTTP/1.0 API with JSON:
//!
//! ```text
//! GET   /route/status                        the status document
//! GET   /route/metrics                       the rows of the metrics (text)
//! POST  /route/select      {"group":G,"member":M|null}   pin a member
//! POST  /route/rotate      {"member":M}      a new identity, make before break
//! POST  /route/reconnect   {"member":M}      a new link, same identity
//! PATCH /route/interval    {"seconds":N}     the seconds between two probes
//! ```
//!
//! Each write answers with the status document of the new state. An error
//! answers `{"error":TEXT}` with 400, 403, 404, 405, 429 or 502.
//! A `member` is a network of the file.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::Value;
use tokio::time::Instant;

use super::Router;
use crate::network::{Ctx, Failure, Link, Network, Swap};

/// The time limit of one exchange on the control socket.
const CONTROL_LIMIT: Duration = Duration::from_secs(5);
/// The most bytes of a request body.
const BODY_LIMIT: usize = 4096;
/// The least time between two rotations of one network.
pub const ROTATE_GAP: Duration = Duration::from_secs(60);
/// The time that the old link of a network lives after a swap, so that the
/// flows on it can end.
pub const DRAIN: Duration = Duration::from_secs(30);

/// The network of a spec. The default is the registry.
pub type Connector = Arc<dyn Fn(&str) -> Result<Arc<dyn Network>, Failure> + Send + Sync>;

/// The state that the write side of the control socket needs.
pub struct ControlState {
    swaps: HashMap<String, Arc<Swap>>,
    specs: BTreeMap<String, String>,
    state_dir: Option<PathBuf>,
    links: tokio::sync::Mutex<HashMap<String, Link>>,
    busy: tokio::sync::Mutex<()>,
    last_rotate: Mutex<HashMap<String, Instant>>,
    rotate_gap: Mutex<Duration>,
    drain: Mutex<Duration>,
    connector: Mutex<Connector>,
    interval: AtomicU64,
    wake: tokio::sync::Notify,
}

impl ControlState {
    pub fn new(
        swaps: HashMap<String, Arc<Swap>>,
        specs: &BTreeMap<String, String>,
        state_dir: Option<PathBuf>,
        interval: Option<Duration>,
    ) -> ControlState {
        ControlState {
            swaps,
            specs: specs.clone(),
            state_dir,
            links: Default::default(),
            busy: Default::default(),
            last_rotate: Default::default(),
            rotate_gap: Mutex::new(ROTATE_GAP),
            drain: Mutex::new(DRAIN),
            connector: Mutex::new(Arc::new(|spec| {
                let network = crate::networks::parse(spec)?;
                Ok(Arc::new(network) as Arc<dyn Network>)
            })),
            interval: AtomicU64::new(interval.map_or(0, |d| d.as_secs())),
            wake: Default::default(),
        }
    }

    /// Keep the links that `Router::up` brought up.
    pub async fn keep(&self, links: HashMap<String, Link>) {
        *self.links.lock().await = links;
    }

    /// The seconds between two probes now; `None` with no health section.
    pub fn interval(&self) -> Option<Duration> {
        match self.interval.load(Ordering::Relaxed) {
            0 => None,
            n => Some(Duration::from_secs(n)),
        }
    }

    /// Wait for the end of an interval, or for a change of it.
    pub async fn pause(&self) {
        let wake = self.wake.notified();
        let Some(interval) = self.interval() else {
            return wake.await;
        };
        tokio::select! {
            _ = tokio::time::sleep(interval) => {}
            _ = wake => {}
        }
    }

    /// Replace the function that makes a network of a spec (the tests).
    pub fn set_connector(&self, connector: Connector) {
        *self.connector.lock().unwrap_or_else(|e| e.into_inner()) = connector;
    }

    /// Set the least time between two rotations of one network.
    pub fn set_rotate_gap(&self, gap: Duration) {
        *self.rotate_gap.lock().unwrap_or_else(|e| e.into_inner()) = gap;
    }

    /// Set the time that an old link lives after a swap.
    pub fn set_drain(&self, drain: Duration) {
        *self.drain.lock().unwrap_or_else(|e| e.into_inner()) = drain;
    }

    /// Make a new link of the network `name` and give it to the new flows
    /// (make before break). The old link ends after the drain time. With
    /// `rotate`, the network makes a new identity; it keeps its state
    /// directory either way. Rotations of one network keep a gap, and each
    /// attempt is logged.
    async fn renew(&self, name: &str, rotate: bool) -> Result<(), Answer> {
        let swap = self
            .swaps
            .get(name)
            .ok_or_else(|| Answer::new(404, format!("{name:?} is not a network of the file")))?;
        let _one_at_a_time = self.busy.lock().await;
        if rotate {
            let gap = *self.rotate_gap.lock().unwrap_or_else(|e| e.into_inner());
            let mut last = self.last_rotate.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(wait) = last
                .get(name)
                .and_then(|at| gap.checked_sub(at.elapsed()))
                .filter(|w| !w.is_zero())
            {
                return Err(Answer::new(
                    429,
                    format!(
                        "{name}: rotated lately; try again in {} s",
                        wait.as_secs() + 1
                    ),
                ));
            }
            last.insert(name.to_string(), Instant::now());
        }
        let ctx = Ctx {
            below: None,
            state_dir: self.state_dir.as_ref().map(|d| d.join(name)),
        };
        if let Some(dir) = &ctx.state_dir {
            std::fs::create_dir_all(dir)
                .map_err(|e| Answer::new(502, format!("{name}: {}: {e}", dir.display())))?;
        }
        let connector = self
            .connector
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let spec = &self.specs[name];
        let network =
            connector(spec).map_err(|e| Answer::new(502, format!("{name}: {}", e.text())))?;
        let up = if rotate {
            network.rotate(&ctx).await
        } else {
            network.up(&ctx).await
        };
        let mut link = up.map_err(|e| Answer::new(502, format!("{name}: {e:#}")))?;
        let Some(way) = link.way.take() else {
            link.close().await;
            return Err(Answer::new(
                502,
                format!("{name}: the network gives no way out"),
            ));
        };
        swap.replace(way.dial);
        let old = self.links.lock().await.insert(name.to_string(), link);
        let what = if rotate { "rotate" } else { "reconnect" };
        eprintln!("podnet: route  {what} {name}");
        if let Some(old) = old {
            let drain = *self.drain.lock().unwrap_or_else(|e| e.into_inner());
            tokio::spawn(async move {
                tokio::time::sleep(drain).await;
                old.close().await;
            });
        }
        Ok(())
    }

    /// Set the seconds between two probes.
    fn set_interval(&self, seconds: u64) -> Result<(), Answer> {
        if self.interval().is_none() {
            return Err(Answer::new(409, "the file has no health section".into()));
        }
        if seconds == 0 || seconds > 86_400 {
            return Err(Answer::new(400, "seconds: give 1 to 86400".into()));
        }
        self.interval.store(seconds, Ordering::Relaxed);
        self.wake.notify_waiters();
        Ok(())
    }
}

/// The body of a good answer.
enum Reply {
    Json(Value),
    Text(String),
}

/// An error answer: the HTTP code and the text.
#[derive(Debug)]
pub struct Answer {
    pub code: u16,
    pub text: String,
}

impl Answer {
    fn new(code: u16, text: String) -> Answer {
        Answer { code, text }
    }
}

impl std::fmt::Display for Answer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "the control socket answered {}: {}",
            self.code, self.text
        )
    }
}

impl std::error::Error for Answer {}

fn reason(code: u16) -> &'static str {
    match code {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        429 => "Too Many Requests",
        _ => "Bad Gateway",
    }
}

/// True when a peer of the socket may use it: the user of the router, or
/// root. The mode of the socket (0600) is the first gate; this is the second.
pub fn peer_allowed(peer: u32, own: u32) -> bool {
    peer == own || peer == 0
}

fn string_of(body: &Value, key: &str) -> Result<String, Answer> {
    body[key]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| Answer::new(400, format!("give {key} (a string)")))
}

impl Router {
    /// Answer on the control socket until the listener fails.
    pub async fn serve_control(self: Arc<Self>, listener: tokio::net::UnixListener) -> Result<()> {
        // SAFETY: geteuid has no precondition.
        let own = unsafe { libc::geteuid() };
        self.serve_control_gated(listener, Arc::new(move |peer| peer_allowed(peer, own)))
            .await
    }

    /// [`Router::serve_control`] with the test `allow` for the user id of a peer.
    pub async fn serve_control_gated(
        self: Arc<Self>,
        listener: tokio::net::UnixListener,
        allow: Arc<dyn Fn(u32) -> bool + Send + Sync>,
    ) -> Result<()> {
        loop {
            let (mut stream, _) = listener.accept().await.context("the control socket")?;
            let router = self.clone();
            let allow = allow.clone();
            tokio::spawn(async move {
                let permitted = stream.peer_cred().map(|c| allow(c.uid())).unwrap_or(false);
                let answer = router.answer(&mut stream, permitted);
                let _ = tokio::time::timeout(CONTROL_LIMIT, answer).await;
            });
        }
    }

    async fn answer(
        &self,
        stream: &mut tokio::net::UnixStream,
        permitted: bool,
    ) -> std::io::Result<()> {
        use tokio::io::AsyncWriteExt as _;
        // The request is read in each case: a close with unread bytes resets
        // the connection and the peer loses the answer.
        let result = match read_request(stream).await {
            Ok(_) if !permitted => Err(Answer::new(
                403,
                "this user may not use the control socket".into(),
            )),
            Ok((method, route, _)) if route == "/route/metrics" => {
                if method == "GET" {
                    Ok(Reply::Text(self.metrics()))
                } else {
                    Err(Answer::new(405, "/route/metrics: use GET".into()))
                }
            }
            Ok((method, route, body)) => self.handle(&method, &route, &body).await.map(Reply::Json),
            Err(e) => Err(e),
        };
        let (code, kind, body) = match result {
            Ok(Reply::Json(body)) => (200, "application/json", body.to_string()),
            Ok(Reply::Text(text)) => (200, "text/plain; version=0.0.4", text),
            Err(a) => (
                a.code,
                "application/json",
                serde_json::json!({"error": a.text}).to_string(),
            ),
        };
        let reply = format!(
            "HTTP/1.0 {code} {}\r\ncontent-type: {kind}\r\ncontent-length: {}\r\n\r\n{body}",
            reason(code),
            body.len()
        );
        stream.write_all(reply.as_bytes()).await?;
        stream.shutdown().await
    }

    /// One request: the new status document, or the error answer.
    pub async fn handle(&self, method: &str, route: &str, body: &[u8]) -> Result<Value, Answer> {
        let allowed = match route {
            "/route/status" => "GET",
            "/route/select" | "/route/rotate" | "/route/reconnect" => "POST",
            "/route/interval" => "PATCH",
            _ => return Err(Answer::new(
                404,
                "the control socket answers /route/status, metrics, select, rotate, reconnect and interval"
                    .into(),
            )),
        };
        if method != allowed {
            return Err(Answer::new(405, format!("{route}: use {allowed}")));
        }
        if route == "/route/status" {
            return Ok(self.status());
        }
        let body: Value = serde_json::from_slice(body)
            .map_err(|e| Answer::new(400, format!("the body is not JSON: {e}")))?;
        match route {
            "/route/select" => {
                let group = string_of(&body, "group")?;
                let member = match &body["member"] {
                    Value::Null => None,
                    Value::String(m) if m == "auto" => None,
                    Value::String(m) => Some(m.as_str()),
                    _ => return Err(Answer::new(400, "member: a name, \"auto\" or null".into())),
                };
                let (_, g) = self
                    .groups
                    .iter()
                    .find(|(n, _)| *n == group)
                    .ok_or_else(|| {
                        Answer::new(404, format!("{group:?} is not a group of the file"))
                    })?;
                g.pin(member)
                    .map_err(|e| Answer::new(404, e.text().to_string()))?;
                eprintln!("podnet: route  select {group} {}", member.unwrap_or("auto"));
            }
            "/route/rotate" => {
                self.control
                    .renew(&string_of(&body, "member")?, true)
                    .await?
            }
            "/route/reconnect" => {
                self.control
                    .renew(&string_of(&body, "member")?, false)
                    .await?
            }
            _ => {
                let seconds = body["seconds"]
                    .as_u64()
                    .ok_or_else(|| Answer::new(400, "give seconds (a whole number)".into()))?;
                self.control.set_interval(seconds)?;
            }
        }
        Ok(self.status())
    }
}

/// Read one request: the method, the route and the body.
async fn read_request(
    stream: &mut tokio::net::UnixStream,
) -> Result<(String, String, Vec<u8>), Answer> {
    use tokio::io::AsyncReadExt as _;
    let bad = |text: &str| Answer::new(400, text.to_string());
    let mut data = Vec::new();
    let mut buf = [0u8; 512];
    let end = loop {
        if let Some(at) = data.windows(4).position(|w| w == b"\r\n\r\n") {
            break at + 4;
        }
        if data.len() >= 8192 {
            return Err(bad("the head is too long"));
        }
        let n = stream
            .read(&mut buf)
            .await
            .map_err(|e| bad(&e.to_string()))?;
        if n == 0 {
            return Err(bad("the request ends in the head"));
        }
        data.extend_from_slice(&buf[..n]);
    };
    let head = String::from_utf8_lossy(&data[..end]).into_owned();
    let mut lines = head.lines();
    let mut first = lines.next().unwrap_or("").split_whitespace();
    let (Some(method), Some(route)) = (first.next(), first.next()) else {
        return Err(bad("give METHOD /route/NAME HTTP/1.0"));
    };
    let length = lines
        .filter_map(|l| l.split_once(':'))
        .find(|(k, _)| k.trim().eq_ignore_ascii_case("content-length"))
        .map(|(_, v)| v.trim().parse::<usize>())
        .transpose()
        .map_err(|_| bad("content-length is not a number"))?
        .unwrap_or(0);
    if length > BODY_LIMIT {
        return Err(bad("the body is too long"));
    }
    let mut body = data[end..].to_vec();
    while body.len() < length {
        let n = stream
            .read(&mut buf)
            .await
            .map_err(|e| bad(&e.to_string()))?;
        if n == 0 {
            return Err(bad("the request ends in the body"));
        }
        body.extend_from_slice(&buf[..n]);
    }
    body.truncate(length);
    Ok((method.to_string(), route.to_string(), body))
}

/// The status document of the router of the control socket `path`
/// (`podnet route --status`).
pub async fn read_status(path: &Path) -> Result<Value> {
    request(path, "GET", "/route/status", None).await
}

/// The metrics rows of the router of the control socket `path`
/// (`podnet route --status --metrics`).
pub async fn read_metrics(path: &Path) -> Result<String> {
    let (code, line, body) = exchange(path, "GET", "/route/metrics", None).await?;
    if code != Some(200) {
        return Err(refusal(code, &line, &body));
    }
    String::from_utf8(body).context("the metrics of the control socket")
}

/// One request to the control socket `path`; the answer is the JSON of a 200.
/// An error answer is an [`Answer`] in the error.
pub async fn request(
    path: &Path,
    method: &str,
    route: &str,
    body: Option<&Value>,
) -> Result<Value> {
    let (code, line, answer) = exchange(path, method, route, body).await?;
    if code != Some(200) {
        return Err(refusal(code, &line, &answer));
    }
    serde_json::from_slice(&answer).context("the answer of the control socket")
}

/// The error of an answer that is not a 200: an [`Answer`] with the text of
/// its JSON `error`, else the status line.
fn refusal(code: Option<u16>, line: &str, body: &[u8]) -> anyhow::Error {
    let value: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let text = value["error"].as_str().unwrap_or(line).to_string();
    match code {
        Some(code) => Answer::new(code, text).into(),
        None => anyhow::anyhow!("the control socket answered: {line}"),
    }
}

/// One exchange: the code, the status line and the body of the answer.
async fn exchange(
    path: &Path,
    method: &str,
    route: &str,
    body: Option<&Value>,
) -> Result<(Option<u16>, String, Vec<u8>)> {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let exchange = async {
        let mut stream = tokio::net::UnixStream::connect(path)
            .await
            .with_context(|| format!("the control socket unix:{}", path.display()))?;
        let body = body.map(|b| b.to_string()).unwrap_or_default();
        let head = format!(
            "{method} {route} HTTP/1.0\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n",
            body.len()
        );
        // A router that refuses the peer answers and closes before it reads.
        let _ = stream.write_all(head.as_bytes()).await;
        let _ = stream.write_all(body.as_bytes()).await;
        let mut answer = Vec::new();
        (&mut stream)
            .take(16 << 20)
            .read_to_end(&mut answer)
            .await?;
        Ok::<_, anyhow::Error>(answer)
    };
    let answer = tokio::time::timeout(CONTROL_LIMIT, exchange)
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "the control socket unix:{} gave no answer in 5 s",
                path.display()
            )
        })??;
    let split = answer
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .context("the control socket gave no HTTP answer")?;
    let status_line = String::from_utf8_lossy(&answer[..split]);
    let line = status_line.lines().next().unwrap_or("").to_string();
    let code = line
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse::<u16>().ok());
    Ok((code, line, answer[split + 4..].to_vec()))
}

#[cfg(test)]
mod tests;
