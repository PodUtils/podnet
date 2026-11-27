//! Provider-agnostic MASQUE enrolment trait and providers (QUIC-4).
//!
//! Splits provider enrolment from the MASQUE HTTP/3 transport client:
//! - [`WarpEnrolment`]: wraps Cloudflare WARP device and P-256 identity enrolment.
//! - [`TokenEnrolment`] (aliased as [`EndpointEnrolment`]): generic RFC 9298/9484
//!   MASQUE provider using `--masque-endpoint`, optional bearer token, custom CA,
//!   or pinned SPKI.

use std::net::SocketAddr;

use pod_core::BoxFuture;
use pod_warp::provider::{Cloudflare, Enrolment as PodEnrolment};
use pod_warp::{Api, Device, DeviceHints, Identity};
use rustls::pki_types::{pem::PemObject as _, CertificateDer};

use super::error::MasqueError;
use super::MasqueEndpoint;

/// TLS authentication and server verification material produced by enrolment.
#[derive(Clone, Debug)]
pub enum MasqueTlsAuth {
    /// Mutual TLS with a P-256 device identity and a pinned peer SPKI (DER).
    PinnedKey {
        identity: Identity,
        peer_spki_der: Vec<u8>,
    },
    /// Custom root CA certificates (DER) with no client certificate requirement.
    CustomCa { ca_der: Vec<Vec<u8>> },
    /// Standard podnet trust store roots.
    SystemRoots,
}

/// Provider-neutral output of MASQUE enrolment consumed by [`super::MasqueClient`].
#[derive(Clone, Debug)]
pub struct EnrolledEndpoint {
    /// Provider name reported in logs and status output.
    pub provider: String,
    /// Candidate front-door addresses in priority order.
    pub endpoints: Vec<SocketAddr>,
    /// Optional unresolved endpoint name when DNS lookup is deferred to connect.
    pub unresolved: Option<MasqueEndpoint>,
    /// TLS server name (SNI).
    pub sni: String,
    /// HTTP/3 `:authority` pseudo-header.
    pub authority: String,
    /// `:protocol` value used for `CONNECT-IP` (`connect-ip` or `cf-connect-ip`).
    pub ip_protocol: String,
    /// TLS verification and client authentication material.
    pub tls: MasqueTlsAuth,
    /// Extra HTTP/3 headers sent on extended CONNECT requests.
    pub extra_headers: Vec<(String, String)>,
}

/// Provider enrolment interface split from the MASQUE transport client (QUIC-4).
pub trait MasqueEnrolment: Send + Sync {
    /// Return the provider name.
    fn provider(&self) -> &str;

    /// Enrol with the provider and return transport credentials and endpoints.
    fn enrol(&self) -> BoxFuture<'_, Result<EnrolledEndpoint, MasqueError>>;
}

/// Enrolment source for [`WarpEnrolment`].
#[derive(Clone)]
enum WarpSource {
    Stored { device: Device, identity: Identity },
    Live { api: Api, hints: DeviceHints },
}

/// Provider 1: Cloudflare WARP identity and device enrolment.
#[derive(Clone)]
pub struct WarpEnrolment {
    source: WarpSource,
    endpoint_override: Option<MasqueEndpoint>,
    extra_endpoints: Vec<SocketAddr>,
    sni_override: Option<String>,
    authority_override: Option<String>,
    ip_protocol: String,
}

impl WarpEnrolment {
    /// Build a WARP enrolment adapter from a saved device and MASQUE identity.
    pub fn from_parts(device: Device, identity: Identity) -> Self {
        Self {
            source: WarpSource::Stored { device, identity },
            endpoint_override: None,
            extra_endpoints: Vec::new(),
            sni_override: None,
            authority_override: None,
            ip_protocol: "cf-connect-ip".into(),
        }
    }

    /// Build a WARP enrolment adapter that registers a new device against `api`.
    pub fn from_api(api: Api, hints: DeviceHints) -> Self {
        Self {
            source: WarpSource::Live { api, hints },
            endpoint_override: None,
            extra_endpoints: Vec::new(),
            sni_override: None,
            authority_override: None,
            ip_protocol: "cf-connect-ip".into(),
        }
    }

