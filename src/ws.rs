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

use anyhow::{bail, Context, Result};
use futures::{SinkExt as _, StreamExt as _};
use pod_ws::ConnectError;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

use crate::metrics::Metrics;
use crate::net::BoxedIo;

/// The client WebSocket of podnet, as `pod-ws` gives it.
pub type Session = pod_ws::RelaySession<pod_ws::url::Stream>;

/// Compute `Sec-WebSocket-Accept` for a client's `Sec-WebSocket-Key`.
pub use pod_ws::handshake::accept_key;

/// The edge side of a forward: pod-cloudflare's.
pub use pod_cloudflare::ws::{forward_path, forward_url, serve_websocket, DEFAULT_PATH};

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
    let trust = crate::trust::ws_trust();
    pod_ws::bridge::dial(url, &way(), &trust, timeout, authorization)
        .await
        .map_err(|err| failed(url, err))
}

/// The lead of the error line of a failed [`dial`] of `remote`: a failure
/// of the proxy or of a name is on the way, so the line does not say that
/// the remote does not answer (GitHub #697).
pub fn remote_lead(remote: &str, err: &anyhow::Error) -> String {
    // GitHub #727: the policy refused before any connect; the remote was
    // not asked.
    if crate::policy::refusal_in(err).is_some() {
        return format!("the egress policy refuses the remote {remote}");
    }
    match crate::hop::of(err) {
        Some(crate::hop::Hop::Proxy | crate::hop::Hop::Dns) => format!("the remote {remote}"),
        _ => format!("the remote {remote} does not answer"),
    }
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
    let session = dial_again(url, authorization).await?;
    bridge_session(session, io, &Metrics::new()).await
}

/// The tries of a dial of [`connect_bridge`] (T-407).
const DIAL_TRIES: u32 = 3;

/// [`dial`], tried again after a failure that can pass (a reset, a
/// refusal of the edge with HTTP 5xx, a time limit): no byte of the local
/// stream went out yet, so a new dial loses nothing. HTTP 4xx is final.
async fn dial_again(url: &str, authorization: Option<&str>) -> Result<Session> {
    let mut tries = 0;
    loop {
        tries += 1;
        let err = match pod_ws::bridge::dial(
            url,
            &way(),
            &crate::trust::ws_trust(),
            DIAL_TIMEOUT,
            authorization,
        )
        .await
        {
            Ok(session) => return Ok(session),
            Err(err) => err,
        };
        let final_error = matches!(err, ConnectError::Config(_))
            || matches!(err, ConnectError::Refused { status, .. } if (400..500).contains(&status));
        if final_error || tries >= DIAL_TRIES {
            return Err(failed(url, err));
        }
        tracing::warn!(attempt = tries, error = %failed(url, err), "the WebSocket dial failed; podnet dials again");
        tokio::time::sleep(std::time::Duration::from_millis(250 * u64::from(tries))).await;
    }
}

/// Pump bytes both ways between a WebSocket of [`dial`] and a raw stream.
pub async fn bridge_session(session: Session, io: BoxedIo, metrics: &Metrics) -> Result<()> {
    pod_ws::bridge(session, io, metrics)
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

    /// T-407: a dial that the far end refuses once with HTTP 503 is tried
    /// again; HTTP 403 is final.
    #[tokio::test]
    async fn a_refused_upgrade_is_tried_again() {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        crate::test_host::need_loopback!();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let first_status = std::sync::Arc::new(std::sync::Mutex::new("503 Service Unavailable"));
        let status = first_status.clone();
        tokio::spawn(async move {
            // The first connection gets the status; each next one an upgrade.
            let (mut c, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 4096];
            let _ = c.read(&mut buf).await;
            let line = *status.lock().unwrap();
            let _ = c
                .write_all(format!("HTTP/1.1 {line}\r\ncontent-length: 0\r\n\r\n").as_bytes())
                .await;
            drop(c);
            loop {
                let (c, _) = listener.accept().await.unwrap();
                tokio::spawn(async move {
                    if let Ok(mut ws) = tokio_tungstenite::accept_async(c).await {
                        use futures::{SinkExt as _, StreamExt as _};
                        while let Some(Ok(m)) = ws.next().await {
                            if ws.send(m).await.is_err() {
                                break;
                            }
                        }
                    }
                });
            }
        });
        let url = format!("ws://{addr}/");
        assert!(dial_again(&url, None).await.is_ok(), "a 503 is tried again");
        *first_status.lock().unwrap() = "403 Forbidden";
        // A new server for the 403 case.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let accepted = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = accepted.clone();
        tokio::spawn(async move {
            loop {
                let (mut c, _) = listener.accept().await.unwrap();
                count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let mut buf = [0u8; 4096];
                let _ = c.read(&mut buf).await;
                let _ = c
                    .write_all(b"HTTP/1.1 403 Forbidden\r\ncontent-length: 0\r\n\r\n")
                    .await;
            }
        });
        let err = dial_again(&format!("ws://{addr}/"), None)
            .await
            .unwrap_err();
        assert!(format!("{err:#}").contains("403"), "{err:#}");
        assert_eq!(
            accepted.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "a 403 is final"
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
