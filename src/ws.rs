//! WebSocket-carried raw streams.
//!
//! A quick tunnel is an HTTP tunnel, but it proxies WebSocket upgrades. That is
//! enough to carry *anything*: the origin exposes a WebSocket endpoint, each
//! connection is bridged to a raw stream (TCP or unix), and a client on the far
//! side turns local sockets into WebSocket connections. This is the websocat /
//! wstunnel idea, and it removes the HTTP-only limit without needing the edge to
//! understand the payload.
//!
//! One WebSocket message maps to one write, so binary framing is preserved and
//! interactive protocols (SSH, RDP, databases, raw sockets) work unchanged.
//!
//! The WebSocket, its bridge and its client are `pod-ws` of
//! PodUtils/podcrates, over podnet's dial, its TLS context and its user
//! agent ([`way`]), with the limits of tokio-tungstenite. The relay client
//! (`ws_relay.rs`) keeps a bridge of tokio-tungstenite ([`bridge`]) until
//! `pod-relay`.

use crate::cloudflare::stream as cqstream;
use anyhow::{bail, Context, Result};
use futures::{SinkExt as _, StreamExt as _};
use pod_ws::{ConnectError, Limits};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;
use tokio_util::compat::{FuturesAsyncReadCompatExt, FuturesAsyncWriteCompatExt};

use crate::ingress::{ForwardTarget, OriginOptions, Service};
use crate::metrics::Metrics;
use crate::net::BoxedIo;
use crate::proxy::{header_value, write_response_meta, EdgeReader, EdgeWriter};

/// The client WebSocket of podnet, as `pod-ws` gives it.
pub type Session = pod_ws::RelaySession<pod_ws::url::Stream>;

/// Default path for a `--forward` WebSocket endpoint.
pub const DEFAULT_PATH: &str = "/__podnet/ws";

/// The path of the first `--forward` rule of `ingress` (issue 203).
/// `None` also when the `path:` of the rule is a regular expression with a
/// special character: it names no one URL (issue 269).
pub fn forward_path(ingress: &crate::ingress::Ingress) -> Option<String> {
    let rule = ingress
        .rules
        .iter()
        .find(|rule| matches!(rule.service, Service::Forward(_)))?;
    match &rule.path {
        None => Some(DEFAULT_PATH.to_string()),
        Some(pattern) => pattern.literal_path(),
    }
}

/// The wss:// URL of the forward endpoint at `path` of the public URL `url`.
pub fn forward_url(url: &str, path: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://")
        .map(|r| ("wss", r))
        .or_else(|| url.strip_prefix("http://").map(|r| ("ws", r)))?;
    let host = rest.1.split('/').next()?;
    let path = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    };
    Some(format!("{}://{host}{path}", rest.0))
}

/// Compute `Sec-WebSocket-Accept` for a client's `Sec-WebSocket-Key`.
pub use pod_ws::handshake::accept_key;

/// The way out of a WebSocket of podnet: its dial (the egress layer:
/// `--proxy`, NO_PROXY, `--resolve`, the system resolver and DoH), its TLS
/// context (`--ca-file`, SSL_CERT_FILE) and its user agent.
pub fn way() -> pod_ws::Way {
    pod_ws::Way::new(
        crate::trust::context().clone(),
        crate::egress::egress(None),
        format!("podnet/{}", env!("CARGO_PKG_VERSION")),
    )
}

/// The byte counts of a bridge, in podnet's metrics.
struct Counted<'a>(&'a Metrics);

impl pod_ws::Tally for Counted<'_> {
    fn add_in(&self, bytes: u64) {
        self.0.add_in(bytes);
    }
    fn add_out(&self, bytes: u64) {
        self.0.add_out(bytes);
    }
}

