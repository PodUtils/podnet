//! Upload / download / proxy sessions — the feature set quickbridge brings,
//! reimplemented as a library service so a tunnel can expose it directly.
//!
//! - **Upload**: a multipart form saves files into a directory.
//! - **Download**: serves one file (or a clipboard snapshot) once.
//! - **Proxy**: forwards everything to an existing local HTTP port.
//!
//! A random token lives in the URL path; an optional 6-digit PIN gate adds a
//! second factor. `stop_after` tears the session down after the first
//! successful transfer.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Multipart, State};
use axum::http::{header, HeaderMap, StatusCode, Uri};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Form, Router};
use serde::Deserialize;
use tokio::sync::Notify;

use crate::gate::{Gate, UNLOCK_PATH};
use podnet_core::util::{ensure_dir, human_bytes, sanitize_filename};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShareMode {
    Upload,
    Download,
    Proxy,
}

impl ShareMode {
    pub fn parse(value: &str) -> Result<Self> {
        match value.to_ascii_lowercase().as_str() {
            "upload" => Ok(ShareMode::Upload),
            "download" => Ok(ShareMode::Download),
            "proxy" => Ok(ShareMode::Proxy),
            other => bail!("unknown share mode {other:?}"),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ShareMode::Upload => "upload",
            ShareMode::Download => "download",
            ShareMode::Proxy => "proxy",
        }
    }
}

/// The default most bytes of one upload request: the edge of Cloudflare
/// takes at most 100 MB in one request body on the free plan.
pub const MAX_REQUEST: u64 = 100_000_000;

#[derive(Clone, Debug)]
pub struct ShareConfig {
    pub mode: ShareMode,
    pub token: String,
    pub dest: PathBuf,
    pub file: Option<PathBuf>,
    pub target: Option<String>,
    /// Upload only: the most bytes of one file.
    pub max_bytes: u64,
    /// Upload only: the most files of the session.
    pub max_files: u32,
    /// Upload only: the most bytes of the session, over all files.
    pub max_total: u64,
    /// Upload only: the most bytes of one upload request (issue 92). A
    /// larger `Content-Length` gets 413 before podnet reads the body.
    pub max_request: u64,
    pub stop_after: bool,
    pub gate: Gate,
}

impl ShareConfig {
    pub fn upload(dest: impl Into<PathBuf>) -> Self {
        Self {
            mode: ShareMode::Upload,
            token: podnet_core::util::random_token(),
            dest: dest.into(),
            file: None,
            target: None,
            max_bytes: 512 * 1024 * 1024,
            max_files: 32,
            max_total: 4 * 1024 * 1024 * 1024,
            max_request: MAX_REQUEST,
            stop_after: false,
            gate: Gate::off(),
        }
    }

    pub fn download(file: impl Into<PathBuf>) -> Self {
        Self {
            mode: ShareMode::Download,
            token: podnet_core::util::random_token(),
            dest: PathBuf::new(),
            file: Some(file.into()),
            target: None,
            max_bytes: 0,
            max_files: 0,
            max_total: 0,
            max_request: 0,
            stop_after: false,
            gate: Gate::off(),
        }
    }

    pub fn proxy(target: impl Into<String>) -> Self {
        Self {
            mode: ShareMode::Proxy,
            token: podnet_core::util::random_token(),
            dest: PathBuf::new(),
            file: None,
            target: Some(target.into()),
            max_bytes: 0,
            max_files: 0,
            max_total: 0,
            max_request: 0,
            stop_after: false,
            gate: Gate::pin(),
        }
    }

    pub fn with_gate(mut self, gate: Gate) -> Self {
        self.gate = gate;
        self
    }

    pub fn stop_after(mut self, yes: bool) -> Self {
        self.stop_after = yes;
        self
    }

    pub fn describe(&self) -> String {
        match self.mode {
            ShareMode::Upload => format!("share:upload:{}", self.dest.display()),
            ShareMode::Download => format!(
                "share:download:{}",
                self.file
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default()
            ),
            ShareMode::Proxy => format!("share:proxy:{}", self.target.clone().unwrap_or_default()),
        }
    }

