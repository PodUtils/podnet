//! Origin connections: TCP, TLS, unix sockets, and TLS over unix.

use std::sync::Once;

use anyhow::{bail, Context, Result};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;

pub trait AsyncReadWrite: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> AsyncReadWrite for T {}

pub type BoxedIo = Box<dyn AsyncReadWrite>;

/// Make `path` free for a new unix listener (issues 202 and 115, and the
/// rule of podssh): a socket that answers belongs to another process and
/// is refused (exit 69), unless `replace`; a stale socket file is removed;
/// another kind of file is never removed (exit 64).
#[cfg(unix)]
pub fn claim_socket_path(path: &std::path::Path, replace: bool) -> Result<()> {
    use crate::exit::{fault, Fault};
    use std::os::unix::fs::FileTypeExt as _;
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_socket() => {
            if !replace && std::os::unix::net::UnixStream::connect(path).is_ok() {
                return Err(fault(
                    Fault::Unavailable,
                    anyhow::anyhow!(
                        "unix:{} is served by another process (podnet ps; podnet stop NAME, or --replace where the command has it)",
                        path.display()
                    ),
                ));
            }
            std::fs::remove_file(path)
                .with_context(|| format!("removing the old socket unix:{}", path.display()))?;
            Ok(())
        }
        Ok(_) => Err(fault(
            Fault::Usage,
            anyhow::anyhow!("unix:{} exists and is not a socket", path.display()),
        )),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err).with_context(|| format!("unix:{}", path.display())),
    }
}

/// Give a refused bind (`AddrInUse`, `PermissionDenied`, `AddrNotAvailable`
/// in the chain of `err`) the class `Fault::Unavailable`: exit 69.
pub fn bind_fault(err: anyhow::Error) -> anyhow::Error {
    use crate::exit::{fault, Fault};
    use std::io::ErrorKind::{AddrInUse, AddrNotAvailable, PermissionDenied};
    let refused = err.chain().any(|e| {
        e.downcast_ref::<std::io::Error>()
            .is_some_and(|io| matches!(io.kind(), AddrInUse | PermissionDenied | AddrNotAvailable))
    });
    if refused {
        fault(Fault::Unavailable, err)
    } else {
        err
    }
}

/// Bind a TCP listener. A refused bind exits 69.
pub async fn bind_tcp(addr: std::net::SocketAddr) -> Result<tokio::net::TcpListener> {
    tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding tcp://{addr}"))
        .map_err(bind_fault)
}

pub fn init_crypto() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = tokio_rustls::rustls::crypto::ring::default_provider().install_default();
    });
}

/// The addresses of a name of RFC 6761 for this host: `localhost` and
/// `*.localhost` (issue 119: a static build on a host with no /etc/hosts
/// did not resolve `localhost`).
pub fn local_name(host: &str) -> Option<Vec<std::net::IpAddr>> {
    let name = host.trim_end_matches('.').to_ascii_lowercase();
    (name == "localhost" || name.ends_with(".localhost")).then(|| {
        vec![
            std::net::IpAddr::from([127, 0, 0, 1]),
            std::net::IpAddr::from(std::net::Ipv6Addr::LOCALHOST),
        ]
    })
}

/// `HOST:PORT` or `[V6]:PORT`.
#[doc(hidden)]
pub fn split_host_port(addr: &str) -> Option<(&str, u16)> {
    let (host, port) = addr.rsplit_once(':')?;
    let host = host.trim_start_matches('[').trim_end_matches(']');
    Some((host, port.parse().ok()?))
}

/// Connect to an origin. `localhost` needs no resolver. The error says if
/// the name or the connection failed (issue 119: both were "connecting to
/// HOST:PORT"). An origin name does not go to DNS over HTTPS.
pub async fn connect_origin(host: &str, port: u16) -> Result<TcpStream> {
    let addrs: Vec<std::net::SocketAddr> = match host.parse::<std::net::IpAddr>() {
        Ok(ip) => vec![(ip, port).into()],
        Err(_) => match local_name(host) {
            Some(ips) => ips.into_iter().map(|ip| (ip, port).into()).collect(),
            None => tokio::net::lookup_host((host, port))
                .await
                .map_err(|e| {
                    crate::hop::at(
                        crate::hop::Hop::Dns,
                        anyhow::anyhow!("resolving the origin {host}: {e}"),
                    )
                })?
                .collect(),
        },
    };
    let mut last = None;
    for addr in &addrs {
        match TcpStream::connect(addr).await {
            Ok(tcp) => return Ok(tcp),
            Err(e) => last = Some(anyhow::anyhow!("connecting to {addr}: {e}")),
        }
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("the origin {host} has no address")))
}