    /// Override the MASQUE front-door endpoint (`--masque-endpoint`).
    pub fn with_endpoint(mut self, endpoint: MasqueEndpoint) -> Self {
        self.endpoint_override = Some(endpoint);
        self
    }

    /// Set explicit candidate front-door addresses in priority order.
    pub fn with_endpoints(mut self, endpoints: Vec<SocketAddr>) -> Self {
        self.extra_endpoints = endpoints;
        self
    }

    /// Override the TLS server name (`--masque-sni`).
    pub fn with_sni(mut self, sni: impl Into<String>) -> Self {
        self.sni_override = Some(sni.into());
        self
    }

    /// Override the HTTP/3 `:authority` header.
    pub fn with_authority(mut self, authority: impl Into<String>) -> Self {
        self.authority_override = Some(authority.into());
        self
    }

    /// Override the `CONNECT-IP` `:protocol` value.
    pub fn with_ip_protocol(mut self, protocol: impl Into<String>) -> Self {
        self.ip_protocol = protocol.into();
        self
    }

    fn build_enrolled(
        &self,
        device: &Device,
        identity: &Identity,
        pin_override: Option<Vec<u8>>,
    ) -> Result<EnrolledEndpoint, MasqueError> {
        let pin = match pin_override {
            Some(pin) if !pin.is_empty() => pin,
            _ => {
                let pem = device.peer_public_key_pem().ok_or_else(|| {
                    MasqueError::enrolment(
                        "WARP device record has no pinned peer public key".to_string(),
                    )
                })?;
                pod_warp::identity::spki_from_pem(pem).map_err(|e| {
                    MasqueError::enrolment(format!("invalid WARP peer public key: {}", e.chain()))
                })?
            }
        };

        let mut endpoints = Vec::new();
        let mut unresolved = None;
        if !self.extra_endpoints.is_empty() {
            endpoints.extend_from_slice(&self.extra_endpoints);
        } else if let Some(ep) = self.endpoint_override {
            if let Some(addr) = ep.socket_addr() {
                endpoints.push(addr);
            } else {
                unresolved = Some(ep);
            }
        } else {
            endpoints = device.masque_endpoints();
        }
        if endpoints.is_empty() && unresolved.is_none() {
            return Err(MasqueError::enrolment(
                "WARP device record has no MASQUE endpoint".to_string(),
            ));
        }

        let sni = self
            .sni_override
            .clone()
            .or_else(|| {
                self.endpoint_override
                    .and_then(|e| e.sni().map(str::to_string))
            })
            .unwrap_or_else(|| pod_warp::service::masque_sni().to_string());
        let authority = self
            .authority_override
            .clone()
            .unwrap_or_else(|| pod_warp::service::connect_uri().to_string());

        Ok(EnrolledEndpoint {
            provider: "warp".into(),
            endpoints,
            unresolved,
            sni,
            authority,
            ip_protocol: self.ip_protocol.clone(),
            tls: MasqueTlsAuth::PinnedKey {
                identity: identity.clone(),
                peer_spki_der: pin,
            },
            extra_headers: Vec::new(),
        })
    }
}

impl MasqueEnrolment for WarpEnrolment {
    fn provider(&self) -> &str {
        "warp"
    }

    fn enrol(&self) -> BoxFuture<'_, Result<EnrolledEndpoint, MasqueError>> {
        Box::pin(async move {
            match &self.source {
                WarpSource::Stored { device, identity } => {
                    self.build_enrolled(device, identity, None)
                }
                WarpSource::Live { api, hints } => {
                    let identity = Identity::generate().map_err(|e| {
                        MasqueError::enrolment(format!("generating WARP identity: {}", e.chain()))
                    })?;
                    let adapter = Cloudflare::new(api.clone());
                    let enrolled = adapter
                        .enrol(&identity, hints)
                        .await
                        .map_err(|e| MasqueError::from_enrol_error(&e))?;
                    let device = enrolled.device.ok_or_else(|| {
                        MasqueError::enrolment("WARP enrolment returned no device record")
                    })?;
                    self.build_enrolled(&device, &identity, Some(enrolled.pin))
                }
            }
        })
    }
}

