//! Shared upstream logic: proxy specs, routing rules, SOCKS5/HTTP handshakes,
//! UDP association (the glue of `pod_proxy::udp`), and chaining.
//!
//! Both `podnet-netns` backends and any other caller use one implementation, so a
//! new transport is added once rather than per backend.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use anyhow::{bail, Context, Result};

/// An upstream that never answers must not stall the supervisor and the child's
/// `connect` forever.
pub const UPSTREAM_TIMEOUT: Duration = Duration::from_secs(30);

pub async fn within_timeout<F, T>(what: &str, future: F) -> Result<T>
where
    F: std::future::Future<Output = Result<T>>,
{
    match tokio::time::timeout(UPSTREAM_TIMEOUT, future).await {
        Ok(result) => result,
        Err(_) => bail!("{what} timed out after {:?}", UPSTREAM_TIMEOUT),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProxyKind {
    Socks5,
    Http,
    /// Not a proxy at all: dial the target through a running `tailscaled`'s
    /// LocalAPI `ts-dial` endpoint, so the tailnet is the transport.
    Tailscale,
    /// No proxy: TCP to the target from this process (a rule `=direct`).
    Direct,
}

#[derive(Clone, Debug)]
pub struct Proxy {
    pub kind: ProxyKind,
    pub host: String,
    pub port: u16,
    pub user: Option<String>,
    pub pass: Option<String>,
    /// When set, the proxy is reached over an `AF_UNIX` socket instead of TCP.
    /// A sealed host may permit unix connects and deny every TCP one.
    pub unix_path: Option<String>,
}

impl Proxy {
    /// Whether the proxy is reached over an `AF_UNIX` socket.
    pub fn is_unix(&self) -> bool {
        self.unix_path.is_some()
    }

    pub fn parse(spec: &str) -> Result<Self> {
        if spec == "direct" {
            return Ok(Self {
                kind: ProxyKind::Direct,
                host: String::new(),
                port: 0,
                user: None,
                pass: None,
                unix_path: None,
            });
        }
        // unix:/path or socks5+unix:/path — a SOCKS5 proxy on a unix socket.
        if let Some(path) = spec
            .strip_prefix("unix:")
            .or_else(|| spec.strip_prefix("socks5+unix:"))
            .or_else(|| spec.strip_prefix("socks5+unix://"))
        {
            if path.is_empty() {
                bail!("unix proxy {spec:?} has no path");
            }
            return Ok(Self {
                kind: ProxyKind::Socks5,
                host: String::new(),
                port: 0,
                user: None,
                pass: None,
                unix_path: Some(path.to_string()),
            });
        }
        if let Some(path) = spec
            .strip_prefix("tailscale:")
            .or_else(|| spec.strip_prefix("tailscale://"))
        {
            if path.is_empty() {
                bail!("tailscale proxy {spec:?} has no socket path");
            }
            return Ok(Self {
                kind: ProxyKind::Tailscale,
                host: String::new(),
                port: 0,
                user: None,
                pass: None,
                unix_path: Some(path.to_string()),
            });
        }
        let (scheme, rest) = match spec.split_once("://") {
            Some((s, r)) => (s, r),
            None => ("socks5", spec),
        };
        // socks5://unix:/path or http://unix:/path: a proxy reached over a
        // unix socket (issue 102: http:// gave a parse error of the port).
        if let Some(path) = rest.strip_prefix("unix:") {
            let path = path.trim_start_matches("//");
            if path.is_empty() {
                bail!("the unix proxy {spec:?} has no path");
            }
            let kind = match scheme {
                "socks5" | "socks5h" | "socks" => ProxyKind::Socks5,
                "http" => ProxyKind::Http,
                other => bail!(
                    "{other}:// over a unix socket is not supported; use socks5://unix:/path or http://unix:/path"
                ),
            };
            return Ok(Self {
                kind,
                host: String::new(),
                port: 0,
                user: None,
                pass: None,
                unix_path: Some(path.to_string()),
            });
        }
        let kind = match scheme {
            "socks5" | "socks5h" | "socks" => ProxyKind::Socks5,
            "http" | "https" => ProxyKind::Http,
            other => bail!("unknown proxy scheme {other:?} (socks5:// or http://)"),
        };
        // user:pass@host:port
        let (creds, hostport) = match rest.rsplit_once('@') {
            Some((c, h)) => (Some(c), h),
            None => (None, rest),
        };
        let (user, pass) = match creds {
            Some(c) => match c.split_once(':') {
                Some((u, p)) => (Some(u.to_string()), Some(p.to_string())),
                None => (Some(c.to_string()), None),
            },
            None => (None, None),
        };
        let (host, port) = hostport
            .rsplit_once(':')
            .with_context(|| format!("proxy {spec:?} needs host:port"))?;
        if kind == ProxyKind::Socks5 {
            check_socks5_credentials(user.as_deref(), pass.as_deref())?;
        }
        Ok(Self {
            kind,
            host: host.to_string(),
            port: port.parse().context("proxy port")?,
            user,
            pass,
            unix_path: None,
        })
    }

    pub fn endpoint(&self) -> String {
        if self.kind == ProxyKind::Direct {
            return "direct".to_string();
        }
        match &self.unix_path {
            Some(p) => format!("unix:{p}"),
            None => format!("{}:{}", self.host, self.port),
        }
    }

    /// How the previous hop should address this one when chaining. A unix
    /// socket has no `host:port`, so it is named `unix:<path>` and the hop in
    /// front resolves it (the same convention the tests' proxy uses).
    pub fn chain_addr(&self) -> String {
        self.endpoint()
    }
}

pub use crate::rules::{
    domain_matches, in_cidr, load_nets, select_chain, Choice, Net, Rule, RuleAction, RuleMatch,
    Selected, MAX_NETS,
};

/// TCP to `target` (`host:port`, `[v6]:port`) from this process.
pub async fn connect_direct(target: &str) -> Result<tokio::net::TcpStream> {
    let stream = tokio::net::TcpStream::connect(target)
        .await
        .with_context(|| format!("direct TCP to {target}"))?;
    let _ = stream.set_nodelay(true);
    Ok(stream)
}

pub async fn connect_upstream(proxy: &Proxy, target: &str) -> Result<tokio::net::TcpStream> {
    if proxy.kind == ProxyKind::Direct {
        return connect_direct(target).await;
    }
    let stream = tokio::net::TcpStream::connect(proxy.endpoint())
        .await
        .with_context(|| format!("connect to proxy {}", proxy.endpoint()))?;
    match proxy.kind {
        ProxyKind::Socks5 => socks5_connect(stream, proxy, target).await,
        ProxyKind::Http => http_connect(stream, proxy, target).await,
        ProxyKind::Tailscale => bail!("the tailscale hop is only available on the seccomp backend"),
        ProxyKind::Direct => unreachable!("handled above"),
    }
}

/// A connected upstream: TCP, or a unix socket where TCP is denied.
pub trait AsyncStream: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send {}
impl<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send> AsyncStream for T {}
pub type Upstream = Box<dyn AsyncStream>;

/// Dial the proxy and run its handshake. With more than one `-x`, the hops
/// are **chained**: dial the first, then ask each hop to CONNECT to the next,
/// and finally ask the last for the target. Each intermediate hop sees only the
/// address of the next one.
pub async fn connect_upstream_async(chain: &[Proxy], target: &str) -> Result<Upstream> {
    let (first, rest) = chain.split_first().context("no proxy configured")?;
    if first.kind == ProxyKind::Direct {
        if !rest.is_empty() {
            bail!("direct cannot be chained with a proxy hop");
        }
        return Ok(Box::new(connect_direct(target).await?));
    }
    if first.kind == ProxyKind::Tailscale {
        if !rest.is_empty() {
            bail!("a tailscale hop cannot be chained with a proxy hop");
        }
        let path = first
            .unix_path
            .as_deref()
            .context("tailscale hop has no socket")?;
        let (host, port) = split_target(target)?;
        let conn = crate::tailscale::Dialer::new(path)
            .dial(&host, port)
            .await
            .with_context(|| format!("tailnet dial {target}"))?;
        return Ok(Box::new(conn));
    }
    let mut stream: Upstream = dial_proxy(first).await?;
    if rest.is_empty() {
        return match first.kind {
            ProxyKind::Socks5 => socks5_connect(stream, first, target).await,
            ProxyKind::Http => http_connect(stream, first, target).await,
            ProxyKind::Tailscale | ProxyKind::Direct => unreachable!("handled above"),
        };
    }
    for (i, hop) in chain.iter().enumerate().take(chain.len() - 1) {
        if hop.kind != ProxyKind::Socks5 {
            bail!(
                "chaining needs SOCKS5 hops; {} is {:?}",
                hop.endpoint(),
                hop.kind
            );
        }
        let next = &chain[i + 1];
        stream = socks5_connect(stream, hop, &next.chain_addr()).await?;
    }
    let last = chain.last().unwrap();
    match last.kind {
        ProxyKind::Socks5 => socks5_connect(stream, last, target).await,
        ProxyKind::Http => http_connect(stream, last, target).await,
        ProxyKind::Tailscale => bail!("a tailscale hop must be the only hop"),
        ProxyKind::Direct => bail!("direct must be the only hop"),
    }
}

/// The longest user and password of the SOCKS5 user/password method
/// (RFC 1929): each has a one-byte length.
pub const SOCKS5_CREDENTIAL_MAX: usize = 255;

/// Refuse a SOCKS5 user or password that RFC 1929 cannot carry. Before
/// (issue 304), the length byte was truncated and the auth stream lost
/// its frames.
fn check_socks5_credentials(user: Option<&str>, pass: Option<&str>) -> Result<()> {
    for (what, value) in [("user", user), ("password", pass)] {
        if value.is_some_and(|v| v.len() > SOCKS5_CREDENTIAL_MAX) {
            bail!(
                "the {what} of the SOCKS5 proxy is longer than {SOCKS5_CREDENTIAL_MAX} bytes \
                 (the limit of RFC 1929)"
            );
        }
    }
    Ok(())
}

async fn socks5_handshake<S>(mut stream: S, proxy: &Proxy) -> Result<S>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    // A Proxy of the library can have fields that parse did not check.
    check_socks5_credentials(proxy.user.as_deref(), proxy.pass.as_deref())?;
    let methods: &[u8] = if proxy.user.is_some() {
        &[0x00, 0x02]
    } else {
        &[0x00]
    };
    stream.write_all(&[0x05, methods.len() as u8]).await?;
    stream.write_all(methods).await?;
    let mut resp = [0u8; 2];
    stream.read_exact(&mut resp).await?;
    if resp[0] != 0x05 || resp[1] == 0xff {
        bail!("proxy refused SOCKS5 methods");
    }
    if resp[1] == 0x02 {
        let user = proxy.user.as_deref().unwrap_or("");
        let pass = proxy.pass.as_deref().unwrap_or("");
        stream.write_all(&[0x01, user.len() as u8]).await?;
        stream.write_all(user.as_bytes()).await?;
        stream.write_all(&[pass.len() as u8]).await?;
        stream.write_all(pass.as_bytes()).await?;
        let mut auth = [0u8; 2];
        stream.read_exact(&mut auth).await?;
        if auth[1] != 0x00 {
            bail!("SOCKS5 authentication failed");
        }
    }
    Ok(stream)
}

/// Read an `ATYP`-prefixed address and port from a SOCKS5 reply.
async fn read_socks_addr<S>(stream: &mut S, atyp: u8) -> Result<SocketAddr>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    use tokio::io::AsyncReadExt;
    match atyp {
        0x01 => {
            let mut b = [0u8; 6];
            stream.read_exact(&mut b).await?;
            Ok(SocketAddr::from((
                Ipv4Addr::new(b[0], b[1], b[2], b[3]),
                u16::from_be_bytes([b[4], b[5]]),
            )))
        }
        0x04 => {
            let mut b = [0u8; 18];
            stream.read_exact(&mut b).await?;
            let mut o = [0u8; 16];
            o.copy_from_slice(&b[..16]);
            Ok(SocketAddr::from((
                std::net::Ipv6Addr::from(o),
                u16::from_be_bytes([b[16], b[17]]),
            )))
        }
        0x03 => {
            let mut l = [0u8; 1];
            stream.read_exact(&mut l).await?;
            let mut host = vec![0u8; l[0] as usize];
            stream.read_exact(&mut host).await?;
            let mut p = [0u8; 2];
            stream.read_exact(&mut p).await?;
            let host = String::from_utf8_lossy(&host).to_string();
            let ip: IpAddr = host
                .parse()
                .context("relay returned a name, not an address")?;
            Ok(SocketAddr::new(ip, u16::from_be_bytes(p)))
        }
        other => bail!("bad SOCKS5 address type {other}"),
    }
}

