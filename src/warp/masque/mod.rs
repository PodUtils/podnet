//! Provider-agnostic MASQUE client over HTTP/3 and QUIC (QUIC-1 to QUIC-5).
//!
//! Splits provider enrolment ([`MasqueEnrolment`]) from the shared QUIC/HTTP/3
//! transport client ([`MasqueClient`]), supports `CONNECT-IP` (RFC 9484),
//! `CONNECT-UDP` (RFC 9298), and `WebTransport` on one connection, enforces
//! IPv6-only socket rules, and separates enrolment failures from transport
//! failures ([`MasqueError`]).

pub mod client;
pub mod enrol;
pub mod error;
pub mod family;
pub mod session;
#[doc(hidden)]
pub mod standin;

use std::fmt;
use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;

use pod_core::Target;

pub use client::{
    encode_connect_headers, MasqueClient, MasqueConnection, DEFAULT_HANDSHAKE_TIMEOUT,
};
pub use enrol::{
    EndpointEnrolment, EnrolledEndpoint, MasqueEnrolment, MasqueTlsAuth, TokenEnrolment,
    WarpEnrolment,
};
pub use error::MasqueError;
pub use family::{
    select_endpoints, AttemptKind, AttemptTracker, IpPolicy, SocketAttempt, TrackedUdp,
};
pub use session::{IpSession, UdpSession, WebTransportSession, WEBTRANSPORT_BIDI_STREAM};

/// Maximum DNS hostname length stored in [`MasqueEndpoint`].
const MAX_HOST_LEN: usize = 253;

/// One MASQUE transport mode carried over HTTP/3 extended CONNECT (QUIC-2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MasqueMode {
    /// RFC 9484 IP proxying (`:protocol=connect-ip` or provider-specific protocol).
    ConnectIp,
    /// RFC 9298 UDP proxying (`:protocol=connect-udp`) to `host:port`.
    ConnectUdp { host: String, port: u16 },
    /// WebTransport over HTTP/3 (`:protocol=webtransport`) at `path`.
    WebTransport { path: String },
}

/// A MASQUE front-door endpoint parsed from `--masque-endpoint`.
///
/// Accepts `IP:PORT`, `[IPv6]:PORT`, `HOST:PORT`, or `https://HOST[:PORT]`
/// (default port 443). Implements `Copy` for CLI forwarding.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct MasqueEndpoint {
    addr: Option<SocketAddr>,
    host_buf: [u8; MAX_HOST_LEN],
    host_len: u8,
    port: u16,
    has_scheme: bool,
}

impl MasqueEndpoint {
    /// Build an endpoint from a resolved [`SocketAddr`].
    pub fn from_socket_addr(addr: SocketAddr) -> Self {
        let host = addr.ip().to_string();
        let mut host_buf = [0u8; MAX_HOST_LEN];
        let bytes = host.as_bytes();
        host_buf[..bytes.len()].copy_from_slice(bytes);
        Self {
            addr: Some(addr),
            host_buf,
            host_len: bytes.len() as u8,
            port: addr.port(),
            has_scheme: false,
        }
    }

    /// Parse `IP:PORT`, `[IPv6]:PORT`, `HOST:PORT`, or `https://HOST[:PORT]`.
    pub fn parse(raw: &str) -> Result<Self, MasqueError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(MasqueError::enrolment("MASQUE endpoint must not be empty"));
        }
        let (has_scheme, rest) = if let Some(after) = trimmed.strip_prefix("https://") {
            let after = after.trim_end_matches('/');
            if after.is_empty() || after.contains('/') {
                return Err(MasqueError::enrolment(format!(
                    "{raw:?}: give https://HOST[:PORT] with no path"
                )));
            }
            (true, after)
        } else if trimmed.contains("://") {
            return Err(MasqueError::enrolment(format!(
                "{raw:?}: MASQUE endpoint scheme must be https://"
            )));
        } else {
            (false, trimmed)
        };

        if let Ok(addr) = rest.parse::<SocketAddr>() {
            if addr.port() == 0 {
                return Err(MasqueError::enrolment(format!(
                    "{raw:?}: port must be between 1 and 65535"
                )));
            }
            let mut ep = Self::from_socket_addr(addr);
            ep.has_scheme = has_scheme;
            return Ok(ep);
        }

        let (host, port) = split_host_port(rest, has_scheme).ok_or_else(|| {
            MasqueError::enrolment(format!(
                "{raw:?}: give IP:PORT, [IPv6]:PORT, HOST:PORT, or https://HOST[:PORT]"
            ))
        })?;
        if port == 0 {
            return Err(MasqueError::enrolment(format!(
                "{raw:?}: port must be between 1 and 65535"
            )));
        }
        if host.is_empty() || host.len() > MAX_HOST_LEN || host.contains(char::is_whitespace) {
            return Err(MasqueError::enrolment(format!(
                "{raw:?}: invalid MASQUE endpoint host"
            )));
        }
        let addr = host
            .parse::<IpAddr>()
            .ok()
            .map(|ip| SocketAddr::new(ip, port));
        let mut host_buf = [0u8; MAX_HOST_LEN];
        host_buf[..host.len()].copy_from_slice(host.as_bytes());
        Ok(Self {
            addr,
            host_buf,
            host_len: host.len() as u8,
            port,
            has_scheme,
        })
    }

    /// Return the resolved socket address when the endpoint is an IP literal.
    pub fn socket_addr(&self) -> Option<SocketAddr> {
        self.addr
    }

    /// Return the endpoint host without brackets.
    pub fn host(&self) -> &str {
        std::str::from_utf8(&self.host_buf[..usize::from(self.host_len)]).unwrap_or("")
    }

    /// Return the endpoint port.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Return the TLS SNI hostname when the endpoint specifies a domain name.
    pub fn sni(&self) -> Option<&str> {
        self.addr.is_none().then(|| self.host())
    }

    /// Format the HTTP/3 `:authority` (`host:port` or `[ipv6]:port`).
    pub fn authority(&self) -> String {
        let host = self.host();
        if host.contains(':') {
            format!("[{host}]:{}", self.port)
        } else {
            format!("{host}:{}", self.port)
        }
    }

    /// Convert to a `pod_core::Target`.
    pub fn target(&self) -> Target {
        match self.addr {
            Some(addr) => Target::Addr(addr),
            None => Target::Name {
                host: self.host().to_string(),
                port: self.port,
            },
        }
    }
}

