//! The file server: the files of a directory, with the guard of the root,
//! ranges and conditional requests (`tower_http::services::ServeDir`), a
//! listing of a directory in HTML and JSON (`listing`) and a directory as
//! one tar stream (`?tar`, T-085).
//!
//! The guard of the root refuses a dot segment, a path out of the root, a
//! link out of the root and each file that is not a regular file or a
//! directory.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{bail, Result};
use axum::Router;
use tower_http::services::{ServeDir, ServeFile};

mod listing;

/// The configuration of a file server on one directory.
#[derive(Clone, Debug)]
pub struct FilesConfig {
    /// The directory to serve.
    pub root: PathBuf,
    /// Each unknown path answers `index.html` (a single-page app).
    pub spa: bool,
    /// `GET DIR/?tar` gives the directory as one ustar stream.
    pub tar: bool,
    /// A directory with no `index.html` answers a listing (T-088).
    pub list: bool,
}

impl FilesConfig {
    /// The files of `root`, with no listing, no tar and no SPA fallback.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            spa: false,
            tar: false,
            list: false,
        }
    }
}

/// The router of a `static:` or `spa:` origin: the guard of the root and the
/// files of `root`. A missing directory is a usage error (exit 64).
pub fn static_router(root: &Path, spa: bool) -> Result<Router> {
    static_router_with(root, spa, false)
}

/// [`static_router`], and with `tar`, a GET of a directory with the query
/// `?tar` gives the directory as one ustar stream (T-085).
pub fn static_router_with(root: &Path, spa: bool, tar: bool) -> Result<Router> {
    router(&FilesConfig {
        spa,
        tar,
        ..FilesConfig::new(root)
    })
}

/// The router of the file server of `config`.
pub fn router(config: &FilesConfig) -> Result<Router> {
    let root = &config.root;
    // Issue 95: a bad --dir is a usage mistake (exit 64), and the text
    // says which mistake.
    if !root.is_dir() {
        let why = if root.exists() {
            "is not a directory"
        } else {
            "does not exist"
        };
        return Err(podnet_core::exit::fault(
            podnet_core::exit::Fault::Usage,
            anyhow::anyhow!("the directory to serve ({}) {why}", root.display()),
        ));
    }
    let canonical = Arc::new(
        root.canonicalize()
            .map_err(|e| anyhow::anyhow!("static root {}: {e}", root.display()))?,
    );
    let dir = ServeDir::new(canonical.as_path()).append_index_html_on_directories(true);
    if config.spa && !canonical.join("index.html").is_file() {
        bail!(
            "--spa needs an index.html in {}: each unknown path answers it",
            root.display()
        );
    }
    let router = if config.spa {
        let index = ServeFile::new(canonical.join("index.html"));
        // fallback, not not_found_service: an SPA route answers 200.
        Router::new().fallback_service(dir.fallback(index))
    } else {
        // Issue 182: a missing path has the body of each other 404 of
        // the guard ("Not Found"); before, ServeDir gave an empty body.
        let missing = tower::service_fn(|_: http::Request<axum::body::Body>| async {
            Ok::<_, std::convert::Infallible>(not_found())
        });
        Router::new().fallback_service(dir.not_found_service(missing))
    };
    let state = Arc::new(StaticState {
        root: canonical.as_ref().clone(),
        tar: config.tar,
        list: config.list,
    });
    Ok(router.layer(axum::middleware::from_fn_with_state(state, static_guard)))
}

/// The state of [`static_guard`].
struct StaticState {
    /// The canonical root.
    root: PathBuf,
    /// Give a directory as a tar stream on `?tar`.
    tar: bool,
    /// Give a listing of a directory with no `index.html`.
    list: bool,
}

/// The answer of [`static_guard`] for a path that is not served.
pub(crate) fn not_found() -> axum::response::Response {
    axum::response::IntoResponse::into_response((http::StatusCode::NOT_FOUND, "Not Found"))
}

/// A segment of a request path that the file server does not serve: a
/// name that starts with a dot (also `.` and `..`), a name of more than
/// 255 bytes, a backslash and a NUL.
pub(crate) fn refused_segment(segment: &str) -> bool {
    segment.starts_with('.')
        || segment.len() > 255
        || segment.contains('\\')
        || segment.contains('\0')
}

/// The path under `root` of a request path, or `None` for a path that the
/// guard refuses (a bad escape, a refused segment, more than 4096 bytes).
pub(crate) fn tree_path(root: &Path, request_path: &str) -> Option<PathBuf> {
    let decoded = percent_decode_path(request_path)?;
    let mut path = root.to_path_buf();
    for segment in decoded.split('/').filter(|s| !s.is_empty()) {
        if refused_segment(segment) {
            return None;
        }
        path.push(segment);
    }
    (path.as_os_str().len() <= 4096).then_some(path)
}

/// Serve a path of the static root only when it stays in the root:
///
/// * A name that starts with a dot (`.git`, `.env`) is not served.
/// * The real path (each symlink resolved) must be in the canonical root, so
///   a link to `/etc/passwd` or `../secret` is not served.
/// * Only a regular file or a directory is served: a FIFO, a socket or a
///   device would hang or leak.
///
/// A link that is changed between this check and the open is still a race;
/// the root must not be writable by an attacker.
async fn static_guard(
    axum::extract::State(state): axum::extract::State<Arc<StaticState>>,
    mut request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    // More than one range: answer the whole file (200), as RFC 9110 permits,
    // in place of the 416 of ServeDir for a multipart range.
    let multi = request
        .headers()
        .get(http::header::RANGE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains(','));
    if multi {
        request.headers_mut().remove(http::header::RANGE);
    }
    let root = &state.root;
    let Some(path) = tree_path(root, request.uri().path()) else {
        return not_found();
    };
    match std::fs::symlink_metadata(&path) {
        // A missing path: ServeDir answers 404 (or the SPA index).
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => next.run(request).await,
        Err(_) => not_found(),
        Ok(_) => match std::fs::canonicalize(&path) {
            Ok(real) if real.starts_with(root.as_path()) => match std::fs::metadata(&real) {
                Ok(meta) if meta.is_dir() && state.tar && wants_tar(&request) => {
                    crate::tar_http::answer(root.clone(), real, request.method())
                }
                Ok(meta)
                    if meta.is_dir()
                        && state.list
                        && is_read(request.method())
                        && !real.join("index.html").is_file() =>
                {
                    listing::answer(root, &real, &request)
                }
                Ok(meta) if meta.is_file() || meta.is_dir() => next.run(request).await,
                _ => not_found(),
            },
            _ => not_found(),
        },
    }
}

/// GET or HEAD.
pub(crate) fn is_read(method: &http::Method) -> bool {
    method == http::Method::GET || method == http::Method::HEAD
}

/// A GET or HEAD with the query key `tar` (`?tar`, `?tar=1`).
fn wants_tar(request: &axum::extract::Request) -> bool {
    is_read(request.method())
        && request
            .uri()
            .query()
            .is_some_and(|q| q.split('&').any(|p| p == "tar" || p.starts_with("tar=")))
}

/// Decode the `%XX` escapes of a URL path. A bad escape or bytes that are
/// not UTF-8 give `None`.
pub(crate) fn percent_decode_path(path: &str) -> Option<String> {
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Encode a name for one segment of a URL path: each byte that is not
/// unreserved (RFC 3986) becomes `%XX`.
pub(crate) fn percent_encode_segment(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for byte in name.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests;