/// Provider 2: generic RFC 9298/9484 MASQUE endpoint with optional bearer token and CA.
#[derive(Clone, Debug)]
pub struct TokenEnrolment {
    provider_name: String,
    endpoint: MasqueEndpoint,
    candidate_endpoints: Vec<SocketAddr>,
    bearer_token: Option<String>,
    require_token: bool,
    ca_der: Vec<Vec<u8>>,
    ca_error: Option<String>,
    pinned: Option<(Identity, Vec<u8>)>,
    sni: Option<String>,
    authority: Option<String>,
    ip_protocol: String,
}

/// Alias for [`TokenEnrolment`] emphasizing generic endpoint enrolment.
pub type EndpointEnrolment = TokenEnrolment;

impl TokenEnrolment {
    /// Create a generic MASQUE provider for `endpoint` (`--masque-endpoint`).
    pub fn new(endpoint: MasqueEndpoint) -> Self {
        Self {
            provider_name: "endpoint".into(),
            endpoint,
            candidate_endpoints: Vec::new(),
            bearer_token: None,
            require_token: false,
            ca_der: Vec::new(),
            ca_error: None,
            pinned: None,
            sni: None,
            authority: None,
            ip_protocol: "connect-ip".into(),
        }
    }

    /// Set a custom provider label reported in logs and status.
    pub fn with_provider_name(mut self, name: impl Into<String>) -> Self {
        self.provider_name = name.into();
        self
    }

    /// Attach a bearer token sent in the `authorization` header.
    pub fn with_bearer_token(mut self, token: impl Into<String>) -> Self {
        self.bearer_token = Some(token.into());
        self.require_token = true;
        self
    }

    /// Require a non-empty bearer token during enrolment.
    pub fn require_bearer_token(mut self) -> Self {
        self.require_token = true;
        self
    }

    /// Trust one custom CA certificate in DER format.
    pub fn with_ca_der(mut self, der: Vec<u8>) -> Self {
        self.ca_der.push(der);
        self
    }

    /// Trust custom CA certificates parsed from PEM bytes.
    pub fn with_ca_pem(mut self, pem: &[u8]) -> Self {
        let mut parsed = Vec::new();
        for item in CertificateDer::pem_slice_iter(pem) {
            match item {
                Ok(cert) => parsed.push(cert.to_vec()),
                Err(err) => {
                    self.ca_error = Some(format!("invalid CA PEM certificate: {err}"));
                    return self;
                }
            }
        }
        if parsed.is_empty() {
            self.ca_error = Some("CA PEM bundle holds no certificate".into());
            return self;
        }
        self.ca_der.extend(parsed);
        self
    }

    /// Pin a peer SPKI (DER) with a client identity instead of CA verification.
    pub fn with_pinned_spki(mut self, identity: Identity, peer_spki_der: Vec<u8>) -> Self {
        self.pinned = Some((identity, peer_spki_der));
        self
    }

    /// Override the TLS server name (SNI).
    pub fn with_sni(mut self, sni: impl Into<String>) -> Self {
        self.sni = Some(sni.into());
        self
    }

    /// Override the HTTP/3 `:authority` header.
    pub fn with_authority(mut self, authority: impl Into<String>) -> Self {
        self.authority = Some(authority.into());
        self
    }

    /// Override the `CONNECT-IP` `:protocol` value (default `"connect-ip"`).
    pub fn with_ip_protocol(mut self, protocol: impl Into<String>) -> Self {
        self.ip_protocol = protocol.into();
        self
    }

    /// Provide explicit candidate socket addresses in priority order.
    pub fn with_endpoints(mut self, endpoints: Vec<SocketAddr>) -> Self {
        self.candidate_endpoints = endpoints;
        self
    }
}

