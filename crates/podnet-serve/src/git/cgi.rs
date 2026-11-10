//! A CGI/1.1 runner (RFC 3875) for a program that answers one HTTP
//! request, as `git http-backend` does (T-092).
//!
//! * The program gets a clean environment: the variables of [`KEPT`] of
//!   this process, the meta-variables of the request and the variables of
//!   the caller ([`Cgi::env`]).
//! * `CONTENT_LENGTH` is set only when the request has a
//!   `Content-Length`. A chunked body has no length: the program reads
//!   its stdin to the end.
//! * Each header of the request becomes `HTTP_NAME`, except the headers
//!   of [`HIDDEN`]: the program does not get the credentials.
//! * The body of the request streams to the stdin of the program, and
//!   its stdout streams to the response after the CGI head. The head
//!   (`Status:` and the other header lines) has at most [`MAX_HEAD`]
//!   bytes.

use std::ffi::OsString;
use std::process::Stdio;

use axum::body::Body;
use axum::response::{IntoResponse, Response};
use futures::StreamExt as _;
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _};

/// The variables of this process that the program gets.
pub const KEPT: &[&str] = &["PATH", "HOME", "LANG", "LC_ALL", "TZ", "TMPDIR"];

/// The headers that do not become a meta-variable.
pub const HIDDEN: &[&str] = &[
    "authorization",
    "proxy-authorization",
    "cookie",
    "content-type",
    "content-length",
    "connection",
    "transfer-encoding",
];

/// The most bytes of the CGI head.
pub const MAX_HEAD: usize = 64 * 1024;

/// The program and the variables of one request.
#[derive(Clone, Debug)]
pub struct Cgi {
    /// The program.
    pub program: OsString,
    /// The arguments of the program.
    pub args: Vec<OsString>,
    /// `PATH_INFO`.
    pub path_info: String,
    /// `REMOTE_USER`: only after a good authentication.
    pub remote_user: Option<String>,
    /// The other variables of the program.
    pub env: Vec<(OsString, OsString)>,
}

/// The name of the meta-variable of a header: `HTTP_` and the name in
/// capitals, with `_` for `-`.
pub fn header_variable(name: &str) -> String {
    let mut out = String::from("HTTP_");
    for c in name.chars() {
        out.push(if c == '-' {
            '_'
        } else {
            c.to_ascii_uppercase()
        });
    }
    out
}

/// The meta-variables of `request` (RFC 3875, section 4.1), with no
/// `PATH_INFO` and no `REMOTE_USER`.
pub fn meta_variables(request: &axum::extract::Request) -> Vec<(OsString, OsString)> {
    use std::os::unix::ffi::OsStringExt as _;
    let mut env: Vec<(OsString, OsString)> = Vec::new();
    let mut put = |name: &str, value: OsString| env.push((OsString::from(name), value));
    put("GATEWAY_INTERFACE", "CGI/1.1".into());
    put("SERVER_PROTOCOL", "HTTP/1.1".into());
    put(
        "SERVER_SOFTWARE",
        concat!("podnet/", env!("CARGO_PKG_VERSION")).into(),
    );
    put("REQUEST_METHOD", request.method().as_str().into());
    put(
        "QUERY_STRING",
        request.uri().query().unwrap_or_default().into(),
    );
    // No address with a unix socket and through the tunnel.
    let remote = request
        .extensions()
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .map(|info| info.0.ip().to_string())
        .unwrap_or_else(|| "0.0.0.0".to_string());
    put("REMOTE_ADDR", remote.into());
    let headers = request.headers();
    if let Some(value) = headers.get(http::header::CONTENT_TYPE) {
        put(
            "CONTENT_TYPE",
            OsString::from_vec(value.as_bytes().to_vec()),
        );
    }
    let chunked = headers
        .get(http::header::TRANSFER_ENCODING)
        .is_some_and(|v| v.as_bytes().eq_ignore_ascii_case(b"chunked"));
    if let Some(value) = headers
        .get(http::header::CONTENT_LENGTH)
        .filter(|_| !chunked)
    {
        put(
            "CONTENT_LENGTH",
            OsString::from_vec(value.as_bytes().to_vec()),
        );
    }
    let mut seen: Vec<String> = Vec::new();
    for name in headers.keys() {
        let name = name.as_str();
        if HIDDEN.contains(&name) || seen.iter().any(|s| s == name) {
            continue;
        }
        seen.push(name.to_string());
        // Two lines of one header become one value, with ", ".
        let joined: Vec<&[u8]> = headers.get_all(name).iter().map(|v| v.as_bytes()).collect();
        put(
            &header_variable(name),
            OsString::from_vec(joined.join(&b", "[..])),
        );
    }
    env
}

/// 502 with `message`.
fn bad_gateway(message: &str) -> Response {
    tracing::warn!(reason = message, "cgi: no answer");
    (http::StatusCode::BAD_GATEWAY, message.to_string()).into_response()
}

