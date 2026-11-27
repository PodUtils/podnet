//! Fast `front=` health check for fronted roads (T-368).
//!
//! A dead or misrouted `front=` must produce an immediate refusal naming the front
//! within 1 second rather than hanging for a long transport timeout.

use std::fmt;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

use super::irc::split_host_port;

/// Hard upper bound for a `front=` health check.
pub const MAX_FRONT_CHECK_TIMEOUT: Duration = Duration::from_secs(1);

/// Refusal returned when a fronted road `front=` host cannot route to the target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrontCheckError {
    pub front: String,
    pub target: String,
    pub reason: String,
}

impl fmt::Display for FrontCheckError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "front {} refused routing to {}: {}",
            self.front, self.target, self.reason
        )
    }
}

impl std::error::Error for FrontCheckError {}

/// Extract a `front=HOST[:PORT]` query parameter from a road URL or spec.
pub fn parse_front_param(spec: &str) -> Option<String> {
    let (_, query) = spec.split_once('?')?;
    for pair in query.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            if k == "front" && !v.trim().is_empty() {
                return Some(v.trim().to_string());
            }
        }
    }
    None
}

/// Probe `front_host` for routing to `target_host` within `timeout` (clamped to <= 1 s).
///
/// Refused connections, HTTP 4xx/5xx responses, wrong routing headers, invalid status
/// lines, and timeouts all return an immediate [`FrontCheckError`] naming `front_host`.
pub async fn check_front(
    front_host: &str,
    target_host: &str,
    timeout: Duration,
) -> Result<(), FrontCheckError> {
    let front_label = front_host.trim().to_string();
    let target_label = target_host.trim().to_string();
    let err = |reason: String| FrontCheckError {
        front: front_label.clone(),
        target: target_label.clone(),
        reason,
    };

    if front_label.is_empty() {
        return Err(err("empty front host".to_string()));
    }
    if target_label.is_empty() {
        return Err(err("empty target host".to_string()));
    }
    if target_label.contains(['\r', '\n', ' ']) {
        return Err(err("invalid target host".to_string()));
    }

    let stripped = front_label
        .strip_prefix("https://")
        .or_else(|| front_label.strip_prefix("http://"))
        .unwrap_or(&front_label);
    let authority = stripped.split(['/', '?']).next().unwrap_or("");
    let default_port = if front_label.starts_with("https://") {
        443
    } else {
        80
    };
    let (host, port) = split_host_port(authority, default_port).map_err(|e| err(e.to_string()))?;
    let addr = if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };

    let bound = timeout.min(MAX_FRONT_CHECK_TIMEOUT);
    match tokio::time::timeout(bound, probe_http_front(&addr, &host, port, &target_label)).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(reason)) => Err(err(reason)),
        Err(_) => Err(err(format!("timed out after {} ms", bound.as_millis()))),
    }
}

async fn probe_http_front(
    addr: &str,
    host: &str,
    port: u16,
    target_host: &str,
) -> Result<(), String> {
    crate::policy::permit(host, port).map_err(|r| r.reason)?;
    let mut stream = TcpStream::connect(addr)
        .await
        .map_err(|e| format!("connect failed: {e}"))?;
    let req = format!("HEAD / HTTP/1.1\r\nHost: {target_host}\r\nConnection: close\r\n\r\n");
    stream
        .write_all(req.as_bytes())
        .await
        .map_err(|e| format!("write failed: {e}"))?;
    stream
        .flush()
        .await
        .map_err(|e| format!("flush failed: {e}"))?;

    let mut reader = BufReader::new(stream);
    let mut status_line = String::new();
    let n = reader
        .read_line(&mut status_line)
        .await
        .map_err(|e| format!("read failed: {e}"))?;
    if n == 0 {
        return Err("closed connection before status line".to_string());
    }
    let code = parse_http_status(&status_line)?;
    if !(200..400).contains(&code) {
        return Err(format!("HTTP {code}"));
    }

    // Check response headers for explicit misrouting indicators.
    let mut header_line = String::new();
    for _ in 0..64 {
        header_line.clear();
        let read = reader
            .read_line(&mut header_line)
            .await
            .map_err(|e| format!("read header failed: {e}"))?;
        if read == 0 || header_line == "\r\n" || header_line == "\n" {
            break;
        }
        if let Some((name, value)) = header_line.split_once(':') {
            let name = name.trim();
            let value = value.trim();
            if name.eq_ignore_ascii_case("x-front-routed-host")
                && !value.eq_ignore_ascii_case(target_host)
            {
                return Err(format!("wrong route to {value}"));
            }
        }
    }
    Ok(())
}

fn parse_http_status(line: &str) -> Result<u16, String> {
    let mut parts = line.split_whitespace();
    let proto = parts
        .next()
        .ok_or_else(|| "missing HTTP status line".to_string())?;
    if !proto.starts_with("HTTP/") {
        return Err(format!("non-HTTP response {proto:?}"));
    }
    let code_str = parts
        .next()
        .ok_or_else(|| "missing HTTP status code".to_string())?;
    code_str
        .parse::<u16>()
        .map_err(|_| format!("invalid HTTP status code {code_str:?}"))
}
