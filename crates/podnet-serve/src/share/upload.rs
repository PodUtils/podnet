//! The upload part of a share session: the router, the landing page and the
//! receiver of the multipart form.

use super::*;

/// The time limit of one chunk of an upload. A body that stops for longer
/// gets 408, and its staged files are removed.
pub const CHUNK_LIMIT: std::time::Duration = std::time::Duration::from_secs(60);

pub(super) fn upload_router(config: &ShareConfig, control: Arc<ShareControl>) -> Router {
    let state = Arc::new(UploadState {
        dest: config.dest.clone(),
        token: config.token.clone(),
        max_bytes: config.max_bytes,
        max_files: config.max_files,
        max_total: config.max_total,
        max_request: config.max_request,
        used_files: AtomicU32::new(0),
        used_bytes: AtomicU64::new(0),
        stop_after: config.stop_after,
        gate: config.gate.clone(),
        control,
    });
    // One request may hold several files: the body limit is the session
    // total, and each file and the total are checked while they arrive.
    let limit = (config.max_total.max(config.max_bytes) as usize)
        .saturating_add(1024 * 1024)
        .min(config.max_request as usize);
    Router::new()
        .route(
            UNLOCK_PATH,
            post(unlock_upload).layer(DefaultBodyLimit::max(crate::gate::MAX_UNLOCK_BODY as usize)),
        )
        .route("/s/{token}", get(upload_landing))
        .route("/s/{token}/", get(upload_landing))
        // Live run 41556133042: with the body limit of the router on this
        // route, the multipart stream ended at the limit, the rest of a
        // refused body was not read, and the edge gave 500 over HTTP/2 in
        // place of the 413. The handler counts the bytes of the request.
        // Issue 232: this layer removes each body limit of axum on the
        // route. The counter of the handler is the only limit: it counts
        // each field, with or with no file name.
        .route(
            "/s/{token}/upload",
            post(upload_receive).layer(DefaultBodyLimit::disable()),
        )
        .layer(DefaultBodyLimit::max(limit))
        .layer(axum::middleware::from_fn_with_state(
            Arc::new(config.token.clone()),
            check_session,
        ))
        .layer(axum::middleware::map_response(no_store))
        .with_state(state)
}

pub(super) async fn unlock_upload(
    State(state): State<Arc<UploadState>>,
    headers: HeaderMap,
    uri: Uri,
    Form(form): Form<UnlockForm>,
) -> Response {
    unlock_response(&state.gate, &headers, &uri, &form.password, &form.next)
}

pub(super) async fn upload_landing(
    State(state): State<Arc<UploadState>>,
    method: http::Method,
    headers: HeaderMap,
    uri: Uri,
) -> Response {
    if let Some(response) = gate_blocked(&state.gate, &method, &headers, &uri) {
        return response;
    }
    state.control.touch();
    Html(upload_page(
        &state.token,
        state.max_bytes,
        state.max_request,
    ))
    .into_response()
}

/// The files of one upload request before they are committed. Each file is
/// written to a hidden `.part` file in the destination. On drop, before the
/// commit, each part file is removed and the reserved file slots and bytes
/// go back to the session: an error, a stalled body or a dropped request
/// leaves no partial file and no used slot.
pub(super) struct Staging<'a> {
    state: &'a UploadState,
    /// (part file, sanitized name, size)
    parts: Vec<(PathBuf, String, u64)>,
    slots: u32,
    bytes: u64,
}

impl Drop for Staging<'_> {
    fn drop(&mut self) {
        for (part, _, _) in &self.parts {
            let _ = std::fs::remove_file(part);
        }
        self.state
            .used_files
            .fetch_sub(self.slots, Ordering::SeqCst);
        self.state
            .used_bytes
            .fetch_sub(self.bytes, Ordering::SeqCst);
    }
}

