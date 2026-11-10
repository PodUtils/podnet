use super::*;

/// A verifier that keeps the chain of the server, then asks `inner` (or
/// accepts, with no `inner`).
#[derive(Debug)]
pub(super) struct Recorder {
    inner: Option<Arc<dyn ServerCertVerifier>>,
    seen: Mutex<Vec<CertificateDer<'static>>>,
    provider: Arc<rustls::crypto::CryptoProvider>,
}

impl ServerCertVerifier for Recorder {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if let Ok(mut seen) = self.seen.lock() {
            seen.push(end_entity.clone().into_owned());
            seen.extend(intermediates.iter().map(|c| c.clone().into_owned()));
        }
        match &self.inner {
            Some(inner) => {
                inner.verify_server_cert(end_entity, intermediates, server_name, ocsp, now)
            }
            None => Ok(ServerCertVerified::assertion()),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// A stream that counts the bytes it reads.
pub(super) struct Counting {
    inner: BoxedIo,
    read: Arc<AtomicU64>,
}

impl AsyncRead for Counting {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let poll = std::pin::Pin::new(&mut self.inner).poll_read(cx, buf);
        let n = buf.filled().len() - before;
        self.read.fetch_add(n as u64, Ordering::Relaxed);
        poll
    }
}

impl AsyncWrite for Counting {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        std::pin::Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

pub(super) fn issuer_of(der: &[u8]) -> Option<String> {
    let (_, cert) = x509_parser::parse_x509_certificate(der).ok()?;
    Some(cert.issuer().to_string())
}

pub(super) fn verifier(tls: Tls) -> Result<Option<Arc<dyn ServerCertVerifier>>, String> {
    let extra: &[&[u8]] = match tls {
        Tls::Web => &[],
        Tls::Edge => &[crate::cloudflare::edge_roots_pem()],
        Tls::Masque | Tls::Plain => return Ok(None),
    };
    let context = crate::trust::context();
    let roots = context.roots().map_err(|e| format!("{e:#}"))?;
    let store = roots.with_extra(extra).map_err(|e| format!("{e:#}"))?;
    let v = rustls::client::WebPkiServerVerifier::builder_with_provider(
        Arc::new(store),
        context.provider().clone(),
    )
    .build()
    .map_err(|e| e.to_string())?;
    Ok(Some(v as Arc<dyn ServerCertVerifier>))
}

/// One TLS handshake on the path to `target`.
pub(super) async fn tls(target: &Target, timeout: Duration) -> TlsResult {
    let unknown = |detail: String| TlsResult {
        verdict: Verdict::Unknown,
        issuer: None,
        detail,
    };
    let inner = match verifier(target.tls) {
        Ok(v) => v,
        Err(e) => return unknown(format!("the roots of podnet: {e}")),
    };
    let recorder = Arc::new(Recorder {
        inner,
        seen: Mutex::new(Vec::new()),
        provider: Arc::new(rustls::crypto::ring::default_provider()),
    });
    let mut config = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(recorder.clone())
        .with_no_client_auth();
    let sni = match target.tls {
        Tls::Web | Tls::Plain => target.host.clone(),
        Tls::Edge => crate::cloudflare::h2_sni().to_string(),
        Tls::Masque => {
            config.alpn_protocols = vec![b"h2".to_vec()];
            crate::warp::register::masque_sni().to_string()
        }
    };
    let Ok(name) = ServerName::try_from(sni.clone()) else {
        return unknown(format!("the name {sni} is not a TLS name"));
    };
    let read = Arc::new(AtomicU64::new(0));
    let work = async {
        let io = crate::egress::dial(&target.host, target.port, None, timeout).await?;
        let io = Counting {
            inner: io,
            read: read.clone(),
        };
        tokio_rustls::TlsConnector::from(Arc::new(config))
            .connect(name, io)
            .await
            .map_err(anyhow::Error::from)
    };
    let result = tokio::time::timeout(timeout, work).await;
    let issuer = recorder
        .seen
        .lock()
        .ok()
        .and_then(|s| s.first().and_then(|c| issuer_of(c)));
    let got = read.load(Ordering::Relaxed);
    let carries_nothing = || TlsResult {
        verdict: Verdict::Refused,
        issuer: None,
        detail: format!(
            "the {} answers and carries nothing: no TLS byte came back",
            if crate::egress::proxy_for(&target.host, target.port).is_some() {
                "proxy"
            } else {
                "peer"
            }
        ),
    };
    match result {
        Ok(Ok(_)) => TlsResult {
            verdict: Verdict::Ok,
            detail: match (&issuer, target.tls) {
                (Some(i), Tls::Masque) => {
                    format!("issuer {i} (pinned per device; not judged here)")
                }
                (Some(i), _) => format!("issuer {i}; the chain is valid"),
                (None, _) => "the chain is valid".into(),
            },
            issuer,
        },
        Err(_) if got == 0 => carries_nothing(),
        Err(_) => unknown(format!(
            "the TLS handshake took more than {} ms",
            timeout.as_millis()
        )),
        Ok(Err(err)) => {
            let cert_fault = err.chain().any(|e| {
                e.downcast_ref::<std::io::Error>()
                    .and_then(|io| io.get_ref())
                    .and_then(|i| i.downcast_ref::<rustls::Error>())
                    .is_some_and(|r| matches!(r, rustls::Error::InvalidCertificate(_)))
            });
            if cert_fault {
                let who = issuer.clone().unwrap_or_else(|| "an unknown issuer".into());
                let hint = match target.tls {
                    Tls::Web => "a proxy terminates TLS: give its root with --ca-file",
                    _ => "a proxy terminates TLS: the edge legs cannot pass through it",
                };
                TlsResult {
                    verdict: Verdict::Refused,
                    issuer,
                    detail: format!("the certificate is issued by {who}, not by a root that podnet expects; {hint}"),
                }
            } else if got == 0 {
                carries_nothing()
            } else {
                TlsResult {
                    verdict: Verdict::Unknown,
                    issuer,
                    detail: format!("{err:#}"),
                }
            }
        }
    }
}
