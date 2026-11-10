//! The download part of a share session: the router, the landing page and the
//! file response with ranges.

use super::*;

pub(super) fn download_router(config: &ShareConfig, control: Arc<ShareControl>) -> Result<Router> {
    let file = config
        .file
        .clone()
        .context("download session needs a file")?;
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "download".into());
    let state = Arc::new(DownloadState {
        token: config.token.clone(),
        file,
        name,
        stop_after: config.stop_after,
        gate: config.gate.clone(),
        control,
    });
    Ok(Router::new()
        .route(
            UNLOCK_PATH,
            post(unlock_download)
                .layer(DefaultBodyLimit::max(crate::gate::MAX_UNLOCK_BODY as usize)),
        )
        .route("/s/{token}", get(download_landing))
        .route("/s/{token}/", get(download_landing))
        .route("/s/{token}/file", get(download_file))
        .layer(axum::middleware::from_fn_with_state(
            Arc::new(config.token.clone()),
            check_session,
        ))
        .layer(axum::middleware::map_response(no_store))
        .with_state(state))
}

pub(super) async fn unlock_download(
    State(state): State<Arc<DownloadState>>,
    headers: HeaderMap,
    uri: Uri,
    Form(form): Form<UnlockForm>,
) -> Response {
    unlock_response(&state.gate, &headers, &uri, &form.password, &form.next)
}

pub(super) async fn download_landing(
    State(state): State<Arc<DownloadState>>,
    method: http::Method,
    headers: HeaderMap,
    uri: Uri,
) -> Response {
    if let Some(response) = gate_blocked(&state.gate, &method, &headers, &uri) {
        return response;
    }
    state.control.touch();
    Html(format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <title>podnet — download</title></head><body>\
         <h1>{}</h1><p><a href=\"/s/{}/file\">Download</a></p></body></html>",
        html_escape(&state.name),
        state.token
    ))
    .into_response()
}

pub(super) async fn download_file(
    State(state): State<Arc<DownloadState>>,
    method: http::Method,
    headers: HeaderMap,
    uri: Uri,
) -> Response {
    if let Some(response) = gate_blocked(&state.gate, &method, &headers, &uri) {
        return response;
    }
    state.control.touch();
    // podssh #105: open with O_NONBLOCK (an open of a FIFO does not
    // block), then serve only a regular file. A FIFO put at the path after
    // the start blocked the request.
    let mut file = match tokio::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(&state.file)
        .await
    {
        Ok(file) => file,
        Err(err) => {
            // Issue 234: the error detail goes to the log, not to the visitor.
            tracing::warn!(error = %err, file = %state.file.display(), "download: cannot open the shared file");
            return (StatusCode::NOT_FOUND, "the shared file is not available").into_response();
        }
    };
    let meta = file.metadata().await.ok();
    if !meta.as_ref().is_some_and(|m| m.is_file()) {
        return (
            StatusCode::NOT_FOUND,
            "the shared file is not a regular file",
        )
            .into_response();
    }
    let len = meta.as_ref().map(|m| m.len()).unwrap_or(0);
    let mtime = meta
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let etag = format!("\"{len:x}-{mtime:x}\"");
    let header_text = |name: header::HeaderName| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };
    if header_text(header::IF_NONE_MATCH)
        .is_some_and(|v| crate::util::http::etag_matches(&v, &etag))
    {
        return Response::builder()
            .status(StatusCode::NOT_MODIFIED)
            .header(header::ETAG, &etag)
            .body(Body::empty())
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
    }
    // One range (a resume). An If-Range that is not this ETag, and more than
    // one range, give the whole file.
    let range_ok = header_text(header::IF_RANGE).is_none_or(|v| v.trim() == etag);
    let range = match header_text(header::RANGE).filter(|_| range_ok) {
        Some(value) => match parse_range(&value, len) {
            Ok(range) => range,
            Err(()) => {
                return Response::builder()
                    .status(StatusCode::RANGE_NOT_SATISFIABLE)
                    .header(header::CONTENT_RANGE, format!("bytes */{len}"))
                    .body(Body::empty())
                    .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
            }
        },
        None => None,
    };
    let (start, end) = range.unwrap_or((0, len.saturating_sub(1)));
    let count = if len == 0 { 0 } else { end - start + 1 };
    if start > 0 {
        use tokio::io::AsyncSeekExt;
        if file.seek(std::io::SeekFrom::Start(start)).await.is_err() {
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }
    let disposition = content_disposition(&state.name);
    let stream = tokio_util::io::ReaderStream::new(tokio::io::AsyncReadExt::take(file, count));
    // --stop-after: the session ends when the last byte of the file has gone
    // out in a GET. A HEAD request, a range that stops before the end, and a
    // client that closes early, do not end it.
    let to_the_end = len == 0 || end + 1 == len;
    let body = if state.stop_after && method != http::Method::HEAD && to_the_end {
        use futures::StreamExt;
        let control = state.control.clone();
        let done = futures::stream::once(async move {
            control.request_stop();
            None::<std::io::Result<bytes::Bytes>>
        })
        .filter_map(|item| async move { item });
        Body::from_stream(stream.chain(done))
    } else {
        Body::from_stream(stream)
    };
    let mut response = Response::builder()
        .status(if range.is_some() {
            StatusCode::PARTIAL_CONTENT
        } else {
            StatusCode::OK
        })
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_LENGTH, count)
        .header(header::CONTENT_DISPOSITION, disposition)
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::ETAG, &etag);
    if range.is_some() {
        response = response.header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{len}"));
    }
    response
        .body(body)
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// Parse a Range header for a file of `len` bytes: `Ok(Some((start, end)))`
/// for one satisfiable range (end included), `Ok(None)` for a header to
/// ignore (more than one range, another unit), `Err` for an unsatisfiable
/// range.
pub(super) fn parse_range(value: &str, len: u64) -> std::result::Result<Option<(u64, u64)>, ()> {
    let Some(spec) = value.trim().strip_prefix("bytes=") else {
        return Ok(None);
    };
    if spec.contains(',') {
        return Ok(None);
    }
    let (a, b) = spec.split_once('-').ok_or(())?;
    let (a, b) = (a.trim(), b.trim());
    let range = if a.is_empty() {
        let n: u64 = b.parse().map_err(|_| ())?;
        if n == 0 || len == 0 {
            return Err(());
        }
        (len.saturating_sub(n), len - 1)
    } else {
        let start: u64 = a.parse().map_err(|_| ())?;
        let end = if b.is_empty() {
            len.saturating_sub(1)
        } else {
            b.parse::<u64>().map_err(|_| ())?.min(len.saturating_sub(1))
        };
        if start >= len || start > end {
            return Err(());
        }
        (start, end)
    };
    Ok(Some(range))
}

// ── helpers ─────────────────────────────────────────────────────────────────
