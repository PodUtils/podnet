//! `podnet tunnel inspect replay ID`: send a stored request to its origin
//! again, through `forward_http` of pod-cloudflare with a sink of its own.
//! The ring gets a new exchange that names the first (`replayed_from`).
//!
//! * Only a complete exchange of an HTTP origin (`http`, `https`, `unix`,
//!   `unix+tls`) with its whole request body is sent. An open or incomplete
//!   exchange, a cut request body and an upgrade (a WebSocket) are refused
//!   (exit 65).
//! * A redacted header or query parameter is left out of the request, and
//!   the answer names it (`left_out`).
//! * The replay uses new counters: it is not a visitor request in the
//!   metrics of the tunnel.
//! * The capture gives the head that went to the origin, not the ingress
//!   rule: the rule is the first one whose path matches the target and
//!   whose `httpHostHeader` or hostname matches the `Host` of the head.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use pod_cloudflare::cloudflare::stream::{
    ConnectRequest, ConnectionType, HTTP_HEADER_KEY, HTTP_HOST_KEY, HTTP_METHOD_KEY,
};
use pod_cloudflare::origin::Origins;
use pod_cloudflare::proxy::{forward_http, EdgeReader, EdgeWriter};
use serde_json::{json, Value};

use super::{header, Exchange, Inspector, State, REDACTED};
use crate::exit::{self, Fault};
use crate::ingress::{Ingress, IngressRule, Service};

/// The time limit of one replay.
const REPLAY_TIME: Duration = Duration::from_secs(30);

/// The headers that the replay does not copy: the framing and the hop.
/// `forward_http` writes `Host` and `Connection` itself.
const NOT_COPIED: &[&str] = &[
    "host",
    "content-length",
    "transfer-encoding",
    "connection",
    "keep-alive",
    "te",
    "trailer",
    "upgrade",
    "proxy-connection",
];

/// Why the exchange cannot be sent again, or `None`.
pub(super) fn refusal(exchange: &Exchange) -> Option<String> {
    let id = exchange.id;
    match exchange.state {
        State::Open => return Some(format!("exchange {id} is still open")),
        State::Incomplete => return Some(format!("exchange {id} is incomplete")),
        State::Complete => {}
    }
    if exchange.upgrade() {
        return Some(format!("exchange {id} is an upgrade (a WebSocket)"));
    }
    if exchange.request.cut() > 0 {
        return Some(format!(
            "the request body of exchange {id} is cut ({} of {} bytes kept); start the tunnel with a larger --inspect-body",
            exchange.request.bytes.len(),
            exchange.request.total
        ));
    }
    None
}

/// The rule of the ingress for a stored head with `host` and `target`.
pub(super) fn pick_rule<'a>(
    ingress: &'a Ingress,
    host: &str,
    target: &str,
) -> Option<&'a IngressRule> {
    ingress.rules.iter().find(|rule| {
        let path = rule.path.as_ref().is_none_or(|p| p.is_match(target));
        let by_header = rule
            .origin
            .http_host_header
            .as_deref()
            .is_some_and(|h| h.eq_ignore_ascii_case(host));
        let by_name = || {
            Ingress::new()
                .rule_with((*rule).clone())
                .resolve(host, target)
                .is_some()
        };
        path && (by_header || by_name())
    })
}

/// The target with no redacted query parameter, and the names of the ones
/// that are left out.
pub(super) fn target_of(target: &str, left_out: &mut Vec<String>) -> String {
    let Some((path, query)) = target.split_once('?') else {
        return target.to_string();
    };
    let kept: Vec<&str> = query
        .split('&')
        .filter(|pair| match pair.split_once('=') {
            Some((name, value)) if value == REDACTED => {
                left_out.push(format!("{name} (query)"));
                false
            }
            _ => true,
        })
        .collect();
    if kept.is_empty() {
        path.to_string()
    } else {
        format!("{path}?{}", kept.join("&"))
    }
}