/// Serve one inbound WebSocket upgrade from the edge, bridging it to `target`.
pub async fn serve_websocket(
    target: &ForwardTarget,
    request: &cqstream::ConnectRequest,
    reader: EdgeReader,
    mut writer: EdgeWriter,
    metrics: &Metrics,
) -> Result<()> {
    let Some(key) = header_value(request, "sec-websocket-key") else {
        return write_response_meta(&mut writer, 400, &plain("missing Sec-WebSocket-Key")).await;
    };

    // Dial first: the visitor gets 101 only for a target that answers
    // (issue 273), else 502.
    let service = match target {
        ForwardTarget::Tcp(addr) => Service::Tcp(addr.clone()),
        ForwardTarget::Unix(path) => Service::Unix(path.clone()),
    };
    let io = match crate::origin::dial::dial(&service, &OriginOptions::default()).await {
        Ok(io) => io,
        Err(err) => {
            const TEXT: &str = "podnet: the forward target does not answer\n";
            write_response_meta(&mut writer, 502, &plain(TEXT)).await?;
            futures::io::AsyncWriteExt::write_all(&mut writer, TEXT.as_bytes()).await?;
            futures::io::AsyncWriteExt::close(&mut writer).await?;
            return Err(err)
                .with_context(|| format!("dialing forward target {}", target.describe()));
        }
    };

    let headers = vec![
        ("Upgrade".to_string(), "websocket".to_string()),
        ("Connection".to_string(), "Upgrade".to_string()),
        ("Sec-WebSocket-Accept".to_string(), accept_key(key)),
    ];
    write_response_meta(&mut writer, 101, &headers).await?;

    // The edge stream is a QUIC bidi pair; join it into one socket for the
    // WebSocket codec. `into_inner` recovers the noq halves from the
    // futures-io wrappers used by the request framing.
    let socket = tokio::io::join(reader.compat(), writer.compat_write());
    let session =
        pod_ws::server::session(socket, Vec::new(), None).with_limits(Limits::TUNGSTENITE);
    pod_ws::bridge(session, io, &Counted(metrics))
        .await
        .context("the bridge of the forward WebSocket")
}

/// The time for each step (TCP, TLS, the upgrade) of a client WebSocket.
pub const DIAL_TIMEOUT: std::time::Duration = pod_ws::bridge::DIAL_TIMEOUT;

/// A client WebSocket to `url` (ws:// or wss://). The TCP goes through
/// the egress layer of podnet (issue 191): `--proxy`, NO_PROXY,
/// `--resolve`, the system resolver and DoH. TLS trusts the roots of
/// podnet (`--ca-file`, SSL_CERT_FILE). Issue 205: the credential of a
/// tunnel with --auth-user goes in `Authorization`. Issue 206: the time of
/// each step, at -v.
pub async fn dial(
    url: &str,
    timeout: std::time::Duration,
    authorization: Option<&str>,
) -> Result<Session> {
    let trust = pod_tls::Trust::Default;
    pod_ws::bridge::dial(url, &way(), &trust, timeout, authorization)
        .await
        .map_err(|err| failed(url, err))
}

/// A failure of [`dial`] in podnet's words, with its hop.
fn failed(url: &str, err: ConnectError) -> anyhow::Error {
    let host = pod_ws::url::WsUrl::parse(url)
        .map(|u| u.host)
        .unwrap_or_default();
    match err {
        ConnectError::Config(why) => anyhow::anyhow!("{url} is not a WebSocket URL: {why}"),
        ConnectError::Refused { status: 401, .. } => anyhow::anyhow!(
            "{url} refused the WebSocket upgrade: HTTP 401 (the tunnel needs --auth-user and --auth-password-file)"
        ),
        ConnectError::Refused { status, .. } => {
            anyhow::anyhow!("{url} refused the WebSocket upgrade: HTTP {status}")
        }
        ConnectError::Timeout { step, after } if step.contains("TLS") => crate::hop::at(
            crate::hop::Hop::Network,
            anyhow::anyhow!("TLS to {host}: no answer in {} s", after.as_secs()),
        ),
        ConnectError::Timeout { after, .. } => anyhow::anyhow!(
            "the WebSocket upgrade at {url}: no answer in {} s",
            after.as_secs()
        ),
        ConnectError::Tls(failure) => {
            let hop = if failure.cut.is_some() {
                crate::hop::Hop::Network
            } else {
                crate::hop::Hop::Tls
            };
            crate::hop::at(hop, anyhow::anyhow!("TLS to {host}: {}", failure.why))
        }
        ConnectError::Dial(dial) => {
            let failure = pod_core::Failure::from(dial);
            let hop = match failure.stage {
                pod_core::Stage::Proxy => Some(crate::hop::Hop::Proxy),
                pod_core::Stage::Dns => Some(crate::hop::Hop::Dns),
                _ => None,
            };
            let err = anyhow::Error::new(failure).context(format!("TCP to {host}"));
            match hop {
                Some(hop) => crate::hop::at(hop, err),
                None => err,
            }
        }
        other => anyhow::Error::new(other).context(format!("the WebSocket upgrade at {url}")),
    }
}