impl Staging<'_> {
    /// Take `n` bytes of the session total, or say no.
    fn reserve_bytes(&mut self, n: u64) -> bool {
        let max = self.state.max_total;
        let used_bytes = &self.state.used_bytes;
        let ok = update_atomic!(used_bytes, |used: u64| used
            .checked_add(n)
            .filter(|t| *t <= max))
        .is_ok();
        if ok {
            self.bytes += n;
        }
        ok
    }

    /// Move each part file to a unique final name. A committed file is no
    /// longer removed on drop, and its slot and bytes stay used.
    async fn commit(mut self) -> Result<Vec<(String, u64)>, std::io::Error> {
        let mut saved = Vec::new();
        let parts = std::mem::take(&mut self.parts);
        let mut left = parts.clone();
        for (part, name, size) in parts {
            let placed = match create_unique(&self.state.dest, &name).await {
                Ok((path, placeholder)) => {
                    drop(placeholder);
                    // The placeholder (made with create_new) is ours: the
                    // rename replaces it, and never another file.
                    match tokio::fs::rename(&part, &path).await {
                        Ok(()) => Ok(path),
                        Err(err) => {
                            let _ = tokio::fs::remove_file(&path).await;
                            Err(err)
                        }
                    }
                }
                Err(err) => Err(err),
            };
            let path = match placed {
                Ok(path) => path,
                Err(err) => {
                    // The committed files keep their slots and bytes; the
                    // rest are removed and released on drop.
                    self.slots = left.len() as u32;
                    self.bytes = left.iter().map(|(_, _, size)| size).sum();
                    self.parts = left;
                    return Err(err);
                }
            };
            left.remove(0);
            let final_name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or(name);
            saved.push((final_name, size));
        }
        // Committed: keep the slots and the bytes.
        self.slots = 0;
        self.bytes = 0;
        Ok(saved)
    }
}

/// Issue 234: an error of the file system goes to the log of the operator.
/// The visitor gets a fixed text, with no error detail of the host.
pub(super) fn save_failed(what: &str, err: &std::io::Error) -> Response {
    tracing::warn!(error = %err, "upload: {what}");
    refuse(
        StatusCode::INTERNAL_SERVER_ERROR,
        "the server cannot save the upload".to_string(),
    )
}

pub(super) fn refuse(status: StatusCode, message: String) -> Response {
    (
        status,
        format!("{message}; no file of this upload was saved"),
    )
        .into_response()
}

/// The next chunk of a multipart field, with the time limit [`CHUNK_LIMIT`].
/// An error is the response to send.
pub(super) async fn next_chunk(
    field: &mut axum::extract::multipart::Field<'_>,
) -> Result<Option<bytes::Bytes>, Box<Response>> {
    match tokio::time::timeout(CHUNK_LIMIT, field.chunk()).await {
        Ok(Ok(chunk)) => Ok(chunk),
        Ok(Err(err)) => Err(Box::new(refuse(
            StatusCode::BAD_REQUEST,
            format!("upload aborted: {err}"),
        ))),
        Err(_) => Err(Box::new(refuse(
            StatusCode::REQUEST_TIMEOUT,
            format!("no data for {} s", CHUNK_LIMIT.as_secs()),
        ))),
    }
}

/// The longest pause of a body that is read and dropped.
pub(super) const DROP_IDLE: std::time::Duration = std::time::Duration::from_secs(5);
/// The longest time to read and drop a body.
pub(super) const DROP_LIMIT: std::time::Duration = std::time::Duration::from_secs(120);

/// Read the rest of a multipart body and drop it: until its end, a pause of
/// [`DROP_IDLE`], or [`DROP_LIMIT`]. Gives the bytes dropped.
pub(super) async fn drop_body(multipart: &mut Multipart) -> u64 {
    let mut dropped = 0u64;
    let work = async {
        while let Ok(Ok(Some(mut field))) =
            tokio::time::timeout(DROP_IDLE, multipart.next_field()).await
        {
            while let Ok(Ok(Some(chunk))) = tokio::time::timeout(DROP_IDLE, field.chunk()).await {
                dropped += chunk.len() as u64;
            }
        }
    };
    let _ = tokio::time::timeout(DROP_LIMIT, work).await;
    dropped
}

