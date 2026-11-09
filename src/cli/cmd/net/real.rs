//! The real destination of a `net proxy --forward`.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{bail, Context, Result};

/// The time limit of one dial of a forward.
const DIAL_TIMEOUT: Duration = Duration::from_secs(10);

/// A real destination a virtual forward bridges to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RealTarget {
    /// A name or an address, and a port.
    Tcp {
        host: String,
        port: u16,
    },
    Unix(PathBuf),
}

impl RealTarget {
    /// `HOST:PORT`, `[V6]:PORT` or `unix:/PATH`.
    pub(crate) fn parse(value: &str) -> Result<Self> {
        if let Some(path) = value.strip_prefix("unix:") {
            if path.is_empty() {
                bail!("unix: real destination has no path");
            }
            return Ok(Self::Unix(PathBuf::from(path)));
        }
        let bad = || format!("bad real address {value:?}: want HOST:PORT, [V6]:PORT or unix:/PATH");
        let (host, port) = value.rsplit_once(':').with_context(bad)?;
        let host = host
            .strip_prefix('[')
            .map_or(host, |h| h.trim_end_matches(']'));
        if host.is_empty() {
            bail!("{}", bad());
        }
        // A bare IPv6 address has no port: `::1:80` is not an address.
        if host.contains(':') && !value.starts_with('[') {
            bail!("{}", bad());
        }
        let port = port.parse().with_context(bad)?;
        Ok(Self::Tcp {
            host: host.to_string(),
            port,
        })
    }

    pub(crate) fn describe(&self) -> String {
        match self {
            Self::Tcp { host, port } if host.contains(':') => format!("[{host}]:{port}"),
            Self::Tcp { host, port } => format!("{host}:{port}"),
            Self::Unix(path) => format!("unix:{}", path.display()),
        }
    }

    /// True when the dial goes through the proxy of the process (with
    /// `NO_PROXY`): a TCP target that is not a loopback name or address.
    fn uses_process_proxy(&self) -> bool {
        match self {
            Self::Tcp { host, .. } => !is_loopback(host),
            Self::Unix(_) => false,
        }
    }

    /// Open a real connection, type-erased so the forward is generic.
    pub(crate) async fn dial(&self) -> std::io::Result<Box<dyn DialStream>> {
        match self {
            Self::Tcp { host, port } => {
                let route = self
                    .uses_process_proxy()
                    .then(|| podnet::egress::proxy_for(host, *port))
                    .flatten();
                let dialed: Result<Box<dyn DialStream>> = if self.uses_process_proxy() {
                    podnet::egress::dial(host, *port, None, DIAL_TIMEOUT)
                        .await
                        .map(|io| Box::new(io) as Box<dyn DialStream>)
                } else {
                    podnet::egress::dial_direct(host, *port, DIAL_TIMEOUT)
                        .await
                        .map(|io| Box::new(io) as Box<dyn DialStream>)
                };
                dialed.map_err(|err| {
                    let label = route.map(|p| p.label());
                    std::io::Error::other(error_words(&self.describe(), label.as_deref(), &err))
                })
            }
            Self::Unix(path) => Ok(Box::new(
                tokio::net::UnixStream::connect(path).await.map_err(|err| {
                    std::io::Error::new(err.kind(), format!("unix:{}: {err}", path.display()))
                })?,
            )),
        }
    }
}

fn is_loopback(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// The words of a failed dial: the target, the way (the proxy or direct), the
/// hop that failed and the cause.
fn error_words(target: &str, proxy: Option<&str>, err: &anyhow::Error) -> String {
    let way = proxy.map_or("direct".to_string(), |p| format!("via {p}"));
    let hop = podnet::hop::of(err).map_or(String::new(), |h| format!(" (hop: {})", h.name()));
    format!("forward target {target} {way}{hop}: {err:#}")
}

/// The bounds a forwarded connection must satisfy. A single local trait lets
/// the real destination be either a TCP or a unix stream.
pub(crate) trait DialStream:
    tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + Unpin
{
}

impl<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + Unpin> DialStream for T {}

#[cfg(test)]
mod tests {
    use super::*;

    fn tcp(host: &str, port: u16) -> RealTarget {
        RealTarget::Tcp {
            host: host.into(),
            port,
        }
    }

    #[test]
    fn a_literal_a_name_a_v6_and_a_unix_path_parse() {
        assert_eq!(
            RealTarget::parse("127.0.0.1:3000").unwrap(),
            tcp("127.0.0.1", 3000)
        );
        assert_eq!(
            RealTarget::parse("localhost:3000").unwrap(),
            tcp("localhost", 3000)
        );
        assert_eq!(RealTarget::parse("[::1]:80").unwrap(), tcp("::1", 80));
        assert_eq!(
            RealTarget::parse("unix:/run/app.sock").unwrap(),
            RealTarget::Unix("/run/app.sock".into())
        );
        assert_eq!(tcp("::1", 80).describe(), "[::1]:80");
        for bad in ["", "host", "host:x", ":80", "::1:80", "unix:"] {
            assert!(RealTarget::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn only_a_target_that_is_not_loopback_uses_the_process_proxy() {
        assert!(!tcp("127.0.0.1", 1).uses_process_proxy());
        assert!(!tcp("LOCALHOST", 1).uses_process_proxy());
        assert!(!tcp("::1", 1).uses_process_proxy());
        assert!(!RealTarget::Unix("/x".into()).uses_process_proxy());
        assert!(tcp("db.internal", 5432).uses_process_proxy());
        assert!(tcp("10.0.0.5", 5432).uses_process_proxy());
    }

    #[test]
    fn the_error_words_name_the_target_the_way_and_the_hop() {
        let err = podnet::hop::at(podnet::hop::Hop::Proxy, anyhow::anyhow!("refused"));
        let words = error_words("db.internal:5432", Some("http://gw:80"), &err);
        assert!(words.contains("db.internal:5432"), "{words}");
        assert!(words.contains("via http://gw:80"), "{words}");
        assert!(words.contains("hop: proxy"), "{words}");
        let direct = error_words("127.0.0.1:1", None, &anyhow::anyhow!("refused"));
        assert!(direct.contains("direct"), "{direct}");
    }
}
