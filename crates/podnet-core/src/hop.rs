//! The hop of a failure: which part of the path broke. The error line of
//! podnet names it, and gives a second line with what may help; the failure
//! report (`PODNET_ERROR_PROGRAM`) has it in `hop`.

use std::fmt;

/// One part of the path from podnet to a visitor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hop {
    /// The HTTP or SOCKS5 proxy.
    Proxy,
    /// Name resolution.
    Dns,
    /// TLS with the edge, the MASQUE front door or an API.
    Tls,
    /// The network ended a connection: a reset, or a close, before or
    /// during the TLS handshake (issue 127: an egress policy, not a trust
    /// or a clock problem).
    Network,
    /// The registration of the tunnel with the edge.
    EdgeRegister,
    /// An edge connection after it registered.
    EdgeStream,
    /// The origin of the tunnel.
    Origin,
}

impl Hop {
    pub fn name(self) -> &'static str {
        match self {
            Hop::Proxy => "proxy",
            Hop::Dns => "dns",
            Hop::Tls => "tls",
            Hop::Network => "network",
            Hop::EdgeRegister => "edge-register",
            Hop::EdgeStream => "edge-stream",
            Hop::Origin => "origin",
        }
    }

    /// What may help, for a failure of this hop.
    pub fn hint(self) -> &'static str {
        match self {
            Hop::Proxy => "check the proxy URL, its user and password, and the allow-list of the proxy; --no-proxy goes around it",
            Hop::Dns => "give the address with --resolve HOST=IP, or let a proxy resolve names (socks5h://)",
            Hop::Tls => "behind a proxy that inspects TLS, give its root with --ca-file; check the clock of the host",
            Hop::Network => "the network ended the connection before the TLS handshake was done: a policy of this host or of its network (an egress allow-list) may block this destination; podnet doctor shows each path",
            Hop::EdgeRegister => "when the edge says retry: no, check the token or the credentials file; when it says retry: yes, start again after the wait that it gives",
            Hop::EdgeStream => "the edge connection was lost; podnet connects again, and --leg-failover and --transport warp choose other legs",
            Hop::Origin => "check that the origin listens on the address of --url",
        }
    }
}

impl fmt::Display for Hop {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// An error with its hop. The text is the text of the error; the causes
/// come through `source`.
#[derive(Debug)]
pub struct AtHop {
    pub hop: Hop,
    inner: anyhow::Error,
}

impl fmt::Display for AtHop {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.inner)
    }
}

impl AtHop {
    /// The error that has the hop (its own chain).
    pub fn inner(&self) -> &anyhow::Error {
        &self.inner
    }
}

impl std::error::Error for AtHop {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.inner.source()
    }
}

/// Give `err` the hop `hop`. An error that has a hop keeps it.
pub fn at(hop: Hop, err: anyhow::Error) -> anyhow::Error {
    if of(&err).is_some() {
        return err;
    }
    anyhow::Error::new(AtHop { hop, inner: err })
}

/// The hop of `err`: the innermost one that the chain names.
pub fn of(err: &anyhow::Error) -> Option<Hop> {
    let tagged = err
        .chain()
        .filter_map(|e| e.downcast_ref::<AtHop>())
        .last()
        .map(|e| e.hop);
    tagged
        .or_else(|| {
            err.chain()
                .any(|e| e.downcast_ref::<pod_cloudflare::fail::Refused>().is_some())
                .then_some(Hop::EdgeRegister)
        })
        .or_else(|| {
            err.chain()
                .filter_map(|e| e.downcast_ref::<pod_core::Failure>())
                .filter_map(|f| of_stage(f.stage))
                .last()
        })
}

/// The hop of a stage of a failure of a crate.
pub fn of_stage(stage: pod_core::Stage) -> Option<Hop> {
    use pod_core::Stage;
    match stage {
        Stage::Proxy => Some(Hop::Proxy),
        Stage::Dns => Some(Hop::Dns),
        Stage::Tls => Some(Hop::Tls),
        Stage::Network => Some(Hop::Network),
        Stage::EdgeRegister => Some(Hop::EdgeRegister),
        Stage::EdgeStream => Some(Hop::EdgeStream),
        Stage::Origin => Some(Hop::Origin),
        _ => None,
    }
}