    /// Path appended to the public URL for the session.
    pub fn url_path(&self) -> String {
        match self.mode {
            ShareMode::Proxy => "/".to_string(),
            _ => format!("/s/{}/", self.token),
        }
    }
}

/// Cross-task session control: last activity + stop signal.
pub struct ShareControl {
    last: Mutex<Instant>,
    stop: Notify,
    stopped: std::sync::atomic::AtomicBool,
}

impl Default for ShareControl {
    fn default() -> Self {
        Self {
            last: Mutex::new(Instant::now()),
            stop: Notify::new(),
            stopped: std::sync::atomic::AtomicBool::new(false),
        }
    }
}

impl ShareControl {
    pub fn touch(&self) {
        if let Ok(mut guard) = self.last.lock() {
            *guard = Instant::now();
        }
    }

    pub fn idle_for(&self) -> Duration {
        self.last
            .lock()
            .map(|guard| guard.elapsed())
            .unwrap_or_default()
    }

    pub fn request_stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        self.stop.notify_waiters();
    }

    /// True after [`request_stop`](Self::request_stop).
    pub fn stop_requested(&self) -> bool {
        self.stopped.load(Ordering::SeqCst)
    }

    /// Ready after [`request_stop`](Self::request_stop), also when the stop
    /// came before this call.
    pub async fn wait_stop(&self) {
        loop {
            let notified = self.stop.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.stop_requested() {
                return;
            }
            notified.await;
        }
    }
}

struct UploadState {
    dest: PathBuf,
    token: String,
    max_bytes: u64,
    max_files: u32,
    max_total: u64,
    max_request: u64,
    used_files: AtomicU32,
    used_bytes: AtomicU64,
    stop_after: bool,
    gate: Gate,
    control: Arc<ShareControl>,
}

struct DownloadState {
    token: String,
    file: PathBuf,
    name: String,
    stop_after: bool,
    gate: Gate,
    control: Arc<ShareControl>,
}

/// A built share session.
pub struct ShareApp {
    config: ShareConfig,
    control: Arc<ShareControl>,
    router: Router,
}

impl ShareApp {
    pub fn new(config: ShareConfig) -> Result<Self> {
        if config.mode == ShareMode::Proxy {
            bail!("proxy sessions are forwarded by the tunnel, not served by ShareApp");
        }
        if config.mode == ShareMode::Upload {
            ensure_dir(&config.dest)?;
        }
        if let Some(file) = &config.file {
            match std::fs::metadata(file) {
                Ok(meta) if meta.is_file() => {}
                // Issue 249: exit 78, not 1.
                Ok(_) => {
                    return Err(podnet_core::exit::fault(
                        podnet_core::exit::Fault::Config,
                        anyhow::anyhow!("--file {}: not a regular file", file.display()),
                    ))
                }
                Err(err) => {
                    return Err(podnet_core::exit::fault(
                        podnet_core::exit::Fault::Config,
                        anyhow::anyhow!("--file {}: {err}", file.display()),
                    ))
                }
            }
        }
        let control = Arc::new(ShareControl::default());
        let router = match config.mode {
            ShareMode::Upload => upload_router(&config, control.clone()),
            ShareMode::Download => download_router(&config, control.clone())?,
            ShareMode::Proxy => unreachable!(),
        };
        Ok(Self {
            config,
            control,
            router,
        })
    }

    pub fn config(&self) -> &ShareConfig {
        &self.config
    }

    pub fn control(&self) -> Arc<ShareControl> {
        self.control.clone()
    }

    pub fn router(&self) -> Router {
        self.router.clone()
    }
}

mod download;
#[cfg(test)]
mod tests;
mod upload;

use download::*;
pub use upload::CHUNK_LIMIT;
use upload::*;

