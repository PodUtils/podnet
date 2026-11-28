//! A small HTTP/1.1 client for the requests of podnet: the quick-tunnel API,
//! DNS over HTTPS, the WARP API and the fetch of the public URL.
//!
//! It is the client of `pod-proxy` (PodUtils/podcrates) over the dial of
//! [`crate::egress`] (so each request takes the proxy, `NO_PROXY` and the
//! attempt delay of that module), with the roots of [`crate::trust`] for
//! `https://` (ALPN `http/1.1`) and podnet's user agent. Each request sends
//! `Connection: close`; the body of the answer is read by `Content-Length`,
//! by chunks, or to the end of the connection, up to [`MAX_BODY`] bytes, or
//! up to the limit of [`Request::max_body`]. [`send`] gives a failure
//! podnet's words and its hop.

use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::task::{Context as TaskContext, Poll};

use anyhow::{Context as _, Result};
use pod_core::{BoxFuture, Class, Connected, Dial, Failure, Stage, Target};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::time::Instant;

pub use pod_proxy::client::{Request, MAX_BODY};
pub use pod_proxy::http::Response;

use crate::egress::Refusal;
use crate::hop::{self, AtHop, Hop};

/// The client of the process.
fn client() -> &'static pod_proxy::Client {
    static CLIENT: OnceLock<pod_proxy::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        pod_proxy::Client::new(crate::trust::context().clone(), crate::egress::egress(None))
            .with_user_agent(format!("podnet/{}", env!("CARGO_PKG_VERSION")))
            .with_records(crate::egress::resolver())
    })
}

/// Send `request` and read the answer, the whole within its time limit.
pub async fn send(mut request: Request) -> Result<Response> {
    let what = format!("{} {}", request.method, without_query(&request.url));
    let timeout = request.timeout;
    if request.url.starts_with("https://")
        && request.trust.is_none()
        && crate::trust::context().ech().mode() != pod_tls::ech::Mode::Off
    {
        request.trust = Some(pod_tls::Trust::Default);
    }
    let watch = request.url.starts_with("https://").then(|| {
        let connected = Arc::new(AtomicBool::new(false));
        let spoke = Arc::new(AtomicBool::new(false));
        let inner = request
            .via
            .take()
            .unwrap_or_else(|| crate::egress::egress(None));
        request.via = Some(Arc::new(TlsWatchDial {
            inner,
            connected: connected.clone(),
            spoke: spoke.clone(),
        }));
        (connected, spoke)
    });
    client().send(request).await.map_err(|failure| {
        if let Some((connected, spoke)) = watch {
            if failure.stage == Stage::Connect
                && failure.class == Class::Timeout
                && connected.load(Ordering::Relaxed)
                && !spoke.load(Ordering::Relaxed)
            {
                return hop::at(
                    Hop::TlsTimeout,
                    anyhow::anyhow!(
                        "{what}: TLS handshake timed out after {} s",
                        timeout.as_secs()
                    ),
                );
            }
        }
        failed(failure, &what)
    })
}

/// Dial wrapper that tracks whether TCP connected and whether the peer sent
/// any byte, so a timeout during a silent TLS handshake is not misreported as
/// a TCP connect timeout (T-625).
struct TlsWatchDial {
    inner: Arc<dyn Dial>,
    connected: Arc<AtomicBool>,
    spoke: Arc<AtomicBool>,
}

impl Dial for TlsWatchDial {
    fn label(&self) -> String {
        self.inner.label()
    }

    fn proxy_label(&self, target: &Target) -> Option<String> {
        self.inner.proxy_label(target)
    }

    fn dial<'a>(
        &'a self,
        target: &'a Target,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(async move {
            let mut conn = self.inner.dial(target, deadline).await?;
            self.connected.store(true, Ordering::Relaxed);
            conn.io = Box::new(SpyIo {
                inner: conn.io,
                spoke: self.spoke.clone(),
            });
            Ok(conn)
        })
    }

    fn names_inside(&self) -> bool {
        self.inner.names_inside()
    }

    fn datagrams<'a>(&'a self) -> BoxFuture<'a, Result<Arc<dyn pod_core::Datagrams>, Failure>> {
        self.inner.datagrams()
    }
}

struct SpyIo {
    inner: pod_core::BoxedIo,
    spoke: Arc<AtomicBool>,
}

impl AsyncRead for SpyIo {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let poll = Pin::new(&mut self.inner).poll_read(cx, buf);
        if matches!(poll, Poll::Ready(Ok(()))) && buf.filled().len() > before {
            self.spoke.store(true, Ordering::Relaxed);
        }
        poll
    }
}