/// Run `cgi` for `request` and give its answer. A program that does not
/// start, or that gives no complete head, gives 502.
pub async fn run(cgi: Cgi, request: axum::extract::Request) -> Response {
    let mut command = tokio::process::Command::new(&cgi.program);
    command.args(&cgi.args).env_clear();
    for name in KEPT {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    command.envs(meta_variables(&request));
    command.env("PATH_INFO", &cgi.path_info);
    if let Some(user) = &cgi.remote_user {
        command.env("REMOTE_USER", user);
        command.env("AUTH_TYPE", "Basic");
    }
    command.envs(cgi.env.iter().map(|(k, v)| (k, v)));
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(err) => {
            return bad_gateway(&format!(
                "{} does not start: {err}",
                cgi.program.to_string_lossy()
            ))
        }
    };
    let (Some(mut stdin), Some(mut stdout), Some(stderr)) =
        (child.stdin.take(), child.stdout.take(), child.stderr.take())
    else {
        return bad_gateway("the program has no pipes");
    };
    let mut body = request.into_body().into_data_stream();
    tokio::spawn(async move {
        while let Some(chunk) = body.next().await {
            let Ok(chunk) = chunk else { break };
            if stdin.write_all(&chunk).await.is_err() {
                break;
            }
        }
        // The drop closes stdin: the program reads its end.
    });
    let program = cgi.program.to_string_lossy().into_owned();
    tokio::spawn(log_stderr(program.clone(), stderr));
    tokio::spawn(async move {
        match child.wait().await {
            Ok(status) if !status.success() => {
                tracing::info!(program, %status, "cgi: the program ended with an error")
            }
            Err(err) => tracing::info!(program, error = %err, "cgi: no exit status"),
            Ok(_) => {}
        }
    });
    let mut head = Vec::new();
    let end = loop {
        if let Some(end) = head_end(&head) {
            break end;
        }
        if head.len() >= MAX_HEAD {
            return bad_gateway("the CGI head is too large");
        }
        let mut buf = [0u8; 8192];
        match stdout.read(&mut buf).await {
            Ok(0) => return bad_gateway("the program ended before the CGI head"),
            Ok(n) => head.extend_from_slice(&buf[..n]),
            Err(err) => return bad_gateway(&format!("reading the CGI head: {err}")),
        }
    };
    let rest = bytes::Bytes::copy_from_slice(&head[end.1..]);
    let mut response = match parse_head(&head[..end.0]) {
        Ok(response) => response,
        Err(err) => return bad_gateway(&err),
    };
    let first = futures::stream::once(async move { Ok::<_, std::io::Error>(rest) });
    let stream = first.chain(tokio_util::io::ReaderStream::new(stdout));
    *response.body_mut() = Body::from_stream(stream);
    response
}

/// The end of the head in `buf`: the index of the blank line, and the
/// index of the first byte of the body.
fn head_end(buf: &[u8]) -> Option<(usize, usize)> {
    let mut i = 0;
    while let Some(at) = buf[i..].iter().position(|&b| b == b'\n') {
        let line_end = i + at;
        let next = line_end + 1;
        let rest = &buf[next..];
        if rest.starts_with(b"\r\n") {
            return Some((next, next + 2));
        }
        if rest.starts_with(b"\n") {
            return Some((next, next + 1));
        }
        i = next;
    }
    None
}

/// The response of a CGI head: `Status: CODE REASON` gives the status
/// (default 200; 302 with `Location` and no `Status`). Each other line is
/// a header.
pub fn parse_head(head: &[u8]) -> Result<Response, String> {
    let text = std::str::from_utf8(head).map_err(|_| "the CGI head is not UTF-8".to_string())?;
    let mut response = Response::new(Body::empty());
    let mut status = None;
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| format!("a bad line in the CGI head: {line:?}"))?;
        let value = value.trim();
        if name.eq_ignore_ascii_case("status") {
            let code = value.split(' ').next().unwrap_or_default();
            let code = code
                .parse::<u16>()
                .ok()
                .and_then(|c| http::StatusCode::from_u16(c).ok())
                .ok_or_else(|| format!("a bad Status in the CGI head: {value:?}"))?;
            status = Some(code);
            continue;
        }
        let name = http::HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| format!("a bad header name in the CGI head: {name:?}"))?;
        let value = http::HeaderValue::from_str(value)
            .map_err(|_| format!("a bad header value in the CGI head: {value:?}"))?;
        if name == http::header::LOCATION && status.is_none() {
            status = Some(http::StatusCode::FOUND);
        }
        response.headers_mut().append(name, value);
    }
    *response.status_mut() = status.unwrap_or(http::StatusCode::OK);
    Ok(response)
}

/// Log each line of the stderr of the program, at most 64 lines.
async fn log_stderr(program: String, stderr: tokio::process::ChildStderr) {
    let mut lines = tokio::io::BufReader::new(stderr).lines();
    let mut count = 0;
    while let Ok(Some(line)) = lines.next_line().await {
        if count < 64 {
            tracing::info!(program, line, "cgi: stderr");
        }
        count += 1;
    }
}