pub(crate) async fn socks5_connect<S>(stream: S, proxy: &Proxy, target: &str) -> Result<S>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    use tokio::io::AsyncWriteExt;
    // A chained hop is addressed as `unix:<path>`; there is no host:port, so
    // the whole string goes as a domain and the port is 0. The hop resolves it.
    let (host, port) = if target.starts_with("unix:") {
        (target.to_string(), 0u16)
    } else {
        split_target(target)?
    };
    // ATYP 3 has a one-byte length (issue 304: it was truncated).
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    if bare.parse::<IpAddr>().is_err() && host.len() > 255 {
        bail!("the target name {host:?} is longer than 255 bytes (the limit of SOCKS5)");
    }
    let mut stream = socks5_handshake(stream, proxy).await?;
    let mut req = vec![0x05, 0x01, 0x00];
    if let Ok(ip) = bare.parse::<Ipv4Addr>() {
        req.push(0x01);
        req.extend_from_slice(&ip.octets());
    } else if let Ok(ip) = bare.parse::<std::net::Ipv6Addr>() {
        req.push(0x04);
        req.extend_from_slice(&ip.octets());
    } else {
        req.push(0x03);
        req.push(host.len() as u8);
        req.extend_from_slice(host.as_bytes());
    }
    req.extend_from_slice(&port.to_be_bytes());
    stream.write_all(&req).await?;
    let mut head = [0u8; 4];
    tokio::io::AsyncReadExt::read_exact(&mut stream, &mut head).await?;
    if head[1] != 0x00 {
        bail!("SOCKS5 connect to {target} failed: code {}", head[1]);
    }
    read_socks_addr(&mut stream, head[3]).await?;
    Ok(stream)
}