/// Connect a local socket to a remote WebSocket endpoint.
pub async fn connect_bridge(url: &str, io: BoxedIo, authorization: Option<&str>) -> Result<()> {
    let session = dial(url, DIAL_TIMEOUT, authorization).await?;
    bridge_session(session, io, &Metrics::new()).await
}

/// Pump bytes both ways between a WebSocket of [`dial`] and a raw stream.
pub async fn bridge_session(session: Session, io: BoxedIo, metrics: &Metrics) -> Result<()> {
    pod_ws::bridge(session, io, &Counted(metrics))
        .await
        .context("websocket bridge")
}

/// Pump bytes both ways between a WebSocket of tokio-tungstenite and a raw
/// stream: the relay client (`ws_relay.rs`) until `pod-relay`.
pub async fn bridge<S>(ws: WebSocketStream<S>, io: BoxedIo, metrics: &Metrics) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (mut sink, mut stream) = ws.split();
    let (mut io_r, mut io_w) = tokio::io::split(io);
    let (control_tx, mut control_rx) = tokio::sync::mpsc::channel::<Message>(16);
    let in_metrics = metrics.clone();
    let out_metrics = metrics.clone();

    let ws_to_io = async move {
        while let Some(message) = stream.next().await {
            let message = message.context("websocket read")?;
            match message {
                Message::Binary(bytes) => {
                    io_w.write_all(&bytes).await?;
                    in_metrics.add_in(bytes.len() as u64);
                }
                Message::Text(text) => {
                    io_w.write_all(text.as_bytes()).await?;
                    in_metrics.add_in(text.len() as u64);
                }
                Message::Ping(payload) => {
                    let _ = control_tx.send(Message::Pong(payload)).await;
                }
                // podssh #17: the code and the reason of the close of the
                // far end (a relay at its limit: 1009) are in the log.
                Message::Close(frame) => {
                    if let Some(frame) = frame {
                        let code = u16::from(frame.code);
                        if code != 1000 {
                            tracing::warn!(
                                code,
                                reason = %frame.reason,
                                "the far end closed the WebSocket{}",
                                pod_ws::close_hint(code)
                            );
                        } else {
                            tracing::debug!(code, reason = %frame.reason, "the WebSocket closed");
                        }
                    }
                    break;
                }
                _ => {}
            }
        }
        let _ = io_w.shutdown().await;
        Ok::<(), anyhow::Error>(())
    };

    let io_to_ws = async move {
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            tokio::select! {
                maybe = control_rx.recv() => {
                    match maybe {
                        Some(message) => {
                            if sink.send(message).await.is_err() {
                                break;
                            }
                        }
                        None => break,
                    }
                }
                read = io_r.read(&mut buf) => {
                    let n = read?;
                    if n == 0 {
                        break;
                    }
                    out_metrics.add_out(n as u64);
                    if sink
                        .send(Message::Binary(bytes::Bytes::copy_from_slice(&buf[..n])))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            }
        }
        let _ = sink.close().await;
        Ok::<(), anyhow::Error>(())
    };

    let (to_io, to_ws) = tokio::join!(ws_to_io, io_to_ws);
    to_io?;
    to_ws
}

fn plain(message: &str) -> Vec<(String, String)> {
    vec![
        (
            "Content-Type".to_string(),
            "text/plain; charset=utf-8".to_string(),
        ),
        ("Content-Length".to_string(), message.len().to_string()),
    ]
}

/// A local listener specification for `podnet connect`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LocalSpec {
    /// `tcp://[bind:]port`
    Tcp { bind: String, port: u16 },
    /// `unix:///path`
    Unix(std::path::PathBuf),
}