/// Parse `http(s)://host[:port]` into `(host, port, tls)`.
pub fn parse_http_url(url: &str) -> Result<(String, u16, bool)> {
    let (scheme, rest) = url
        .split_once("://")
        .context("origin URL is missing a scheme")?;
    let tls = match scheme {
        "http" => false,
        "https" => true,
        other => bail!("unsupported origin scheme {other:?}"),
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() {
        bail!("origin URL has no host");
    }
    let (host, port) = if let Some(rest) = authority.strip_prefix('[') {
        let (host, rest) = rest
            .split_once(']')
            .context("origin URL has an unclosed IPv6 address")?;
        let port = match rest.strip_prefix(':') {
            Some(port) => port.parse().context("origin URL port is invalid")?,
            None if rest.is_empty() => {
                if tls {
                    443
                } else {
                    80
                }
            }
            None => bail!("origin URL has trailing characters after the IPv6 address"),
        };
        (host.to_string(), port)
    } else {
        match authority.rsplit_once(':') {
            Some((host, port)) if !port.contains(']') => {
                let port: u16 = port.parse().context("origin URL port is invalid")?;
                (host.to_string(), port)
            }
            _ => (authority.to_string(), if tls { 443 } else { 80 }),
        }
    };
    if host.is_empty() {
        bail!("origin URL has no host");
    }
    Ok((host, port, tls))
}
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_second_bind_on_one_port_exits_69() {
        pod_testkit::need_loopback!();
        let first = bind_tcp(([127, 0, 0, 1], 0).into()).await.unwrap();
        let err = bind_tcp(first.local_addr().unwrap()).await.unwrap_err();
        assert_eq!(crate::exit::code_of(&err), crate::exit::UNAVAILABLE);
    }

    #[test]
    fn a_claim_gives_69_for_a_live_socket_64_for_a_file_and_removes_a_stale_one() {
        use crate::exit::{code_of, UNAVAILABLE, USAGE};
        let dir = tempfile::tempdir().unwrap();
        let live = dir.path().join("live.sock");
        let _listener = std::os::unix::net::UnixListener::bind(&live).unwrap();
        let err = claim_socket_path(&live, false).unwrap_err();
        assert_eq!(code_of(&err), UNAVAILABLE);
        let file = dir.path().join("plain");
        std::fs::write(&file, b"x").unwrap();
        let err = claim_socket_path(&file, false).unwrap_err();
        assert_eq!(code_of(&err), USAGE);
        assert!(file.exists());
        let stale = dir.path().join("stale.sock");
        drop(std::os::unix::net::UnixListener::bind(&stale).unwrap());
        claim_socket_path(&stale, false).unwrap();
        assert!(!stale.exists());
    }

    // Issue 119: localhost needs no resolver (a host with no /etc/hosts).
    #[test]
    fn localhost_is_a_local_name() {
        for name in ["localhost", "LOCALHOST.", "app.localhost"] {
            let ips = local_name(name).unwrap();
            assert_eq!(ips[0], std::net::IpAddr::from([127, 0, 0, 1]), "{name}");
        }
        assert!(local_name("example.com").is_none());
        assert!(local_name("localhost.example.com").is_none());
    }

    #[tokio::test]
    async fn an_origin_on_localhost_connects_and_a_refusal_names_the_address() {
        pod_testkit::need_loopback!();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(connect_origin("localhost", port).await.is_ok());
        drop(listener);
        let err = connect_origin("localhost", port).await.unwrap_err();
        assert!(format!("{err}").contains("connecting to "), "{err}");
        assert!(!format!("{err}").contains("resolving"), "{err}");
        let err = connect_origin("no-such-name.invalid", 80)
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("resolving the origin"), "{err}");
        assert_eq!(split_host_port("[::1]:8080"), Some(("::1", 8080)));
    }

    #[test]
    fn parses_urls() {
        assert_eq!(
            parse_http_url("http://127.0.0.1:8080").unwrap(),
            ("127.0.0.1".into(), 8080, false)
        );
        assert_eq!(
            parse_http_url("https://example.com").unwrap(),
            ("example.com".into(), 443, true)
        );
        assert_eq!(
            parse_http_url("http://example.com/x").unwrap(),
            ("example.com".into(), 80, false)
        );
        assert_eq!(
            parse_http_url("https://[::1]:8443").unwrap(),
            ("::1".into(), 8443, true)
        );
        assert!(parse_http_url("ftp://x").is_err());
        assert!(parse_http_url("http://").is_err());
    }
}