/// The metadata of the edge for the stored request: the method, the host,
/// and each header but the framing, the hop and the redacted ones. A body
/// gets its `Content-Length`.
pub(super) fn metadata(
    exchange: &Exchange,
    host: &str,
    left_out: &mut Vec<String>,
) -> Vec<(String, String)> {
    let mut out = vec![
        (HTTP_METHOD_KEY.to_string(), exchange.method.clone()),
        (HTTP_HOST_KEY.to_string(), host.to_string()),
    ];
    for (name, value) in &exchange.request_headers {
        let lower = name.to_ascii_lowercase();
        if NOT_COPIED.contains(&lower.as_str()) {
            continue;
        }
        let redacted = value == REDACTED
            && exchange
                .redacted
                .iter()
                .any(|n| n.eq_ignore_ascii_case(name));
        if redacted {
            left_out.push(name.clone());
            continue;
        }
        out.push((format!("{HTTP_HEADER_KEY}:{name}"), value.clone()));
    }
    let had_length = header(&exchange.request_headers, "content-length").is_some();
    if had_length || !exchange.request.bytes.is_empty() {
        out.push((
            format!("{HTTP_HEADER_KEY}:Content-Length"),
            exchange.request.bytes.len().to_string(),
        ));
    }
    out
}

/// Send the stored request `id` to its origin, and give the document of the
/// new exchange.
pub(super) async fn replay(inspector: &Arc<Inspector>, id: u64) -> Result<Value> {
    let exchange = inspector.get(id).ok_or_else(|| {
        exit::fault(
            Fault::NoInput,
            anyhow!(
                "the ring has no exchange {id} (it holds the last {} only)",
                inspector.options().keep
            ),
        )
    })?;
    if let Some(why) = refusal(&exchange) {
        return Err(exit::fault(Fault::DataError, anyhow!("{why}; no replay")));
    }
    let host_header = header(&exchange.request_headers, "host")
        .unwrap_or_default()
        .to_string();
    let mut left_out = Vec::new();
    let target = target_of(&exchange.target, &mut left_out);
    let rule = pick_rule(&inspector.ingress, &host_header, &target)
        .filter(|rule| matches!(rule.service, Service::Http(_) | Service::Unix(_) | Service::UnixTls(_)))
        .ok_or_else(|| {
            exit::fault(
                Fault::DataError,
                anyhow!("no HTTP origin of the ingress matches Host {host_header:?} and {target}; no replay"),
            )
        })?;
    let host = inspector.host().ok_or_else(|| {
        exit::fault(
            Fault::Unavailable,
            anyhow!("the tunnel has not started; no replay"),
        )
    })?;
    let request = ConnectRequest {
        dest: format!("http://{host_header}{target}"),
        conn_type: ConnectionType::Http,
        metadata: metadata(&exchange, &host_header, &mut left_out),
    };
    let sink = inspector.sink(Some(id));
    let origins = Origins::new(host.tls.clone())
        .with_report(host.report.clone())
        .with_words(host.settings.words.clone())
        .with_no_verify(host.origin_no_verify.clone())
        .with_capture(Some(sink.clone()));
    let reader: EdgeReader = Box::pin(futures::io::Cursor::new(exchange.request.bytes.clone()));
    let writer: EdgeWriter = Box::pin(futures::io::sink());
    let metrics = crate::metrics::Metrics::new();
    let sent = forward_http(
        &origins,
        &rule.service,
        &rule.origin,
        &request,
        reader,
        writer,
        &metrics,
    );
    let done = tokio::time::timeout(REPLAY_TIME, sent).await;
    drop(origins);
    let error = match done {
        Ok(Ok(())) => None,
        Ok(Err(err)) => Some(exit::fault(
            Fault::Unavailable,
            anyhow!("the origin: {}", err.chain()),
        )),
        Err(_) => Some(exit::fault(
            Fault::Timeout,
            anyhow!("the origin gave no full answer in 30 s"),
        )),
    };
    let new = sink.last().and_then(|new| inspector.get(new));
    drop(sink);
    let Some(mut new) = new else {
        return Err(error.unwrap_or_else(|| {
            exit::fault(Fault::Unavailable, anyhow!("the origin got no request"))
        }));
    };
    // The sink of this replay ends with the replay: an exchange with no end
    // is incomplete.
    if new.state == State::Open {
        new.state = State::Incomplete;
    }
    let mut doc = json!({
        "replayed_from": id,
        "exchange": new.detail(),
        "left_out": left_out,
    });
    if let Some(err) = error {
        doc["error"] = exit::text(&err).into();
    }
    Ok(doc)
}