impl MasqueEnrolment for TokenEnrolment {
    fn provider(&self) -> &str {
        &self.provider_name
    }

    fn enrol(&self) -> BoxFuture<'_, Result<EnrolledEndpoint, MasqueError>> {
        Box::pin(async move {
            if self.provider_name.trim().is_empty() {
                return Err(MasqueError::enrolment(
                    "MASQUE provider name must not be empty",
                ));
            }
            if let Some(err) = &self.ca_error {
                return Err(MasqueError::enrolment(err.clone()));
            }
            let mut extra_headers = Vec::new();
            if self.require_token {
                let token = self.bearer_token.as_deref().map(str::trim).unwrap_or("");
                if token.is_empty() {
                    return Err(MasqueError::enrolment(
                        "bearer token is missing or empty for token enrolment",
                    ));
                }
                extra_headers.push(("authorization".to_string(), format!("Bearer {token}")));
            }

            let tls = if let Some((identity, peer_spki_der)) = &self.pinned {
                if peer_spki_der.is_empty() {
                    return Err(MasqueError::enrolment("pinned peer SPKI must not be empty"));
                }
                MasqueTlsAuth::PinnedKey {
                    identity: identity.clone(),
                    peer_spki_der: peer_spki_der.clone(),
                }
            } else if !self.ca_der.is_empty() {
                MasqueTlsAuth::CustomCa {
                    ca_der: self.ca_der.clone(),
                }
            } else {
                MasqueTlsAuth::SystemRoots
            };

            let (endpoints, unresolved) = if !self.candidate_endpoints.is_empty() {
                (self.candidate_endpoints.clone(), None)
            } else if let Some(addr) = self.endpoint.socket_addr() {
                (vec![addr], None)
            } else {
                (Vec::new(), Some(self.endpoint))
            };

            let sni = self
                .sni
                .clone()
                .unwrap_or_else(|| self.endpoint.host().to_string());
            let authority = self
                .authority
                .clone()
                .unwrap_or_else(|| self.endpoint.authority());

            Ok(EnrolledEndpoint {
                provider: self.provider_name.clone(),
                endpoints,
                unresolved,
                sni,
                authority,
                ip_protocol: self.ip_protocol.clone(),
                tls,
                extra_headers,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn token_enrolment_rejects_empty_bearer_token_as_enrolment_error() {
        let ep = MasqueEndpoint::parse("https://127.0.0.1:8443").unwrap();
        let provider = TokenEnrolment::new(ep).with_bearer_token("   ");
        let err = provider.enrol().await.unwrap_err();
        assert!(err.is_enrolment(), "{err}");
        assert_eq!(err.kind(), "enrolment");
    }

    #[tokio::test]
    async fn token_enrolment_rejects_bad_ca_pem_as_enrolment_error() {
        let ep = MasqueEndpoint::parse("https://[::1]:8443").unwrap();
        let provider = TokenEnrolment::new(ep).with_ca_pem(b"not a pem cert");
        let err = provider.enrol().await.unwrap_err();
        assert!(err.is_enrolment(), "{err}");
    }

    #[tokio::test]
    async fn token_enrolment_attaches_bearer_header_and_connect_ip_protocol() {
        let ep = MasqueEndpoint::parse("https://[::1]:8443").unwrap();
        let provider = TokenEnrolment::new(ep)
            .with_provider_name("acme-masque")
            .with_bearer_token("secret-123")
            .with_sni("localhost");
        let enrolled = provider.enrol().await.unwrap();
        assert_eq!(enrolled.provider, "acme-masque");
        assert_eq!(enrolled.ip_protocol, "connect-ip");
        assert_eq!(enrolled.sni, "localhost");
        assert_eq!(enrolled.endpoints, vec!["[::1]:8443".parse().unwrap()]);
        assert_eq!(
            enrolled.extra_headers,
            vec![("authorization".into(), "Bearer secret-123".into())]
        );
    }
}