/// `UDP ASSOCIATE` (`pod_proxy::udp::associate`): keep the TCP control
/// connection and learn the relay address.
pub async fn socks5_udp_associate<S>(stream: S, proxy: &Proxy) -> Result<(S, SocketAddr)>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let shared = shared_proxy(proxy)?;
    Ok(pod_proxy::udp::associate(stream, APP, &shared).await?)
}

/// Ask for a UDP association in the stream (`pod_proxy::udp::open_in_stream`,
/// the command 0x83, issue 84). An error with `pod_proxy::DialError::UdpRefused`
/// in it: the server does not know the command (the caller can use
/// `UDP ASSOCIATE`).
pub async fn socks5_udp_in_stream<S>(stream: S, proxy: &Proxy) -> Result<S>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let shared = shared_proxy(proxy)?;
    Ok(pod_proxy::udp::open_in_stream(stream, APP, &shared).await?)
}

/// The name of the program in the words of a refusal of `pod-proxy`.
const APP: &str = "podnet";

/// The `pod_proxy::Proxy` of a SOCKS5 proxy: its credentials and its name
/// in a message. `pod-proxy` sends the credentials with no check of their
/// length, so this function checks them first.
fn shared_proxy(proxy: &Proxy) -> Result<pod_proxy::Proxy> {
    check_socks5_credentials(proxy.user.as_deref(), proxy.pass.as_deref())?;
    let spec = match &proxy.unix_path {
        Some(path) => {
            let path = std::path::absolute(path).unwrap_or_else(|_| path.into());
            format!("socks5h+unix:{}", path.display())
        }
        None => {
            let userinfo = match (&proxy.user, &proxy.pass) {
                (None, _) => String::new(),
                (Some(user), pass) => format!(
                    "{}:{}@",
                    percent_encode(user),
                    percent_encode(pass.as_deref().unwrap_or(""))
                ),
            };
            let host = match proxy.host.contains(':') && !proxy.host.starts_with('[') {
                true => format!("[{}]", proxy.host),
                false => proxy.host.clone(),
            };
            format!("socks5h://{userinfo}{host}:{}", proxy.port)
        }
    };
    pod_proxy::Proxy::parse(&spec).map_err(|why| anyhow::anyhow!("the SOCKS5 proxy: {why}"))
}

