//! A tunnel through an HTTP CONNECT proxy, on the dial of `pod-proxy` of
//! PodUtils/podcrates (T-473): the one CONNECT client of PodUtils. This
//! module gives its blocking form to the library (`connect_tunnel`) and the
//! proxy and the dialer of one given proxy to the async callers
//! (`podnet ssh-relay --connect-proxy`).
//!
//! The dial of `pod-proxy` reads the head of the answer to its empty line and
//! not one byte further: the first bytes of the tunnel stay in the socket. A
//! refusal keeps the status, the reason phrase of the proxy and the first
//! line of its body. A proxy that asks for Digest gets the Digest answer.

use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

/// The name of the program in the words of a refusal of the proxy.
const APP: &str = "podnet";

/// Connect to `host:port` through the HTTP proxy at `proxy_addr`.
///
/// Returns the raw stream, positioned at the first byte after the CONNECT
/// response headers. The caller is responsible for whatever protocol runs on
/// top (TLS, SSH, ...). The read and write timeouts of the stream are
/// `timeout`.
///
/// # Errors
///
/// Returns an error if the proxy refuses the CONNECT (notably a `403` from an
/// allow-list proxy: [`io::ErrorKind::ConnectionRefused`]), if the response
/// is malformed, or if the header block has no end within 16 KiB.
pub fn connect_tunnel(
    proxy_addr: &str,
    host: &str,
    port: u16,
    timeout: Duration,
) -> io::Result<TunnelStream<TcpStream>> {
    connect_tunnel_with_auth(proxy_addr, host, port, timeout, None)
}

/// Open a CONNECT tunnel, optionally authenticating to the proxy.
///
/// A `Proxy-Authorization: Basic` header is sent when credentials are given,
/// and a Digest answer when the proxy asks for Digest. Proxies that require
/// it answer `407` rather than `403`, and the status is surfaced so a
/// missing credential can be told from a blocked destination.
///
/// The dial runs on a thread of its own: the blocking dial of `pod-proxy`
/// runs a runtime of its own, which a task of a tokio runtime cannot block
/// on.
pub fn connect_tunnel_with_auth(
    proxy_addr: &str,
    host: &str,
    port: u16,
    timeout: Duration,
    credentials: Option<(&str, &str)>,
) -> io::Result<TunnelStream<TcpStream>> {
    let proxy = proxy_of(proxy_addr, credentials)?;
    let dial = || -> io::Result<TcpStream> {
        let settings = pod_proxy::Settings::new(APP, pod_proxy::ProxyChoice::Via(proxy));
        let resolve = Arc::new(pod_proxy::resolve::System::new());
        let dialer = pod_proxy::blocking::Dialer::new(settings, resolve)?;
        dialer.tcp(host, port, timeout).map_err(io_error)
    };
    let stream = std::thread::scope(|scope| scope.spawn(dial).join())
        .map_err(|_| io::Error::other("the thread of the CONNECT dial stopped"))??;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    Ok(TunnelStream {
        stream,
        leftover: Vec::new(),
    })
}

/// The proxy at `proxy_addr` (`HOST:PORT`, or a URL such as
/// `http://HOST:PORT`), with `credentials` when they are given.
///
/// # Errors
///
/// [`io::ErrorKind::InvalidInput`] when `pod-proxy` cannot read the address.
pub fn proxy_of(
    proxy_addr: &str,
    credentials: Option<(&str, &str)>,
) -> io::Result<pod_proxy::Proxy> {
    let spec = match credentials {
        None => proxy_addr.to_string(),
        Some((user, pass)) => {
            let (scheme, rest) = proxy_addr.split_once("://").unwrap_or(("http", proxy_addr));
            format!(
                "{scheme}://{}:{}@{rest}",
                percent_encode(user),
                percent_encode(pass)
            )
        }
    };
    pod_proxy::Proxy::parse(&spec).map_err(|why| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("the proxy {proxy_addr:?}: {why}"),
        )
    })
}

/// The async dialer that reaches each target through `proxy` (its names
/// by the system resolver).
pub fn dialer_via(proxy: pod_proxy::Proxy) -> pod_proxy::Dialer {
    let settings = pod_proxy::Settings::new(APP, pod_proxy::ProxyChoice::Via(proxy));
    pod_proxy::Dialer::new(settings, Arc::new(pod_proxy::resolve::System::new()))
}

/// The error of a dial as an error of I/O: a refusal of the proxy is
/// [`io::ErrorKind::ConnectionRefused`], a step out of time
/// [`io::ErrorKind::TimedOut`]. The text is the one line of `pod-proxy`.
pub fn io_error(e: pod_proxy::DialError) -> io::Error {
    use pod_proxy::DialError;
    let kind = match &e {
        DialError::ProxyRefused { .. } | DialError::ProxyUnreachable { .. } => {
            io::ErrorKind::ConnectionRefused
        }
        DialError::Timeout { .. } => io::ErrorKind::TimedOut,
        DialError::Connect {
            kind: Some(kind), ..
        } => *kind,
        DialError::InvalidTarget(_) | DialError::BadProxy(_) => io::ErrorKind::InvalidInput,
        _ => io::ErrorKind::Other,
    };
    io::Error::new(kind, e.to_string())
}

/// Each byte of `text` that is not unreserved (RFC 3986), as `%XX`.
fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// Standard base64, written out so proxy auth and the WebSocket
/// handshake key need no extra dependency.
pub fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

/// Strip an `http://` or `https://` prefix from a proxy address.
///
/// A CONNECT proxy address is `host:port`; the URL form is what
/// `HTTPS_PROXY` usually holds, so both are accepted.
pub fn strip_scheme(value: String) -> String {
    for scheme in ["http://", "https://"] {
        if let Some(rest) = value.strip_prefix(scheme) {
            return rest.to_string();
        }
    }
    value
}

/// Alias for [`base64_encode`], for callers outside this module.
pub fn base64_encode_pub(input: &[u8]) -> String {
    base64_encode(input)
}

/// A byte pipe to a remote host, established through a CONNECT proxy.
///
/// Generic over the underlying stream. The dial of `pod-proxy` reads no byte
/// past the head of the answer, so `leftover` is empty after a dial; a
/// caller that read past a head of its own gives those bytes back here.
#[derive(Debug)]
pub struct TunnelStream<R = TcpStream> {
    stream: R,
    leftover: Vec<u8>,
}

impl<R> TunnelStream<R> {
    /// The stream with `leftover`, the bytes read past a head, first.
    pub fn new(stream: R, leftover: Vec<u8>) -> Self {
        TunnelStream { stream, leftover }
    }

    /// Return any bytes that were read past the CONNECT response headers.
    pub fn into_parts(self) -> (R, Vec<u8>) {
        (self.stream, self.leftover)
    }
}

/// Read the full `TunnelStream`, including any bytes buffered before the
/// first read on the underlying stream.
impl<R: Read> Read for TunnelStream<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if !self.leftover.is_empty() {
            let n = self.leftover.len().min(buf.len());
            buf[..n].copy_from_slice(&self.leftover[..n]);
            self.leftover.drain(..n);
            return Ok(n);
        }
        self.stream.read(buf)
    }
}

impl<R: Write> Write for TunnelStream<R> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.stream.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}

#[cfg(test)]
mod tests;
