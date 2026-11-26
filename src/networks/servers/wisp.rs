//! A Wisp server as a network: `wisp:wss://HOST[:PORT]/PATH` (T-127).
//!
//! Many TCP streams go over one WebSocket to the server, by the Wisp
//! protocol of Mercury Workshop (versions 1 and 2). The server dials the
//! target of each stream. podnet has no code of this road: it is `wisp` of
//! `pod-egress`.
//!
//! * `wisp:wss://HOST[:PORT]/PATH` (TLS, default port 443),
//!   `wisp:ws://HOST[:PORT]/PATH` (no TLS, default port 80),
//!   `wisp:HOST[:PORT]/PATH` (the same as `wss://`), and the form of a
//!   forwarder template, `wisp+wss://` and `wisp+ws://`. The URL takes no
//!   user and no password. `wisp:PORT` is still the SOCKS5 hop of a host
//!   named `wisp`.
//! * The server takes no credential of this client: a server that requires
//!   a password or a key is refused at `up`.
//! * `up` opens the connection (the WebSocket and the opening of Wisp). A
//!   dial that finds the connection closed opens a new one, one time.

use std::error::Error as _;
use std::sync::{Arc, Mutex};

use pod_egress::wisp::{AtEnd, Connect, Road, Wisp, WispError};
use pod_tls::Trust;
use pod_ws::url::WsUrl;

use super::super::secret;
use super::super::usage;
use super::STEP;
use crate::network::{BoxFuture, Class, Connected, Dial, Failure, Link, Stage, Target, Way};

/// A server that a spec names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::networks) struct Spec {
    pub(in crate::networks) url: WsUrl,
}

impl Spec {
    /// `wisp:wss://HOST[:PORT]/PATH`, with no query.
    pub(in crate::networks) fn label(&self) -> String {
        format!("wisp:{}", self.url.shown())
    }

    /// The host that the network must reach first.
    pub(in crate::networks) fn target(&self) -> Target {
        Target::new(&self.url.host, self.url.port)
    }
}

/// The URL of a Wisp spec: the text after `wisp:`, or after `wisp+`.
fn url_text(spec: &str) -> Option<&str> {
    let lower = spec.get(..7).unwrap_or(spec).to_ascii_lowercase();
    if lower.starts_with("wisp+ws") {
        return Some(&spec[5..]);
    }
    spec.strip_prefix("wisp:")
}

/// True when `spec` is a Wisp spec. `wisp:PORT` is the SOCKS5 hop of a host
/// named `wisp`, as it was.
pub(super) fn is_wisp(spec: &str) -> bool {
    url_text(spec).is_some_and(|rest| rest.parse::<u16>().is_err())
}

/// Parse `spec`. The error names the spec with no path and no query.
pub(super) fn parse(spec: &str) -> Result<Spec, Failure> {
    let shown = secret::bare(spec);
    let rest = url_text(spec).unwrap_or(spec);
    if rest.is_empty() {
        return Err(usage(format!(
            "{shown}: give the server, wisp:wss://HOST[:PORT]/PATH"
        )));
    }
    let url = if rest.contains("://") {
        rest.to_string()
    } else {
        format!("wss://{}", rest.trim_start_matches("//"))
    };
    WsUrl::parse(&url)
        .map(|url| Spec { url })
        .map_err(|why| usage(format!("{shown}: {why}")))
}

/// The words of the server (its message of the day) go to the log.
fn note() -> pod_egress::Note {
    Arc::new(|event: pod_core::Event| match event.level {
        pod_core::Level::Info => tracing::info!(kind = event.kind, "{}", event.text),
        pod_core::Level::Warn => tracing::warn!(kind = event.kind, "{}", event.text),
    })
}

