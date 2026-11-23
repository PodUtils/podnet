//! An HTTP/3 server of a [`Shape`] on the QUIC stack
//! of `pod-quic` (PodUtils/podnet#486, QUIC-6).
//!
//! `pod-warp` keeps the HTTP/3 control stream open from `v0.82.0` on (RFC 9114,
//! section 6.2.1). This listener adds transport qlog output (`qlog_dir`) and
//! structured HTTP/3 trace events ([`Trace`]) for `podnet warp h3`, while
//! reusing the shape, SETTINGS, and wire framing of `pod-warp`: one HTTP/3
//! library.
//!
//! The listener takes a `pod-quic` endpoint with the TLS of the caller, so
//! it runs no second QUIC stack (QUIC-10).

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use pod_core::{Class, Failure, Stage};
use pod_quic::noq;
use pod_warp::h3::wire::{self, Frames, FRAME_DATA, FRAME_HEADERS, FRAME_SETTINGS};
use pod_warp::h3::Shape;
use rustls::pki_types::PrivateKeyDer;

use super::qlog::Trace;

/// The body of a GET of the shape.
pub const BODY: &[u8] = b"pod-warp h3 shape\n";
/// The most bytes of one read of a stream.
const CHUNK: usize = 64 * 1024;
/// The stream type of a control stream (RFC 9114, section 6.2.1).
pub const CONTROL: u64 = 0x00;

fn failure(class: Class, text: impl Into<String>) -> Failure {
    Failure::new(Stage::Warp, class, text)
}

/// The TLS of a listener of the shape: a self-signed CA and a leaf for
/// `name`, `127.0.0.1` and `::1`, with the ALPN `h3`. A caller that pins no
/// certificate of the shape (the stock client of the gate) verifies none;
/// the CA chain lets a caller that trusts the leaf verify it.
pub fn self_signed(name: &str) -> Result<Arc<noq::crypto::rustls::QuicServerConfig>, Failure> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let ca_key = rcgen::KeyPair::generate()
        .map_err(|e| failure(Class::Unavailable, format!("the CA key of the shape: {e}")))?;
    let mut params = rcgen::CertificateParams::new(Vec::<String>::new())
        .map_err(|e| failure(Class::Unavailable, format!("the CA of the shape: {e}")))?;
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "podnet h3 shape CA");
    let ca = params
        .self_signed(&ca_key)
        .map_err(|e| failure(Class::Unavailable, format!("the CA of the shape: {e}")))?;
    let issuer = rcgen::Issuer::new(params, ca_key);
    let leaf_key = rcgen::KeyPair::generate().map_err(|e| {
        failure(
            Class::Unavailable,
            format!("the leaf key of the shape: {e}"),
        )
    })?;
    let leaf = rcgen::CertificateParams::new(vec![
        name.to_string(),
        "127.0.0.1".to_string(),
        "::1".to_string(),
    ])
    .map_err(|e| failure(Class::Unavailable, format!("the leaf of the shape: {e}")))?
    .signed_by(&leaf_key, &issuer)
    .map_err(|e| failure(Class::Unavailable, format!("the leaf of the shape: {e}")))?;
    let mut tls = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| failure(Class::Unavailable, format!("TLS of the shape: {e}")))?
        .with_no_client_auth()
        .with_single_cert(
            vec![leaf.der().clone(), ca.der().clone()],
            PrivateKeyDer::Pkcs8(leaf_key.serialize_der().into()),
        )
        .map_err(|e| failure(Class::Config, format!("TLS of the shape: {e}")))?;
    tls.alpn_protocols = vec![b"h3".to_vec()];
    let crypto = noq::crypto::rustls::QuicServerConfig::try_from(Arc::new(tls))
        .map_err(|e| failure(Class::Config, format!("QUIC TLS of the shape: {e}")))?;
    Ok(Arc::new(crypto))
}