/// A path under `/s/` must name the session id of this share: the id is the
/// secret of the URL. A wrong id (compared in constant time) answers 404.
async fn check_session(
    State(token): State<Arc<String>>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    if let Some(rest) = request.uri().path().strip_prefix("/s/") {
        let id = rest.split('/').next().unwrap_or("");
        if !crate::gate::constant_time_eq(id.as_bytes(), token.as_bytes()) {
            return (StatusCode::NOT_FOUND, "Not Found").into_response();
        }
    }
    next.run(request).await
}

#[derive(Deserialize)]
pub(crate) struct UnlockForm {
    pub(crate) password: String,
    #[serde(default)]
    pub(crate) next: String,
}

/// The host of a request: the `Host` header, else the authority of the URI.
fn request_host<'a>(headers: &'a HeaderMap, uri: &'a Uri) -> Option<&'a str> {
    headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .or_else(|| uri.authority().map(|a| a.as_str()))
}

pub(crate) fn unlock_response(
    gate: &Gate,
    headers: &HeaderMap,
    uri: &Uri,
    password: &str,
    next: &str,
) -> Response {
    // Issue 229: an unlock from another site does not use a try of the PIN.
    if !crate::gate::same_site(headers, request_host(headers, uri)) {
        return (
            StatusCode::FORBIDDEN,
            "an unlock from another site is refused",
        )
            .into_response();
    }
    match gate.unlock(password) {
        Some(cookie) => {
            let next = crate::gate::safe_next(next);
            let mut response = Redirect::to(next).into_response();
            if let Ok(value) = header::HeaderValue::from_str(&cookie) {
                response.headers_mut().insert(header::SET_COOKIE, value);
            }
            response
        }
        // Issue 194: a wrong PIN is a refusal (the browser shows the page).
        None => (
            http::StatusCode::FORBIDDEN,
            Html(gate.page_html(next, true)),
        )
            .into_response(),
    }
}

pub(crate) fn gate_blocked(
    gate: &Gate,
    method: &http::Method,
    headers: &HeaderMap,
    uri: &Uri,
) -> Option<Response> {
    if uri.path() == UNLOCK_PATH || gate.is_open(headers) {
        return None;
    }
    let next = uri.path_and_query().map(|pq| pq.as_str()).unwrap_or("/");
    let status = http::StatusCode::from_u16(crate::gate::locked_status(method.as_str()))
        .unwrap_or(http::StatusCode::FORBIDDEN);
    Some((status, Html(gate.page_html(next, false))).into_response())
}

use crate::gate::escape_html as html_escape;

/// Issue 271: each response of a share session has `Cache-Control:
/// no-store` (the landing page, the file, the PIN page, the unlock reply).
/// A cache of the edge or of a browser keeps no copy.
async fn no_store(mut response: Response) -> Response {
    response
        .headers_mut()
        .entry(header::CACHE_CONTROL)
        .or_insert(header::HeaderValue::from_static("no-store"));
    response
}

/// `Content-Disposition` for a download of `name` (RFC 6266): `filename=`
/// with an ASCII form (each other byte, `"`, `\` and each control
/// character becomes `_`), and `filename*=UTF-8''` with the percent-encoded
/// name (RFC 5987) when the name is not plain ASCII. Before, a non-ASCII
/// name was put in raw, and the header was lost on the way to the edge.
pub fn content_disposition(name: &str) -> String {
    let ascii: String = name
        .chars()
        .map(|c| {
            if c.is_ascii() && !c.is_ascii_control() && c != '"' && c != '\\' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let mut value = format!("attachment; filename=\"{ascii}\"");
    if ascii != name {
        let mut encoded = String::new();
        for byte in name.bytes() {
            // attr-char of RFC 5987.
            if byte.is_ascii_alphanumeric() || b"!#$&+-.^_`|~".contains(&byte) {
                encoded.push(byte as char);
            } else {
                encoded.push_str(&format!("%{byte:02X}"));
            }
        }
        value.push_str(&format!("; filename*=UTF-8''{encoded}"));
    }
    value
}
