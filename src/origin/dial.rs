//! The dial of a forward origin: TCP, TLS, unix sockets, and TLS over
//! unix.

use std::sync::{Arc, OnceLock};

use anyhow::{bail, Context, Result};
use tokio::io::{AsyncRead, AsyncWrite};
#[cfg(unix)]
use tokio::net::UnixStream;
use tokio_rustls::rustls::client::danger::{
    HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
};
use tokio_rustls::rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use tokio_rustls::rustls::{ClientConfig, DigitallySignedStruct, SignatureScheme};
use tokio_rustls::TlsConnector;

use crate::ingress::{OriginOptions, Service};
use crate::net::{connect_origin, init_crypto, parse_http_url, split_host_port, BoxedIo};

/// Dial the origin for a forward service.
pub async fn dial(service: &Service, options: &OriginOptions) -> Result<BoxedIo> {
    let timeout = options.connect_timeout;
    let connect = async {
        match service {
            Service::Http(url) => {
                let (host, port, tls) = parse_http_url(url)?;
                let tcp = connect_origin(&host, port).await?;
                if tls {
                    if options.no_tls_verify {
                        note_unverified(url);
                    }
                    tls_wrap(tcp, &host, options.no_tls_verify).await
                } else {
                    Ok(Box::new(tcp) as BoxedIo)
                }
            }
            Service::Unix(path) => dial_unix(path, false, options).await,
            Service::UnixTls(path) => dial_unix(path, true, options).await,
            Service::Tcp(addr) => {
                let (host, port) =
                    split_host_port(addr).with_context(|| format!("{addr} is not HOST:PORT"))?;
                let s = connect_origin(host, port).await?;
                let _ = s.set_nodelay(true);
                Ok(Box::new(s) as BoxedIo)
            }
            other => bail!("{} is not a forward service", other.describe()),
        }
    };
    match timeout {
        Some(limit) => tokio::time::timeout(limit, connect)
            .await
            .map_err(|_| anyhow::anyhow!("origin connect timed out after {limit:?}"))?,
        None => connect.await,
    }
}

/// Dial a filesystem socket, optionally wrapping it in TLS.
#[cfg(unix)]
async fn dial_unix(path: &std::path::Path, tls: bool, options: &OriginOptions) -> Result<BoxedIo> {
    let stream = UnixStream::connect(path)
        .await
        .with_context(|| format!("connecting to unix:{}", path.display()))?;
    if tls {
        if options.no_tls_verify {
            note_unverified(&format!("unix+tls:{}", path.display()));
        }
        tls_wrap(stream, "localhost", options.no_tls_verify).await
    } else {
        Ok(Box::new(stream) as BoxedIo)
    }
}

#[cfg(not(unix))]
async fn dial_unix(
    path: &std::path::Path,
    _tls: bool,
    _options: &OriginOptions,
) -> Result<BoxedIo> {
    bail!(
        "unix sockets are not supported on this platform: {}",
        path.display()
    )
}

/// One warning for each origin whose TLS certificate is not checked
/// (noTLSVerify, #248): the first dial of it says so; the later dials do
/// not (a warning for each request would fill the log). Gives true for
/// the first call of `origin`.
pub(crate) fn note_unverified(origin: &str) -> bool {
    static SEEN: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    if seen.iter().any(|o| o == origin) {
        return false;
    }
    if seen.len() < 256 {
        seen.push(origin.to_string());
    }
    tracing::warn!(%origin, "noTLSVerify: podnet does not check the TLS certificate of this origin");
    true
}

async fn tls_wrap<S>(stream: S, server_name: &str, no_verify: bool) -> Result<BoxedIo>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let connector = if no_verify {
        no_verify_connector()
    } else {
        verified_connector()
    };
    let name = ServerName::try_from(server_name.to_string())
        .map_err(|_| anyhow::anyhow!("invalid TLS server name {server_name:?}"))?;
    let tls = connector
        .connect(name, stream)
        .await
        .with_context(|| format!("TLS handshake with {server_name}"))?;
    Ok(Box::new(tls))
}

fn verified_connector() -> TlsConnector {
    static ONCE: OnceLock<TlsConnector> = OnceLock::new();
    ONCE.get_or_init(|| {
        init_crypto();
        let context = crate::trust::context();
        let config = context
            .client_config_with(&[], &[])
            .map(Arc::new)
            .or_else(|err| {
                tracing::warn!("{err:#}; using the compiled-in roots only");
                pod_tls::client_config(
                    &pod_tls::roots_from_compiled_set(),
                    context.provider().clone(),
                    context.groups().clone(),
                )
            })
            .expect("a configuration over the compiled-in roots");
        TlsConnector::from(config)
    })
    .clone()
}

fn no_verify_connector() -> TlsConnector {
    static ONCE: OnceLock<TlsConnector> = OnceLock::new();
    ONCE.get_or_init(|| {
        init_crypto();
        let provider = Arc::new(tokio_rustls::rustls::crypto::ring::default_provider());
        let config = ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoVerify(provider)))
            .with_no_client_auth();
        TlsConnector::from(Arc::new(config))
    })
    .clone()
}

#[derive(Debug)]
struct NoVerify(Arc<tokio_rustls::rustls::crypto::CryptoProvider>);

impl ServerCertVerifier for NoVerify {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, tokio_rustls::rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, tokio_rustls::rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, tokio_rustls::rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// Convenience: the origin path for a service, used by logs.
pub fn origin_label(service: &Service) -> String {
    match service {
        Service::Unix(path) => format!("unix:{}", path.display()),
        Service::UnixTls(path) => format!("unix+tls:{}", path.display()),
        other => other.describe(),
    }
}