/// A listener of the shape: a QUIC endpoint with the ALPN `h3` and the
/// transport of the shape.
pub struct Listener {
    endpoint: noq::Endpoint,
    shape: Shape,
    trace: Option<Arc<Trace>>,
}

impl Listener {
    /// A listener on `addr` with the TLS configuration `crypto`, the
    /// `shape`, the timings of `base`, and an optional qlog trace. The
    /// trace also enables noq's transport qlog in `qlog_dir`, when it is
    /// given.
    pub fn bind(
        addr: SocketAddr,
        crypto: Arc<dyn noq::crypto::ServerConfig>,
        shape: Shape,
        base: &pod_quic::Settings,
        trace: Option<Arc<Trace>>,
        qlog_dir: Option<&Path>,
    ) -> Result<Listener, Failure> {
        let mut config = pod_quic::endpoint::server_config(crypto)?;
        let mut transport = shape.transport(base);
        if let Some(dir) = qlog_dir {
            transport.qlog_from_path(dir, "podnet-h3");
        }
        config.transport_config(Arc::new(transport));
        let endpoint = pod_quic::endpoint::server(config, addr)?;
        Ok(Listener {
            endpoint,
            shape,
            trace,
        })
    }

    /// The address of the listener.
    pub fn local_addr(&self) -> Result<SocketAddr, Failure> {
        self.endpoint.local_addr().map_err(|e| {
            Failure::caused(
                Stage::Quic,
                Class::Unavailable,
                format!("the listener: {e}"),
                e,
            )
        })
    }

    pub fn shape(&self) -> &Shape {
        &self.shape
    }

    pub fn trace(&self) -> Option<&Trace> {
        self.trace.as_deref()
    }

    /// Serve one connection until it closes: its control stream and each of
    /// its requests.
    pub async fn accept(&self) -> Result<(), Failure> {
        let Some(incoming) = self.endpoint.accept().await else {
            return Err(failure(Class::Refused, "the HTTP/3 listener ended"));
        };
        let conn = incoming.await.map_err(|e| {
            Failure::caused(
                Stage::Quic,
                Class::Protocol,
                format!("a connection at the HTTP/3 listener: {e}"),
                e,
            )
        })?;
        self.serve(conn).await
    }

    /// Serve each connection until the endpoint stops.
    pub async fn serve_forever(self: Arc<Self>) {
        while self.accept().await.is_ok() {}
    }

    async fn serve(&self, conn: noq::Connection) -> Result<(), Failure> {
        let alpn = conn
            .handshake_data()
            .and_then(|d| d.downcast::<noq::crypto::rustls::HandshakeData>().ok())
            .and_then(|d| d.protocol)
            .map(|p| String::from_utf8_lossy(&p).into_owned())
            .unwrap_or_default();
        if let Some(trace) = &self.trace {
            trace.event(
                "http3:connection_started",
                serde_json::json!({ "alpn": alpn }),
            );
        }
        // The control stream stays open for the life of the connection. A
        // FIN on it is `H3_CLOSED_CRITICAL_STREAM` to the peer.
        let control = self.control(&conn).await?;
        tracing::debug!(
            stream = control.id().index(),
            "the HTTP/3 control stream is open"
        );
        let uni = tokio::spawn(drain_uni(conn.clone()));
        let trace = self.trace.clone();
        let bi = tokio::spawn(requests(conn.clone(), trace));
        let _ = conn.closed().await;
        uni.abort();
        bi.abort();
        drop(control);
        if let Some(trace) = &self.trace {
            trace.event("http3:connection_closed", serde_json::json!({}));
        }
        Ok(())
    }

