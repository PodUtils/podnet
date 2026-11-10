//! Uploads into the tree of the file server (T-090): a PUT of one file
//! (`curl -T`) and a multipart form to a directory. Each file goes to a
//! hidden part file (mode 0600) in its directory, and a rename gives it its
//! name: a reader never sees a partial file. A request over the limit gets
//! 413 and leaves no file.

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::extract::{FromRequest, Multipart};
use axum::response::{Html, IntoResponse, Response};
use futures::StreamExt as _;
use tokio::io::AsyncWriteExt as _;

use crate::gate::escape_html;
use crate::share::CHUNK_LIMIT;
use podnet_core::util::{human_bytes, sanitize_filename};

/// A part file that is removed on drop, until [`Part::commit`].
struct Part {
    path: PathBuf,
    file: Option<tokio::fs::File>,
}

impl Drop for Part {
    fn drop(&mut self) {
        if !self.path.as_os_str().is_empty() {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

impl Part {
    /// A new part file in `dir`.
    async fn create(dir: &Path) -> std::io::Result<Self> {
        let path = dir.join(format!(
            ".podnet-upload-{}.part",
            &podnet_core::util::random_token()[..16]
        ));
        let file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .await?;
        Ok(Self {
            path,
            file: Some(file),
        })
    }

    async fn write(&mut self, data: &[u8]) -> std::io::Result<()> {
        match self.file.as_mut() {
            Some(file) => file.write_all(data).await,
            None => Err(std::io::Error::other("the part file is closed")),
        }
    }

    /// Flush the part file and give it the name `target`.
    async fn commit(mut self, target: &Path) -> std::io::Result<()> {
        if let Some(mut file) = self.file.take() {
            file.flush().await?;
            file.sync_all().await?;
        }
        tokio::fs::rename(&self.path, target).await?;
        self.path = PathBuf::new();
        Ok(())
    }
}

/// A refusal with a text body.
fn refuse(status: http::StatusCode, text: impl Into<String>) -> Response {
    (status, format!("{}\n", text.into())).into_response()
}

/// An error of the file system: the operator gets the detail in the log,
/// the visitor a fixed text.
fn failed(what: &str, err: &std::io::Error) -> Response {
    tracing::warn!(error = %err, "files: upload: {what}");
    refuse(
        http::StatusCode::INTERNAL_SERVER_ERROR,
        "the server cannot save the upload",
    )
}

fn too_large(max: u64) -> Response {
    refuse(
        http::StatusCode::PAYLOAD_TOO_LARGE,
        format!(
            "the upload is over {}, the most of one request; no file was saved",
            human_bytes(max)
        ),
    )
}

/// The `Content-Length` of a request.
fn content_length(request: &axum::extract::Request) -> Option<u64> {
    request
        .headers()
        .get(http::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse().ok())
}

/// The directory `path` when it is a directory in the canonical `root`.
fn dir_in_root(root: &Path, path: &Path) -> Option<PathBuf> {
    let real = std::fs::canonicalize(path).ok()?;
    (real.starts_with(root) && real.is_dir()).then_some(real)
}

/// The answer to a PUT or a POST of the file server with uploads on. The
/// access layer gave the write role before this call.
pub(crate) async fn answer(root: &Path, max: u64, request: axum::extract::Request) -> Response {
    let Some(path) = super::tree_path(root, request.uri().path()) else {
        return refuse(
            http::StatusCode::FORBIDDEN,
            "a name that starts with a dot, a backslash and a NUL are refused",
        );
    };
    if content_length(&request).is_some_and(|n| n > max) {
        return too_large(max);
    }
    if request.method() == http::Method::PUT {
        put(root, &path, max, request).await
    } else if request.method() == http::Method::POST {
        form(root, &path, max, request).await
    } else {
        refuse(
            http::StatusCode::METHOD_NOT_ALLOWED,
            "use GET, HEAD, PUT or POST",
        )
    }
}

/// PUT: the body is the file at the path. A new file gives 201, a
/// replaced file 204. The parent directory must exist.
async fn put(root: &Path, path: &Path, max: u64, request: axum::extract::Request) -> Response {
    if request.uri().path().ends_with('/') || path == root {
        return refuse(http::StatusCode::METHOD_NOT_ALLOWED, "PUT names a file");
    }
    let Some(parent) = path.parent().and_then(|p| dir_in_root(root, p)) else {
        return refuse(
            http::StatusCode::CONFLICT,
            "the directory of the file does not exist",
        );
    };
    let Some(name) = path.file_name() else {
        return refuse(http::StatusCode::BAD_REQUEST, "no file name");
    };
    let target = parent.join(name);
    let existed = match std::fs::symlink_metadata(&target) {
        Ok(meta) if meta.is_file() => true,
        Ok(_) => {
            return refuse(
                http::StatusCode::CONFLICT,
                "the path is a directory, a link or a special file",
            )
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => return failed("cannot read the target", &e),
    };
    let mut part = match Part::create(&parent).await {
        Ok(part) => part,
        Err(e) => return failed("cannot create a part file", &e),
    };
    let mut body = request.into_body().into_data_stream();
    let mut written = 0u64;
    loop {
        let chunk = match tokio::time::timeout(CHUNK_LIMIT, body.next()).await {
            Ok(Some(Ok(chunk))) => chunk,
            Ok(None) => break,
            Ok(Some(Err(err))) => {
                return refuse(
                    http::StatusCode::BAD_REQUEST,
                    format!("the upload stopped: {err}"),
                )
            }
            Err(_) => {
                return refuse(
                    http::StatusCode::REQUEST_TIMEOUT,
                    format!("no data for {} s", CHUNK_LIMIT.as_secs()),
                )
            }
        };
        written += chunk.len() as u64;
        if written > max {
            return too_large(max);
        }
        if let Err(e) = part.write(&chunk).await {
            return failed("cannot write a part file", &e);
        }
    }
    if let Err(e) = part.commit(&target).await {
        return failed("cannot move a part file to its name", &e);
    }
    tracing::info!(bytes = written, path = %target.display(), "files: a PUT is saved");
    if existed {
        http::StatusCode::NO_CONTENT.into_response()
    } else {
        (http::StatusCode::CREATED, "created\n").into_response()
    }
}

/// POST of a multipart form to a directory: each file field becomes a
/// file of the directory. A name in use gets a number (`name-1.ext`); no
/// file is replaced.
async fn form(root: &Path, path: &Path, max: u64, request: axum::extract::Request) -> Response {
    let Some(dir) = dir_in_root(root, path) else {
        return refuse(
            http::StatusCode::CONFLICT,
            "POST a form to a directory that exists",
        );
    };
    let back = request.uri().path().to_string();
    let mut multipart = match Multipart::from_request(request.map(Body::new), &()).await {
        Ok(m) => m,
        Err(rejection) => return rejection.into_response(),
    };
    let mut parts: Vec<(Part, String, u64)> = Vec::new();
    let mut total = 0u64;
    loop {
        let mut field = match tokio::time::timeout(CHUNK_LIMIT, multipart.next_field()).await {
            Ok(Ok(Some(field))) => field,
            Ok(Ok(None)) => break,
            Ok(Err(err)) => {
                return refuse(
                    http::StatusCode::BAD_REQUEST,
                    format!("a bad multipart body: {err}"),
                )
            }
            Err(_) => {
                return refuse(
                    http::StatusCode::REQUEST_TIMEOUT,
                    format!("no data for {} s", CHUNK_LIMIT.as_secs()),
                )
            }
        };
        let name = field.file_name().map(sanitize_filename);
        let mut part = match &name {
            Some(_) => match Part::create(&dir).await {
                Ok(part) => Some(part),
                Err(e) => return failed("cannot create a part file", &e),
            },
            None => None,
        };
        let mut size = 0u64;
        loop {
            let chunk = match crate::share::next_chunk(&mut field).await {
                Ok(Some(chunk)) => chunk,
                Ok(None) => break,
                Err(response) => return *response,
            };
            total += chunk.len() as u64;
            if total > max {
                return too_large(max);
            }
            size += chunk.len() as u64;
            if let Some(part) = part.as_mut() {
                if let Err(e) = part.write(&chunk).await {
                    return failed("cannot write a part file", &e);
                }
            }
        }
        if let (Some(part), Some(name)) = (part, name) {
            parts.push((part, name, size));
        }
    }
    if parts.is_empty() {
        return refuse(http::StatusCode::BAD_REQUEST, "no file in the form");
    }
    let mut saved = Vec::new();
    for (part, name, size) in parts {
        let target = match crate::share::create_unique(&dir, &name).await {
            Ok((target, placeholder)) => {
                drop(placeholder);
                target
            }
            Err(e) => return failed("cannot make a file name", &e),
        };
        // The placeholder (made with create_new) is ours: the rename
        // replaces it, and never another file.
        if let Err(e) = part.commit(&target).await {
            let _ = std::fs::remove_file(&target);
            return failed("cannot move a part file to its name", &e);
        }
        let final_name = target
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or(name);
        saved.push((final_name, size));
    }
    let list: String = saved
        .iter()
        .map(|(name, size)| format!("<li>{} ({})</li>", escape_html(name), human_bytes(*size)))
        .collect();
    (
        http::StatusCode::CREATED,
        Html(format!(
            "<!doctype html><html><head><meta charset=\"utf-8\">\
             <title>podnet: uploaded</title></head><body>\
             <h1>Saved {} file(s)</h1><ul>{list}</ul>\
             <p><a href=\"{}\">Back</a></p></body></html>",
            saved.len(),
            escape_html(&back)
        )),
    )
        .into_response()
}

/// The form of a listing for a write user.
pub(crate) fn form_html() -> &'static str {
    "<form method=\"post\" enctype=\"multipart/form-data\">\
     <input type=\"file\" name=\"files\" multiple required> \
     <button type=\"submit\">Upload</button></form>"
}

#[cfg(test)]
#[path = "upload_tests.rs"]
mod tests;