impl LocalSpec {
    pub fn parse(value: &str) -> Result<Self> {
        let value = value.trim();
        if let Some(rest) = value.strip_prefix("tcp://") {
            let (bind, port) = match rest.rsplit_once(':') {
                Some((bind, port)) => (bind.to_string(), port),
                None => ("127.0.0.1".to_string(), rest),
            };
            let port: u16 = port.parse().context("local TCP port is invalid")?;
            let bind = if bind.is_empty() {
                "127.0.0.1".to_string()
            } else {
                bind
            };
            return Ok(Self::Tcp { bind, port });
        }
        for prefix in ["unix://", "unix:"] {
            if let Some(rest) = value.strip_prefix(prefix) {
                if rest.is_empty() {
                    bail!("local unix socket has no path: {value:?}");
                }
                return Ok(Self::Unix(std::path::PathBuf::from(rest)));
            }
        }
        bail!("local listener must be tcp://[bind:]port or unix:///path, got {value:?}")
    }

    pub fn describe(&self) -> String {
        match self {
            Self::Tcp { bind, port } => format!("tcp://{bind}:{port}"),
            Self::Unix(path) => format!("unix:{}", path.display()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Issue 273: a forward target that does not answer gives 502, not
    /// 101 and then a closed WebSocket.
    #[tokio::test]
    async fn a_target_that_does_not_answer_gives_502() {
        crate::test_host::need_loopback!();
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = closed.local_addr().unwrap();
        drop(closed);
        let request = cqstream::ConnectRequest {
            dest: "https://x/__podnet/ws".into(),
            conn_type: cqstream::ConnectionType::Websocket,
            metadata: vec![(
                "HttpHeader:Sec-Websocket-Key".into(),
                "dGhlIHNhbXBsZSBub25jZQ==".into(),
            )],
        };
        let (near, far) = tokio::io::duplex(1 << 16);
        let reader: EdgeReader = Box::pin(futures::io::empty());
        let writer: EdgeWriter = Box::pin(
            tokio_util::compat::TokioAsyncWriteCompatExt::compat_write(near),
        );
        let target = ForwardTarget::Tcp(addr.to_string());
        let result = serve_websocket(&target, &request, reader, writer, &Metrics::new()).await;
        assert!(result.is_err());
        let mut far = tokio_util::compat::TokioAsyncReadCompatExt::compat(far);
        let (_, meta) = cqstream::read_connect_response(&mut far).await.unwrap();
        assert!(
            meta.contains(&("HttpStatus".to_string(), "502".to_string())),
            "{meta:?}"
        );
        assert!(
            !meta.iter().any(|(k, _)| k.contains("Sec-WebSocket-Accept")),
            "{meta:?}"
        );
    }

    #[test]
    fn parses_local_specs() {
        assert_eq!(
            LocalSpec::parse("tcp://8080").unwrap(),
            LocalSpec::Tcp {
                bind: "127.0.0.1".into(),
                port: 8080
            }
        );
        assert_eq!(
            LocalSpec::parse("tcp://0.0.0.0:9000").unwrap(),
            LocalSpec::Tcp {
                bind: "0.0.0.0".into(),
                port: 9000
            }
        );
        assert_eq!(
            LocalSpec::parse("unix:///tmp/f.sock").unwrap(),
            LocalSpec::Unix("/tmp/f.sock".into())
        );
        assert!(LocalSpec::parse("http://x").is_err());
        assert!(LocalSpec::parse("tcp://nope").is_err());
    }
}

#[cfg(test)]
mod forward_tests {
    use super::*;

    // Issue 203: the forward endpoint of a public URL.
    #[test]
    fn the_forward_url_of_a_public_url() {
        assert_eq!(
            forward_url("https://a-b.trycloudflare.com", DEFAULT_PATH).as_deref(),
            Some("wss://a-b.trycloudflare.com/__podnet/ws")
        );
        assert_eq!(
            forward_url("https://h.example/x/", "raw").as_deref(),
            Some("wss://h.example/raw")
        );
        assert_eq!(forward_url("ftp://h", "/x"), None);
        let ingress = crate::ingress::Ingress::new().rule_with(crate::ingress::IngressRule {
            hostname: None,
            path: Some(crate::ingress::PathPattern::prefix(DEFAULT_PATH)),
            service: Service::Forward(ForwardTarget::Tcp("127.0.0.1:22".into())),
            origin: OriginOptions::default(),
        });
        assert_eq!(forward_path(&ingress).as_deref(), Some(DEFAULT_PATH));
        assert_eq!(forward_path(&crate::ingress::Ingress::new()), None);
    }
}