    /// The control stream of the server: its type, then SETTINGS, before any
    /// answer (RFC 9114, section 6.2.1).
    async fn control(&self, conn: &noq::Connection) -> Result<noq::SendStream, Failure> {
        let mut stream = conn.open_uni().await.map_err(|e| {
            failure(
                Class::Unavailable,
                format!("the HTTP/3 control stream: {e}"),
            )
        })?;
        let mut first = Vec::new();
        first.push(0x00);
        first.extend_from_slice(&pod_warp::h3::frame(
            FRAME_SETTINGS,
            &self.shape.settings_payload(),
        ));
        stream
            .write_all(&first)
            .await
            .map_err(|e| failure(Class::Unavailable, format!("the HTTP/3 SETTINGS: {e}")))?;
        if let Some(trace) = &self.trace {
            trace.event(
                "http3:control_stream",
                serde_json::json!({
                    "stream_id": u64::from(noq::VarInt::from(stream.id())),
                    "stream_type": "control",
                    "role": "server",
                    "settings": self.shape.settings.pairs(),
                }),
            );
        }
        Ok(stream)
    }
}

/// Answer each request stream.
async fn requests(conn: noq::Connection, trace: Option<Arc<Trace>>) {
    while let Ok((mut send, mut recv)) = conn.accept_bi().await {
        let trace = trace.clone();
        tokio::spawn(async move {
            let mut frames = Frames::default();
            let fields = loop {
                if let Ok(Some((id, payload))) = frames.next_frame() {
                    if id == FRAME_HEADERS {
                        break match wire::request_fields(&payload) {
                            Ok(fields) => fields,
                            Err(e) => {
                                let _ = send.finish();
                                tracing::debug!(error = %e.chain(), "an HTTP/3 request of the shape was refused");
                                return;
                            }
                        };
                    }
                    continue;
                }
                match recv.read_chunk(CHUNK).await {
                    Ok(Some(chunk)) => frames.push(&chunk),
                    _ => return,
                }
            };
            let method = field(&fields, ":method").unwrap_or_default();
            let path = field(&fields, ":path").unwrap_or_default();
            let status: u16 = match method.as_str() {
                "GET" | "HEAD" => 200,
                _ => 405,
            };
            if status == 200 {
                let _ = send
                    .write_all(&pod_warp::h3::frame(
                        FRAME_HEADERS,
                        &[0x00, 0x00, 0xc0 | 25],
                    ))
                    .await;
                if method == "GET" {
                    let _ = send.write_all(&pod_warp::h3::frame(FRAME_DATA, BODY)).await;
                }
            } else {
                let _ = send
                    .write_all(&pod_warp::h3::frame(FRAME_HEADERS, &status_405()))
                    .await;
            }
            let _ = send.finish();
            if let Some(trace) = trace {
                trace.event(
                    "http3:request",
                    serde_json::json!({ "method": method, "path": path, "status": status }),
                );
            }
        });
    }
}

/// Read the streams of the client (its control stream and its QPACK
/// streams) until the connection ends.
async fn drain_uni(conn: noq::Connection) {
    while let Ok(mut stream) = conn.accept_uni().await {
        tokio::spawn(async move { while let Ok(Some(_)) = stream.read_chunk(CHUNK).await {} });
    }
}

fn field(fields: &[(String, String)], name: &str) -> Option<String> {
    fields
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v.clone())
}

/// The field section of `:status 405`: two zero bytes (a required field
/// section and no base), a literal with a name reference to `:status`
/// (static 24, a four-bit prefix, so `0x5f 0x09`), and the value `405` with
/// no Huffman code (a seven-bit prefix, `0x03`).
fn status_405() -> Vec<u8> {
    let mut section = vec![0x00, 0x00, 0x5f, 0x09, 0x03];
    section.extend_from_slice(b"405");
    section
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The field section of the refusal decodes to `:status 405`.
    #[test]
    fn the_refusal_is_a_status_of_405() {
        assert_eq!(wire::response_status(&status_405()).unwrap(), 405);
    }

    /// The body of the shape is the one that a stock client reads.
    #[test]
    fn the_body_names_the_shape() {
        assert_eq!(BODY, b"pod-warp h3 shape\n");
    }
}