/// Escape each byte of `text` that is not unreserved (RFC 3986).
fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(char::from(byte))
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

pub(crate) async fn http_connect<S>(mut stream: S, proxy: &Proxy, target: &str) -> Result<S>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut req = format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n");
    if let (Some(u), Some(p)) = (&proxy.user, &proxy.pass) {
        let creds = base64(format!("{u}:{p}").as_bytes());
        req.push_str(&format!("Proxy-Authorization: Basic {creds}\r\n"));
    }
    req.push_str("\r\n");
    stream.write_all(req.as_bytes()).await?;
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    while !buf.ends_with(b"\r\n\r\n") {
        let n = stream.read(&mut byte).await?;
        if n == 0 {
            bail!("proxy closed during CONNECT");
        }
        buf.push(byte[0]);
        if buf.len() > 8192 {
            bail!("proxy CONNECT response too large");
        }
    }
    let text = String::from_utf8_lossy(&buf);
    let status = text
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok());
    if status != Some(200) {
        bail!(
            "proxy CONNECT to {target} failed: {}",
            text.lines().next().unwrap_or("")
        );
    }
    Ok(stream)
}

pub fn split_target(target: &str) -> Result<(String, u16)> {
    let (host, port) = target
        .rsplit_once(':')
        .with_context(|| format!("target {target:?} has no port"))?;
    Ok((host.to_string(), port.parse().context("target port")?))
}

pub fn base64(data: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            T[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

/// Dial the proxy without sending a request; the caller speaks next.
pub async fn dial_proxy(proxy: &Proxy) -> Result<Upstream> {
    match &proxy.unix_path {
        Some(path) => Ok(Box::new(
            tokio::net::UnixStream::connect(path)
                .await
                .with_context(|| format!("connect to unix proxy {path}"))?,
        )),
        None => Ok(Box::new(
            tokio::net::TcpStream::connect(proxy.endpoint())
                .await
                .with_context(|| format!("connect to proxy {}", proxy.endpoint()))?,
        )),
    }
}

#[cfg(test)]
mod tests;
