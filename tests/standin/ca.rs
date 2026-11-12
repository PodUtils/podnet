//! The test CA of the stand-in edge. It is made at run time, for each
//! stand-in, and never leaves the test.

use std::sync::Arc;

use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, Issuer, KeyPair};

/// A CA that signs the certificate of the stand-in edge.
pub struct Ca {
    /// The certificate of the CA, as PEM. A test gives it to podnet with
    /// `SSL_CERT_FILE` (the roots podnet already reads).
    pub pem: String,
    issuer: Issuer<'static, KeyPair>,
}

impl Ca {
    pub fn new() -> Ca {
        let key = KeyPair::generate().expect("a CA key");
        let mut params = CertificateParams::new(Vec::<String>::new()).expect("CA parameters");
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params
            .distinguished_name
            .push(DnType::CommonName, "podnet stand-in edge CA");
        let pem = params.self_signed(&key).expect("a CA certificate").pem();
        Ca {
            pem,
            issuer: Issuer::new(params, key),
        }
    }

    /// The TLS server configuration for `names`. It offers no ALPN: the
    /// edge of Cloudflare speaks HTTP/2 on a connection with no ALPN.
    pub fn server_config(&self, names: &[&str]) -> Arc<rustls::ServerConfig> {
        let key = KeyPair::generate().expect("a leaf key");
        let leaf = CertificateParams::new(names.iter().map(|n| n.to_string()).collect::<Vec<_>>())
            .expect("leaf parameters")
            .signed_by(&key, &self.issuer)
            .expect("a leaf certificate");
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .expect("TLS versions")
            .with_no_client_auth()
            .with_single_cert(
                vec![rustls::pki_types::CertificateDer::from(leaf.der().to_vec())],
                rustls::pki_types::PrivateKeyDer::Pkcs8(key.serialize_der().into()),
            )
            .expect("a server configuration");
        Arc::new(config)
    }
}