pub(super) async fn upload_receive(
    State(state): State<Arc<UploadState>>,
    method: http::Method,
    headers: HeaderMap,
    uri: Uri,
    mut multipart: Multipart,
) -> Response {
    if let Some(response) = gate_blocked(&state.gate, &method, &headers, &uri) {
        return response;
    }
    state.control.touch();
    // A request over the limit (by its Content-Length, issue 92) gets 413
    // and saves nothing. The body is read and dropped first: an answer
    // before the end of the upload did not reach the visitor reliably
    // through the edge (a 502 or a 520 in place of the 413).
    let length = headers
        .get(http::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok());
    if let Some(length) = length.filter(|&n| n > state.max_request) {
        tracing::info!(
            bytes = length,
            limit = state.max_request,
            "upload refused: the request is over --max-request"
        );
        let dropped = drop_body(&mut multipart).await;
        tracing::debug!(dropped, "the body of a refused upload was read and dropped");
        return refuse(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!(
                "the upload is {}; one upload can hold at most {} (send fewer or smaller files)",
                human_bytes(length),
                human_bytes(state.max_request)
            ),
        );
    }

    let mut staging = Staging {
        state: &state,
        parts: Vec::new(),
        slots: 0,
        bytes: 0,
    };
    let mut request_bytes = 0u64;
    let mut over = false;
    'fields: loop {
        let field = match tokio::time::timeout(CHUNK_LIMIT, multipart.next_field()).await {
            Ok(Ok(Some(field))) => field,
            Ok(Ok(None)) => break,
            Ok(Err(err)) => {
                return refuse(
                    StatusCode::BAD_REQUEST,
                    format!("bad multipart body: {err}"),
                )
            }
            Err(_) => {
                return refuse(
                    StatusCode::REQUEST_TIMEOUT,
                    format!("no data for {} s", CHUNK_LIMIT.as_secs()),
                )
            }
        };
        let Some(original) = field.file_name().map(str::to_string) else {
            // Issue 232: a field with no file name counts toward
            // --max-request too. Its bytes are dropped.
            let mut field = field;
            loop {
                let chunk = match next_chunk(&mut field).await {
                    Ok(Some(chunk)) => chunk,
                    Ok(None) => break,
                    Err(response) => return *response,
                };
                state.control.touch();
                request_bytes += chunk.len() as u64;
                if request_bytes > state.max_request {
                    over = true;
                    break 'fields;
                }
            }
            continue;
        };
        let name = sanitize_filename(&original);
        // Reserve a slot of the file limit first: two uploads at the same
        // time cannot both take the last one.
        let used_files = &state.used_files;
        let reserved = update_atomic!(used_files, |n| (n < state.max_files).then_some(n + 1));
        if reserved.is_err() {
            return refuse(
                StatusCode::PAYLOAD_TOO_LARGE,
                format!("session file limit reached ({} files)", state.max_files),
            );
        }
        staging.slots += 1;

        let part = state.dest.join(format!(
            ".podnet-upload-{}.part",
            &podnet_core::util::random_token()[..16]
        ));
        // Issue 233: an upload is for the operator only (mode 0600). The
        // rename keeps the mode of the part file.
        let mut file = match tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&part)
            .await
        {
            Ok(file) => file,
            Err(err) => return save_failed("cannot create a part file", &err),
        };
        staging.parts.push((part, name, 0));
        let index = staging.parts.len() - 1;
        let mut field = field;
        let mut written = 0u64;
        loop {
            let chunk = match next_chunk(&mut field).await {
                Ok(Some(chunk)) => chunk,
                Ok(None) => break,
                Err(response) => return *response,
            };
            // A long upload is activity: --idle-secs does not stop it.
            state.control.touch();
            // A request with no Content-Length over --max-request.
            request_bytes += chunk.len() as u64;
            if request_bytes > state.max_request {
                over = true;
                break 'fields;
            }
            written += chunk.len() as u64;
            if written > state.max_bytes {
                return refuse(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    format!("a file exceeds {}", human_bytes(state.max_bytes)),
                );
            }
            if !staging.reserve_bytes(chunk.len() as u64) {
                return refuse(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    format!(
                        "the session limit of {} is reached",
                        human_bytes(state.max_total)
                    ),
                );
            }
            if let Err(err) = tokio::io::AsyncWriteExt::write_all(&mut file, &chunk).await {
                return save_failed("cannot write a part file", &err);
            }
        }
        if let Err(err) = tokio::io::AsyncWriteExt::flush(&mut file).await {
            return save_failed("cannot write a part file", &err);
        }
        staging.parts[index].2 = written;
    }
    if over {
        tracing::info!(
            limit = state.max_request,
            "upload refused: the request is over --max-request"
        );
        let dropped = drop_body(&mut multipart).await;
        tracing::debug!(dropped, "the body of a refused upload was read and dropped");
        return refuse(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!(
                "the upload is over {}, the most of one upload (send fewer or smaller files)",
                human_bytes(state.max_request)
            ),
        );
    }

    if staging.parts.is_empty() {
        return (StatusCode::BAD_REQUEST, "no files in upload").into_response();
    }
    let saved = match staging.commit().await {
        Ok(saved) => saved,
        Err(err) => return save_failed("cannot move a part file to its name", &err),
    };
    let total: u64 = saved.iter().map(|(_, size)| size).sum();
    if state.stop_after {
        state.control.request_stop();
    }
    let list: String = saved
        .iter()
        .map(|(name, size)| format!("<li>{} — {}</li>", html_escape(name), human_bytes(*size)))
        .collect();
    Html(format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <title>podnet — uploaded</title></head><body>\
         <h1>Received {} file(s), {}</h1><ul>{list}</ul></body></html>",
        saved.len(),
        human_bytes(total)
    ))
    .into_response()
}