/// Extension for `Result`: `.hop(Hop::Proxy)`.
pub trait ResultExt<T> {
    fn hop(self, hop: Hop) -> anyhow::Result<T>;
}

impl<T, E: Into<anyhow::Error>> ResultExt<T> for Result<T, E> {
    fn hop(self, hop: Hop) -> anyhow::Result<T> {
        self.map_err(|e| at(hop, e.into()))
    }
}

/// The text of a TLS error: "TLS alert: `<description>`" for an alert of the
/// peer, the end of the connection for a reset or a close, else the text
/// of the error.
pub fn tls_text(err: &std::io::Error) -> String {
    if let Some(inner) = err.get_ref() {
        if let Some(rustls::Error::AlertReceived(alert)) = inner.downcast_ref::<rustls::Error>() {
            return format!("TLS alert from the peer: {alert:?}");
        }
    }
    match err.kind() {
        std::io::ErrorKind::ConnectionReset => {
            format!("the connection was reset during the TLS handshake ({err})")
        }
        std::io::ErrorKind::UnexpectedEof => {
            "the connection was closed during the TLS handshake".to_string()
        }
        _ => err.to_string(),
    }
}

/// The hop of an error of a TLS handshake (issue 127): an error of rustls
/// (an alert, a certificate) is [`Hop::Tls`]; a reset, a close or a time
/// out of the connection is [`Hop::Network`].
pub fn tls_hop(err: &std::io::Error) -> Hop {
    use std::io::ErrorKind::*;
    if err
        .get_ref()
        .is_some_and(|inner| inner.downcast_ref::<rustls::Error>().is_some())
    {
        return Hop::Tls;
    }
    match err.kind() {
        ConnectionReset | ConnectionAborted | BrokenPipe | UnexpectedEof | TimedOut
        | ConnectionRefused => Hop::Network,
        _ => Hop::Tls,
    }
}

/// The kind of a tunnel whose registration the edge refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TunnelKind {
    /// A quick tunnel: the quick-tunnel service made it some seconds
    /// before.
    Quick,
    /// A named tunnel: a token or a credentials file.
    Named,
}

/// One refusal of a registration, as the edge said it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Said {
    /// The cause that the edge gave: "Unauthorized: Tunnel not found".
    pub cause: String,
    /// The edge asks for a retry (`retry: yes`).
    pub retry: bool,
}

/// The words of pod-cloudflare before the cause of a refusal
/// (`pod_cloudflare::fail::Refused`).
const REFUSED: &str = "the edge refused the registration: ";

/// The refusals of a registration in `err`. The typed refusal of
/// pod-cloudflare comes first. pod-cloudflare v0.82.0 does not keep it
/// when it joins the failures of the legs ("no edge leg worked: ..."):
/// the second path reads each refusal from the text of the chain.
pub fn refusals(err: &anyhow::Error) -> Vec<Said> {
    let typed: Vec<Said> = err
        .chain()
        .filter_map(|e| e.downcast_ref::<pod_cloudflare::fail::Refused>())
        .map(|r| Said {
            cause: r.cause.clone(),
            retry: r.should_retry,
        })
        .collect();
    if !typed.is_empty() {
        return typed;
    }
    let text: Vec<String> = err.chain().map(|e| e.to_string()).collect();
    let mut found = Vec::new();
    for mut rest in text.iter().map(String::as_str) {
        while let Some(at) = rest.find(REFUSED) {
            rest = &rest[at + REFUSED.len()..];
            let Some(end) = rest.find(" (retry: ") else {
                break;
            };
            let said = Said {
                cause: rest[..end].to_string(),
                retry: rest[end..].starts_with(" (retry: yes"),
            };
            if !found.contains(&said) {
                found.push(said);
            }
            rest = &rest[end..];
        }
    }
    found
}

/// A refused registration, as podnet reads it: the kind of the tunnel,
/// and whether a new start can work. The text is the lead of the error
/// line; the cause is the error of the tunnel.
#[derive(Debug)]
pub struct EdgeRefusal {
    pub tunnel: TunnelKind,
    /// Each refusal asks for a retry.
    pub retry: bool,
    /// A refusal says "Tunnel not found".
    pub not_found: bool,
    inner: anyhow::Error,
}