impl AsyncWrite for SpyIo {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

/// The JSON of the body of an answer.
pub fn json<T: serde::de::DeserializeOwned>(response: &Response) -> Result<T> {
    serde_json::from_slice(&response.body).context("decoding the JSON body")
}

fn without_query(url: &str) -> &str {
    url.split(['?', '#']).next().unwrap_or(url)
}

/// A failed request in podnet's words: the request and why, each cause
/// once. The dial's refusal and hop stay as [`crate::egress`] made them; a
/// handshake that the network ended is at the hop `network`, in the words of
/// [`hop::tls_text`], a silent TLS peer is at `tls-timeout`, and one that TLS
/// refused is at the hop `tls`.
pub(crate) fn failed(failure: Failure, what: &str) -> anyhow::Error {
    let (mut refusal, mut dialed, mut io) = (None, None, None);
    let mut next: Option<&(dyn std::error::Error + 'static)> = Some(&failure);
    while let Some(e) = next {
        refusal = refusal
            .or_else(|| e.downcast_ref::<Refusal>().cloned())
            .or_else(|| match e.downcast_ref::<pod_proxy::DialError>() {
                Some(pod_proxy::DialError::ProxyRefused {
                    proxy,
                    target,
                    status,
                    reason,
                }) => Some(Refusal::for_connect(proxy, target, *status, reason)),
                _ => None,
            });
        dialed = dialed.or_else(|| e.downcast_ref::<AtHop>().map(|h| h.hop));
        io = io.or_else(|| e.downcast_ref::<std::io::Error>());
        next = e.source();
    }
    let (text, hop) = match (failure.stage, io) {
        (Stage::Network, Some(io)) if dialed.is_none() => (
            format!("{what}: TLS: {}", hop::tls_text(io)),
            Some(hop::tls_hop(io)),
        ),
        _ => (
            failure.chain(),
            dialed.or_else(|| hop::of_failure(&failure)),
        ),
    };
    let err = match refusal {
        Some(refused) => anyhow::Error::new(Refusal {
            status: refused.status,
            text,
        }),
        None => anyhow::anyhow!(text),
    };
    match hop {
        Some(hop) => hop::at(hop, err),
        None => err,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Direct loopback dialer for tests that must not depend on process proxy state.
    struct DirectLoopback;

    impl Dial for DirectLoopback {
        fn label(&self) -> String {
            "direct-test".into()
        }

        fn dial<'a>(
            &'a self,
            target: &'a Target,
            _deadline: Instant,
        ) -> BoxFuture<'a, Result<Connected, Failure>> {
            Box::pin(async move {
                let addr = format!("{}:{}", target.host(), target.port());
                let stream = tokio::net::TcpStream::connect(&addr)
                    .await
                    .map_err(|e| Failure::io(Stage::Connect, format!("connecting to {addr}"), e))?;
                Ok(Connected::new(Box::new(stream)))
            })
        }
    }

    /// T-625 (GitHub #791): a silent TLS peer produces Hop::TlsTimeout and one
    /// consistent hint across connect, relay connect, and bench.
    #[tokio::test]
    async fn silent_tls_peer_maps_identically_for_connect_bench_and_relay_connect() {
        crate::init_crypto();
        crate::test_host::need_loopback!();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (_stream, _) = listener.accept().await.unwrap();
            tokio::time::sleep(Duration::from_secs(10)).await;
        });

        let mut req =
            Request::new("GET", format!("https://{addr}/")).timeout(Duration::from_secs(1));
        req.via = Some(Arc::new(DirectLoopback));
        let bench_err = send(req).await.unwrap_err();
        server.abort();

        let ws_timeout = pod_ws::ConnectError::Timeout {
            step: "the TLS handshake",
            after: Duration::from_secs(20),
        };
        let connect_err =
            crate::ws::failed(&format!("wss://{addr}/__podnet/ws"), ws_timeout.clone());
        let relay_err = crate::relay_reverse::connect_error("pair-a", ws_timeout);

        let cases = [
            ("connect", connect_err),
            ("bench", bench_err),
            ("relay connect", relay_err),
        ];
        let mut hints = Vec::new();
        for (name, err) in cases {
            assert_eq!(hop::of(&err), Some(Hop::TlsTimeout), "{name}: {err:#}");
            let line = crate::report::error_line(&err);
            let hint = line.lines().nth(1).unwrap_or_default().to_string();
            assert!(
                hint.contains("did not answer the TLS handshake") && hint.contains("speaks TLS"),
                "{name}: {line}"
            );
            for wrong in ["ended the connection", "--ca-file", "clock"] {
                assert!(!hint.contains(wrong), "{name}: {line}");
            }
            hints.push((name, hint));
        }
        assert_eq!(hints[0].1, hints[1].1, "{hints:?}");
        assert_eq!(hints[1].1, hints[2].1, "{hints:?}");
    }
}