impl FromStr for MasqueEndpoint {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s).map_err(|e| e.detail().to_string())
    }
}

impl fmt::Display for MasqueEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.has_scheme {
            write!(f, "https://{}", self.authority())
        } else {
            f.write_str(&self.authority())
        }
    }
}

impl fmt::Debug for MasqueEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("MasqueEndpoint")
            .field(&self.to_string())
            .finish()
    }
}

/// Convert `MasqueOptions` endpoint fields into a pinned `pod_warp::Door`.
pub(crate) fn door_of(
    addr: Option<SocketAddr>,
    endpoint: Option<MasqueEndpoint>,
) -> Option<pod_warp::Door> {
    if let Some(addr) = addr {
        Some(pod_warp::Door::Pinned(Target::Addr(addr)))
    } else {
        endpoint.map(|ep| pod_warp::Door::Pinned(ep.target()))
    }
}

fn split_host_port(rest: &str, allow_default_port: bool) -> Option<(&str, u16)> {
    if let Some(after_open) = rest.strip_prefix('[') {
        let (host, tail) = after_open.split_once(']')?;
        if host.is_empty() {
            return None;
        }
        if tail.is_empty() {
            return allow_default_port.then_some((host, 443));
        }
        let port = tail.strip_prefix(':')?.parse::<u16>().ok()?;
        return Some((host, port));
    }
    if let Some((host, port_str)) = rest.rsplit_once(':') {
        if host.contains(':') || host.is_empty() {
            return None;
        }
        let port = port_str.parse::<u16>().ok()?;
        return Some((host, port));
    }
    if allow_default_port && !rest.is_empty() {
        return Some((rest, 443));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_parser_accepts_ip_host_and_https_url() {
        let v4 = MasqueEndpoint::parse("127.0.0.1:443").unwrap();
        assert_eq!(v4.socket_addr(), Some("127.0.0.1:443".parse().unwrap()));
        assert_eq!(v4.to_string(), "127.0.0.1:443");

        let v6 = MasqueEndpoint::parse("[2606:4700:103::2]:443").unwrap();
        assert_eq!(
            v6.socket_addr(),
            Some("[2606:4700:103::2]:443".parse().unwrap())
        );
        assert_eq!(v6.to_string(), "[2606:4700:103::2]:443");

        let https_v6 = MasqueEndpoint::parse("https://[::1]:8443").unwrap();
        assert_eq!(https_v6.socket_addr(), Some("[::1]:8443".parse().unwrap()));
        assert_eq!(https_v6.to_string(), "https://[::1]:8443");

        let https_host = MasqueEndpoint::parse("https://masque.example").unwrap();
        assert_eq!(https_host.socket_addr(), None);
        assert_eq!(https_host.host(), "masque.example");
        assert_eq!(https_host.port(), 443);
        assert_eq!(https_host.sni(), Some("masque.example"));

        let host_port = MasqueEndpoint::parse("masque.example:8443").unwrap();
        assert_eq!(host_port.authority(), "masque.example:8443");
        assert_eq!(host_port.to_string(), "masque.example:8443");

        for bad in [
            "",
            "not-an-address",
            "http://example.com:443",
            "https://",
            "https://example.com:443/path",
            "127.0.0.1:0",
        ] {
            assert!(MasqueEndpoint::parse(bad).is_err(), "{bad:?}");
        }
    }
}