impl EdgeRefusal {
    /// A new start can work: the edge asks for a retry, or it does not
    /// know a quick tunnel that the service made some seconds before
    /// (live 45169011007). The exit code is 75.
    pub fn transient(&self) -> bool {
        self.retry || (self.tunnel == TunnelKind::Quick && self.not_found)
    }

    /// What may help.
    pub fn hint(&self) -> &'static str {
        match (self.tunnel, self.transient(), self.not_found) {
            (TunnelKind::Quick, true, _) => "the edge does not know the new quick tunnel yet; a new tunnel can take some seconds to get to the edge; start podnet again (exit 75)",
            (TunnelKind::Named, true, _) => "the edge asks for a retry; a new named tunnel can take some seconds to get to the edge; start podnet again after the wait that it gives (exit 75)",
            (TunnelKind::Quick, false, _) => "the edge refused the credentials that the quick-tunnel service gave; start podnet again to get a new quick tunnel",
            (TunnelKind::Named, false, true) => "the edge does not know this tunnel; check that the named tunnel exists, and that the token or the credentials file is of this tunnel",
            (TunnelKind::Named, false, false) => "a refused token does not change with a retry; check the token or the credentials file, or use a quick tunnel",
        }
    }
}

impl fmt::Display for EdgeRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.tunnel {
            TunnelKind::Quick => f.write_str("the quick tunnel did not register"),
            TunnelKind::Named => f.write_str("the named tunnel did not register"),
        }
    }
}

impl std::error::Error for EdgeRefusal {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.inner.as_ref())
    }
}

/// `err` as an [`EdgeRefusal`] of a tunnel of kind `tunnel` when it holds
/// a refusal of the registration; else `err`.
pub fn read_refusal(err: anyhow::Error, tunnel: TunnelKind) -> anyhow::Error {
    if refusal_of(&err).is_some() {
        return err;
    }
    let said = refusals(&err);
    if said.is_empty() {
        return err;
    }
    anyhow::Error::new(EdgeRefusal {
        tunnel,
        retry: said.iter().all(|s| s.retry),
        not_found: said
            .iter()
            .any(|s| s.cause.to_ascii_lowercase().contains("tunnel not found")),
        inner: err,
    })
}

/// The [`EdgeRefusal`] in the chain of `err`.
pub fn refusal_of(err: &anyhow::Error) -> Option<&EdgeRefusal> {
    err.chain().find_map(|e| e.downcast_ref::<EdgeRefusal>())
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Context;

    // Issue 127: a reset during the handshake is the network, not trust.
    #[test]
    fn a_reset_in_the_tls_handshake_is_the_network() {
        use std::io::{Error, ErrorKind};
        let reset = Error::from(ErrorKind::ConnectionReset);
        assert_eq!(tls_hop(&reset), Hop::Network);
        assert!(tls_text(&reset).contains("reset during the TLS handshake"));
        assert_eq!(
            tls_hop(&Error::from(ErrorKind::UnexpectedEof)),
            Hop::Network
        );
        let cert = Error::new(
            ErrorKind::InvalidData,
            rustls::Error::InvalidCertificate(rustls::CertificateError::UnknownIssuer),
        );
        assert_eq!(tls_hop(&cert), Hop::Tls);
        assert!(!Hop::Network.hint().contains("--ca-file"));
        assert!(!Hop::Network.hint().contains("clock"));
    }

    #[test]
    fn the_innermost_hop_is_kept_through_context() {
        let err = Err::<(), _>(anyhow::anyhow!("403 not on the egress allowlist"))
            .hop(Hop::Proxy)
            .context("POST https://api.trycloudflare.com/tunnel")
            .map_err(|e| at(Hop::EdgeRegister, e))
            .unwrap_err();
        assert_eq!(of(&err), Some(Hop::Proxy));
        assert_eq!(
            format!("{err:#}"),
            "POST https://api.trycloudflare.com/tunnel: 403 not on the egress allowlist"
        );
        assert_eq!(of(&anyhow::anyhow!("plain")), None);
    }

    #[test]
    fn a_tls_alert_is_named() {
        let err = std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            rustls::Error::AlertReceived(rustls::AlertDescription::HandshakeFailure),
        );
        assert_eq!(tls_text(&err), "TLS alert from the peer: HandshakeFailure");
    }
}
