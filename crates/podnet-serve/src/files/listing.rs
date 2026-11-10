//! The listing of a directory of the file server (T-088): HTML for a
//! browser, JSON for `Accept: application/json`. The listing leaves out
//! each name that starts with a dot, each name that the guard refuses,
//! each link out of the root and each file that is not a regular file or a
//! directory.

use std::path::Path;

use axum::response::{Html, IntoResponse, Redirect, Response};

use crate::gate::escape_html;

/// One entry of a listing.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Entry {
    pub(crate) name: String,
    pub(crate) dir: bool,
    pub(crate) size: u64,
    /// The time of the last change, in seconds after the Unix epoch.
    pub(crate) modified: u64,
}

/// The entries of the directory `real` (in the canonical `root`) that the
/// guard serves: directories first, then files, each group by name.
pub(crate) fn entries(root: &Path, real: &Path) -> std::io::Result<Vec<Entry>> {
    let mut out = Vec::new();
    for item in std::fs::read_dir(real)? {
        let Ok(item) = item else { continue };
        let Ok(name) = item.file_name().into_string() else {
            continue;
        };
        if super::refused_segment(&name) {
            continue;
        }
        let path = item.path();
        let Ok(target) = std::fs::canonicalize(&path) else {
            continue;
        };
        if !target.starts_with(root) {
            continue;
        }
        let Ok(meta) = std::fs::metadata(&target) else {
            continue;
        };
        if !meta.is_dir() && !meta.is_file() {
            continue;
        }
        let modified = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        out.push(Entry {
            name,
            dir: meta.is_dir(),
            size: if meta.is_dir() { 0 } else { meta.len() },
            modified,
        });
    }
    out.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.cmp(&b.name)));
    Ok(out)
}

/// True when the client asks for JSON (`Accept: application/json`).
fn wants_json(request: &axum::extract::Request) -> bool {
    request
        .headers()
        .get(http::header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.split(',').any(|part| {
                part.split(';')
                    .next()
                    .is_some_and(|t| t.trim().eq_ignore_ascii_case("application/json"))
            })
        })
}

/// The answer for a GET or HEAD of the directory `real`. A path with no
/// `/` at its end is sent to the path with the `/` (308), so each relative
/// link of the listing names an entry of this directory.
pub(crate) fn answer(root: &Path, real: &Path, request: &axum::extract::Request) -> Response {
    let path = request.uri().path();
    if !path.ends_with('/') {
        let target = match request.uri().query() {
            Some(q) => format!("{path}/?{q}"),
            None => format!("{path}/"),
        };
        return Redirect::permanent(&target).into_response();
    }
    let list = match entries(root, real) {
        Ok(list) => list,
        Err(err) => {
            tracing::warn!(error = %err, "files: cannot read a directory");
            return super::not_found();
        }
    };
    let shown = super::percent_decode_path(path).unwrap_or_else(|| "/".into());
    let mut response = if wants_json(request) {
        json(&shown, &list).into_response()
    } else {
        Html(html(&shown, &list)).into_response()
    };
    response
        .headers_mut()
        .insert(http::header::VARY, http::HeaderValue::from_static("accept"));
    response
}

/// The JSON listing: `{"path": "/sub/", "entries": [{"name", "type",
/// "size", "modified"}]}`, with `type` `"dir"` or `"file"`.
pub(crate) fn json(path: &str, list: &[Entry]) -> Response {
    let entries: Vec<serde_json::Value> = list
        .iter()
        .map(|e| {
            serde_json::json!({
                "name": e.name,
                "type": if e.dir { "dir" } else { "file" },
                "size": e.size,
                "modified": e.modified,
            })
        })
        .collect();
    let doc = serde_json::json!({ "path": path, "entries": entries });
    (
        [(http::header::CONTENT_TYPE, "application/json")],
        doc.to_string(),
    )
        .into_response()
}

/// The HTML listing. Each name is escaped, and each link is the name with
/// each byte that is not unreserved as `%XX`.
pub(crate) fn html(path: &str, list: &[Entry]) -> String {
    let title = escape_html(path);
    let mut rows = String::new();
    if path != "/" {
        rows.push_str("<tr><td><a href=\"../\">../</a></td><td></td></tr>");
    }
    for e in list {
        let slash = if e.dir { "/" } else { "" };
        let href = escape_html(&super::percent_encode_segment(&e.name));
        let size = if e.dir {
            String::new()
        } else {
            podnet_core::util::human_bytes(e.size)
        };
        rows.push_str(&format!(
            "<tr><td><a href=\"{href}{slash}\">{}{slash}</a></td><td>{size}</td></tr>",
            escape_html(&e.name)
        ));
    }
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <title>Index of {title}</title><style>\
         body{{font-family:system-ui,sans-serif;margin:2rem}}\
         td{{padding:.2rem 1rem .2rem 0}}td+td{{text-align:right;color:#666}}\
         </style></head><body><h1>Index of {title}</h1>\
         <table>{rows}</table></body></html>"
    )
}