/// Bring the network up through `way`: the connection to the server and the
/// opening of Wisp prove its reach, its certificate and its protocol.
pub(super) async fn up(spec: &Spec, way: pod_ws::Way, trust: Trust) -> Result<Link, Failure> {
    let label = spec.label();
    let connect = Connect {
        way,
        trust,
        timeout: STEP,
        note: note(),
    };
    let held = Held {
        url: spec.url.clone(),
        connect,
        label: label.clone(),
        road: Mutex::new(None),
    };
    let road = held.road().await?;
    let opened = road.wisp.opened();
    let status = serde_json::json!({
        "scheme": "wisp",
        "spec": label,
        "tls": spec.url.tls,
        "version": format!("{}.{}", opened.version.0, opened.version.1),
        "confirms": road.wisp.confirms(),
    });
    Ok(Link::new(Some(Way::destination(Arc::new(held)))).with_status(status))
}

/// The connection of the network: one road of `pod-egress` at a time, and
/// a new one when it closed.
struct Held {
    url: WsUrl,
    connect: Connect,
    label: String,
    road: Mutex<Option<Arc<Road>>>,
}

impl Held {
    /// The road of the connection that is open, else of a new connection.
    /// A fault starts with the spec and names the hop.
    async fn road(&self) -> Result<Arc<Road>, Failure> {
        if let Some(road) = self.road.lock().unwrap_or_else(|e| e.into_inner()).clone() {
            return Ok(road);
        }
        let wisp = Wisp::connect(&self.url, &self.connect).await.map_err(|e| {
            let (stage, class) = e.stage_and_class();
            let text = format!("{}: the Wisp server: {e}", self.label);
            Failure::caused(stage, class, text, e)
        })?;
        let road = Arc::new(Road {
            wisp,
            // A pipe: the target may still answer after the end of input.
            at_end: AtEnd::AfterPeer,
            shown: self.url.shown(),
        });
        *self.road.lock().unwrap_or_else(|e| e.into_inner()) = Some(road.clone());
        Ok(road)
    }

    /// [`Held::road`] before `deadline`.
    async fn road_by(
        &self,
        target: &Target,
        deadline: tokio::time::Instant,
    ) -> Result<Arc<Road>, Failure> {
        tokio::time::timeout_at(deadline, self.road())
            .await
            .map_err(|_| {
                Failure::new(
                    Stage::WebSocket,
                    Class::Timeout,
                    format!(
                        "{}: the Wisp server: no connection in time for {target}",
                        self.label
                    ),
                )
            })?
    }

    /// Forget `road` when it is still the road of the network.
    fn forget(&self, road: &Arc<Road>) {
        let mut held = self.road.lock().unwrap_or_else(|e| e.into_inner());
        if held.as_ref().is_some_and(|r| Arc::ptr_eq(r, road)) {
            *held = None;
        }
    }
}

/// True when `failure` is the end of the connection, not of one stream: the
/// Close of the WebSocket, its failure, or the CLOSE of stream 0.
fn connection_ended(failure: &Failure) -> bool {
    let cause = failure.source().and_then(|e| e.downcast_ref::<WispError>());
    matches!(
        cause,
        Some(WispError::Gone { .. } | WispError::Failed(_) | WispError::Refused(_))
    )
}

impl Dial for Held {
    fn label(&self) -> String {
        self.label.clone()
    }

    fn proxy_label(&self, _target: &Target) -> Option<String> {
        Some(self.url.shown())
    }

    /// A stream of the connection. When the connection closed, the dial
    /// opens a new connection and tries the stream one more time.
    fn dial<'a>(
        &'a self,
        target: &'a Target,
        deadline: tokio::time::Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(async move {
            let road = self.road_by(target, deadline).await?;
            match pod_core::Dial::dial(road.as_ref(), target, deadline).await {
                Err(e) if connection_ended(&e) => {
                    self.forget(&road);
                    let road = self.road_by(target, deadline).await?;
                    pod_core::Dial::dial(road.as_ref(), target, deadline).await
                }
                other => other,
            }
        })
    }

    fn names_inside(&self) -> bool {
        true
    }
}