pub(super) fn upload_page(token: &str, max_bytes: u64, max_request: u64) -> String {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <title>podnet — upload</title><style>\
         body{{font-family:system-ui,sans-serif;background:#0b1020;color:#e8ecf5;\
         display:flex;align-items:center;justify-content:center;min-height:100vh;margin:0}}\
         main{{background:#161d33;padding:2rem;border-radius:12px;width:min(90vw,420px)}}\
         button{{margin-top:1rem;width:100%;padding:.8rem;border:0;border-radius:8px;\
         background:#4f7cff;color:#fff;font-size:1rem}}input[type=file]{{width:100%}}\
         p{{color:#9aa7c7;font-size:.9rem}}</style></head><body><main>\
         <h1>Send files</h1><p>Max {} per file, {} per upload.</p>\
         <form id=\"f\" method=\"post\" action=\"/s/{token}/upload\" enctype=\"multipart/form-data\">\
         <input type=\"file\" name=\"files\" multiple required>\
         <button type=\"submit\">Upload</button></form>\
         <progress id=\"bar\" max=\"100\" value=\"0\" hidden style=\"width:100%;margin-top:1rem\"></progress>\
         <p id=\"state\"></p></main><script>\
         var f=document.getElementById('f'),bar=document.getElementById('bar'),st=document.getElementById('state');\
         f.addEventListener('submit',function(e){{e.preventDefault();\
         var n=0,fs=f.querySelector('input[type=file]').files;for(var i=0;i<fs.length;i++)n+=fs[i].size;\
         if(n>{max_request}){{st.textContent='The files are '+n+' bytes; one upload can hold at most {max_request} bytes.';return;}}\
         var x=new XMLHttpRequest();\
         x.open('POST',f.action);bar.hidden=false;\
         x.upload.onprogress=function(p){{if(p.lengthComputable){{bar.value=100*p.loaded/p.total;\
         st.textContent=Math.round(bar.value)+' %';}}}};\
         x.onload=function(){{document.open();document.write(x.responseText);document.close();}};\
         x.onerror=function(){{st.textContent='The upload failed.';}};\
         x.send(new FormData(f));}});</script></body></html>",
        human_bytes(max_bytes),
        human_bytes(max_request)
    )
}

/// The numbered names that [`create_unique`] tries before random names.
pub(super) const NUMBERED_NAMES: u32 = 100;
/// The random names that [`create_unique`] tries after the numbered names.
pub(super) const RANDOM_NAMES: u32 = 16;

/// Make a new file for `name` in `dir`: `name`, else `stem-1.ext`,
/// `stem-2.ext` and so on, up to [`NUMBERED_NAMES`] names. After that,
/// `stem-<8 random hex digits>.ext` (issue 234: before, the stem was full
/// after 10 000 names, and each upload tried each name). Each try uses
/// `create_new`, so two uploads at the same time never get the same file.
pub(super) async fn create_unique(
    dir: &std::path::Path,
    name: &str,
) -> std::io::Result<(PathBuf, tokio::fs::File)> {
    create_unique_with(dir, name, NUMBERED_NAMES).await
}

pub(super) async fn create_unique_with(
    dir: &std::path::Path,
    name: &str,
    numbered: u32,
) -> std::io::Result<(PathBuf, tokio::fs::File)> {
    let (stem, ext) = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem.to_string(), format!(".{ext}")),
        _ => (name.to_string(), String::new()),
    };
    for index in 0..numbered + RANDOM_NAMES {
        let candidate = if index == 0 {
            dir.join(name)
        } else if index < numbered {
            dir.join(format!("{stem}-{index}{ext}"))
        } else {
            dir.join(format!(
                "{stem}-{}{ext}",
                &podnet_core::util::random_token()[..8]
            ))
        };
        match tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&candidate)
            .await
        {
            Ok(file) => return Ok((candidate, file)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::other("no free file name"))
}
