//! Expose a unix-socket origin on the public internet over SSH port forwarding.
//!
//! # Why this exists
//!
//! The development sandbox has three properties that together rule out every
//! stock tunnel client:
//!
//! 1. `bind()` on a TCP listener returns `EACCES`, so the tool cannot listen
//!    locally and wait to be connected to. A unix socket can be bound, so the
//!    origin is a unix socket rather than a TCP port.
//! 2. `connect()` to any TCP destination returns `EPERM`, including loopback.
//!    So even if a listener existed, no tool could reach it over TCP. A unix
//!    socket is reached with `connect()` on `AF_UNIX`, which is permitted.
//! 3. Egress is only an HTTP CONNECT proxy on an allow-list. The Cloudflare
//!    tunnel edge port (7844) is not on the list; TCP port 443 to an SSH relay
//!    is.
//!
//! An SSH `forwarded-tcpip` channel is a raw byte pipe that the relay opens
//! per visitor. podnet splices that pipe to the unix socket, so the blocked TCP
//! path is never used. That is the whole trick, and it is why the origin can be
//! a unix socket at all.

use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use russh::client::{Config, Handle, Handler, Msg, Session};
use russh::keys::ssh_key::rand_core::UnwrapErr;
use russh::keys::ssh_key::Algorithm;
use russh::keys::PrivateKey;
use russh::keys::PrivateKeyWithHashAlg;
use russh::{Channel, ChannelMsg};
use ssh_key::getrandom::SysRng;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::UnixStream;

use podnet_core::connect;

pub mod auth;
pub mod facts;
pub mod probe;
pub mod supervise;

pub use facts::{Banner, UrlRule};

/// Default relay, reachable on port 443 through the CONNECT proxy: the
/// endpoint `ssh` of `pinggy` (`pod-services`).
pub fn default_relay() -> String {
    podnet_core::services::endpoint("pinggy", "ssh").authority()
}

/// Where the remote forward is requested, and where the origin lives.
#[derive(Debug, Clone)]
pub struct RelayConfig {
    /// `host:port` of the SSH server.
    pub address: String,
    /// More `host:port` of the same relay, tried in order when the transport
    /// to `address` fails (the plain SSH endpoints of the facts).
    pub fallbacks: Vec<String>,
    /// `host:port` of the HTTP CONNECT proxy, or `None` for a direct connect.
    pub proxy: Option<String>,
    /// SSH user. The free relay tier takes an anonymous login.
    pub username: String,
    /// The port that the relay is asked to open for the forward. `None`:
    /// port 0, and the relay chooses. A relay can refuse port 0 (Sandhole).
    pub remote_port: Option<u16>,
    /// Path to the unix socket that serves the origin.
    pub unix_socket: std::path::PathBuf,
    /// Timeout for the CONNECT proxy handshake.
    pub connect_timeout: Duration,
    /// Which URL of the banner of the relay is the tunnel URL.
    pub url_rule: UrlRule,
    /// The SSH session ends after this time with no data from the relay.
    /// `None`: it does not end for that reason.
    pub inactivity: Option<Duration>,
    /// A keepalive request goes to the relay at this interval, and the
    /// session ends after more than three unanswered requests. `None`: no keepalive.
    pub keepalive: Option<Duration>,
}

impl Default for RelayConfig {
    fn default() -> Self {
        Self {
            address: default_relay(),
            fallbacks: Vec::new(),
            proxy: None,
            username: "podnet".to_string(),
            remote_port: None,
            unix_socket: default_origin_socket(),
            connect_timeout: Duration::from_secs(15),
            url_rule: facts::default_rule(),
            inactivity: Some(Duration::from_secs(3600)),
            keepalive: None,
        }
    }
}

/// The default socket of the origin (#230): `origin.sock` in a private
/// directory of this user (`state::exec_dir`, mode 0700), not a fixed path
/// in the shared `/tmp` that another user can make first.
pub fn default_origin_socket() -> std::path::PathBuf {
    match podnet_core::state::exec_dir("podnet-relay") {
        Ok(dir) => dir.join("origin.sock"),
        Err(_) => {
            std::env::temp_dir().join(format!("podnet-origin-{}.sock", unsafe { libc::geteuid() }))
        }
    }
}

/// What a successful run reports back to the caller.
#[derive(Debug, Clone)]
pub struct TunnelHandle {
    /// The public URL a visitor should open.
    pub url: String,
    /// The port the relay allocated for the remote forward.
    pub remote_port: u32,
}

/// Errors that can stop a relay from being established.
#[derive(Debug)]
pub enum RelayError {
    /// The `host:port` string could not be split.
    BadAddress(String),
    /// The SSH key could not be generated.
    Key(String),
    /// The TCP/CONNECT transport could not be opened.
    Transport(String),
    /// The SSH handshake or authentication failed.
    Ssh(String),
    /// The relay accepted us but never published a URL.
    NoUrl,
}

impl std::fmt::Display for RelayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RelayError::BadAddress(a) => {
                write!(f, "expected relay address as host:port, got {a:?}")
            }
            RelayError::Key(e) => write!(f, "could not prepare an SSH client key: {e}"),
            RelayError::Transport(e) => write!(f, "could not open a transport to the relay: {e}"),
            RelayError::Ssh(e) => write!(f, "SSH session failed: {e}"),
            RelayError::NoUrl => write!(f, "relay did not publish a URL within the timeout"),
        }
    }
}

impl std::error::Error for RelayError {}

/// SSH client handler. Each visitor's channel is served the same way, so this
/// only needs to know where the origin is and a counter for the final report.
pub struct Client {
    /// The unix socket every forwarded channel is spliced to.
    origin: std::path::PathBuf,
    /// Count of channels successfully spliced, for the final report.
    served: Arc<Mutex<u64>>,
}

impl Client {
    /// Build a handler bound to one origin socket.
    pub fn new(origin: std::path::PathBuf, served: Arc<Mutex<u64>>) -> Self {
        Self { origin, served }
    }
}

impl Handler for Client {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _server_public_key: &russh::keys::PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        // The relay is reached over an anonymous, ephemeral free-tier tunnel.
        // There is no known-host store to check against, and pinning one would
        // make the tool fail on the relay's routine key rotation. The traffic
        // this carries is only as sensitive as the public URL itself, which is
        // unguessable and already in the open.
        //
        // If this ever fronts something that must not be read by the relay, put
        // a known-hosts check here and refuse unknown keys.
        Ok(true)
    }

    /// A visitor connected to the published port. Accept the channel, then
    /// pump it to the unix-socket origin on its own task.
    ///
    /// The splice is spawned rather than awaited here on purpose: this callback
    /// runs on the session's event loop, and holding it for the lifetime of a
    /// visitor's request would stall every other channel.
    async fn server_channel_open_forwarded_tcpip(
        &mut self,
        channel: Channel<Msg>,
        connected_address: &str,
        connected_port: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: russh::client::ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        let origin = self.origin.clone();
        let served = self.served.clone();
        let peer = format!("{connected_address}:{connected_port}");

        if std::env::var_os("PODNET_DEBUG").is_some() {
            eprintln!("podnet: visitor channel opened from {peer} -> {origin:?}");
        }

        reply.accept().await;

        tokio::spawn(async move {
            match splice_unix_socket(channel, &origin).await {
                Ok(()) => {
                    if let Ok(mut n) = served.lock() {
                        *n += 1;
                    }
                }
                Err(e) => {
                    eprintln!("podnet: visitor {peer} failed: {e}");
                }
            }
            if std::env::var_os("PODNET_DEBUG").is_some() {
                eprintln!("podnet: visitor {peer} channel closed");
            }
        });

        Ok(())
    }
}

/// A TCP stream that yields `prefix` before reading from the inner stream.
///
/// The CONNECT response is read synchronously, so any bytes the proxy
/// coalesced past the header terminator must be replayed before the SSH
/// banner, or the handshake sees a truncated stream.
pub struct PrefixedStream {
    prefix: Vec<u8>,
    offset: usize,
    inner: tokio::net::TcpStream,
}

impl PrefixedStream {
    /// Wrap `inner`, replaying `prefix` before any read from it.
    pub fn new(prefix: Vec<u8>, inner: tokio::net::TcpStream) -> Self {
        Self {
            prefix,
            offset: 0,
            inner,
        }
    }
}

impl AsyncRead for PrefixedStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        if this.offset < this.prefix.len() {
            let remaining = this.prefix.len() - this.offset;
            let n = remaining.min(buf.remaining());
            buf.put_slice(&this.prefix[this.offset..this.offset + n]);
            this.offset += n;
            return Poll::Ready(Ok(()));
        }
        Pin::new(&mut this.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for PrefixedStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.get_mut().inner).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

/// Copy bytes between an SSH channel and a unix socket until the origin closes.
///
/// # Why this is not a simple `try_join`
///
/// The visitor's request ends quickly (a GET carries no body, so the relay
/// signals EOF as soon as the request is sent). A naive `try_join!` of the two
/// directions returns the moment the request leg finishes and would then cancel
/// the response leg before the origin's reply is ever forwarded. So the request
/// is copied first, and only after the relay closes its sending side do we read
/// the origin's response back. That is the same half-close ordering a browser
/// uses for keep-alive requests.
async fn splice_unix_socket(
    channel: Channel<Msg>,
    origin: &std::path::Path,
) -> std::io::Result<()> {
    let stream = UnixStream::connect(origin).await?;
    let (mut origin_reader, mut origin_writer) = tokio::io::split(stream);
    let (mut reader, mut writer) = tokio::io::split(channel.into_stream());
    copy_bidirectional(
        &mut reader,
        &mut writer,
        &mut origin_reader,
        &mut origin_writer,
    )
    .await
}

/// Copy a visitor's channel to a unix-socket origin and back, on one connection.
///
/// The two halves share a single origin connection and run to completion
/// together. That combination is what the live tunnel needs:
/// * one connection, so the reply goes back on the socket that carried the
///   request (splitting them sent the reply to a socket that had seen nothing);
/// * `try_join!`, so the response half is not abandoned when the request half
///   finishes first, which is what a visitor's empty-body GET causes.
#[allow(clippy::too_many_arguments)]
async fn copy_bidirectional<R, W, OR, OW>(
    channel_reader: &mut R,
    channel_writer: &mut W,
    origin_reader: &mut OR,
    origin_writer: &mut OW,
) -> std::io::Result<()>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
    OR: tokio::io::AsyncRead + Unpin,
    OW: tokio::io::AsyncWrite + Unpin,
{
    let debug = std::env::var_os("PODNET_DEBUG").is_some();

    // Both directions run concurrently to completion.
    let up = async {
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            let n = channel_reader.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            if debug {
                eprintln!(
                    "podnet: channel->origin {n} bytes: {:?}",
                    String::from_utf8_lossy(&buf[..n.min(160)])
                );
            }
            origin_writer.write_all(&buf[..n]).await?;
        }
        // Signal end-of-request so the origin writes its response.
        origin_writer.shutdown().await?;
        Ok::<(), std::io::Error>(())
    };

    let down = async {
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            let n = origin_reader.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            if debug {
                eprintln!(
                    "podnet: origin->channel {n} bytes: {:?}",
                    String::from_utf8_lossy(&buf[..n.min(160)])
                );
            }
            channel_writer.write_all(&buf[..n]).await?;
        }
        channel_writer.shutdown().await?;
        Ok::<(), std::io::Error>(())
    };

    // `try_join` waits for BOTH legs; the response half is not abandoned when
    // the request half finishes first.
    tokio::try_join!(up, down)?;
    Ok(())
}

/// Open an SSH transport to the relay, honouring the proxy setting. It tries
/// `address`, then each of `fallbacks`, in order; the error is the one of
/// the last address. A bad address is an error at once.
async fn connect_transport(config: &RelayConfig) -> Result<PrefixedStream, RelayError> {
    let all = std::iter::once(&config.address).chain(&config.fallbacks);
    let mut last = None;
    for address in all {
        match connect_address(config, address).await {
            Ok(stream) => return Ok(stream),
            Err(e @ RelayError::BadAddress(_)) => return Err(e),
            Err(e) => {
                tracing::debug!(%address, error = %e, "the relay address did not answer");
                last = Some(e);
            }
        }
    }
    Err(last.unwrap_or_else(|| RelayError::BadAddress(config.address.clone())))
}

/// Open an SSH transport to one address of the relay.
async fn connect_address(
    config: &RelayConfig,
    address: &str,
) -> Result<PrefixedStream, RelayError> {
    let (host, port) = split_address(address)?;

    let Some(proxy_addr) = &config.proxy else {
        let stream = tokio::net::TcpStream::connect((host.as_str(), port))
            .await
            .map_err(|e| RelayError::Transport(e.to_string()))?;
        return Ok(PrefixedStream::new(Vec::new(), stream));
    };

    // `connect::connect_tunnel` is blocking by design (the provisioning path uses
    // it synchronously too), so keep it off the async worker.
    let proxy_addr = proxy_addr.clone();
    let host = host.clone();
    let timeout = config.connect_timeout;

    let (stream, leftover) = tokio::task::spawn_blocking(move || {
        connect::connect_tunnel(&proxy_addr, &host, port, timeout)
    })
    .await
    .map_err(|e| RelayError::Transport(format!("proxy task failed: {e}")))?
    .map_err(|e| RelayError::Transport(e.to_string()))?
    .into_parts();

    stream
        .set_nonblocking(true)
        .map_err(|e| RelayError::Transport(e.to_string()))?;
    let stream = tokio::net::TcpStream::from_std(stream)
        .map_err(|e| RelayError::Transport(e.to_string()))?;

    Ok(PrefixedStream::new(leftover, stream))
}

/// Split `host:port`, rejecting anything else.
fn split_address(address: &str) -> Result<(String, u16), RelayError> {
    let (host, port) = address
        .rsplit_once(':')
        .ok_or_else(|| RelayError::BadAddress(address.to_string()))?;
    let port = port
        .parse::<u16>()
        .map_err(|_| RelayError::BadAddress(address.to_string()))?;
    if host.is_empty() {
        return Err(RelayError::BadAddress(address.to_string()));
    }
    Ok((host.to_string(), port))
}

/// Check that `address` is `host:port`.
pub fn check_address(address: &str) -> Result<(), RelayError> {
    split_address(address).map(|_| ())
}

/// A live session of the relay: the URL, the SSH handle and the banner channel.
///
/// The session ends when the relay closes the connection or the banner
/// channel. Dropping the value closes the tunnel.
pub struct Live {
    /// The public URL and the remote port.
    pub tunnel: TunnelHandle,
    /// The SSH connection.
    pub handle: Handle<Client>,
    /// The session channel that carries the banner of the relay.
    pub banner: Channel<Msg>,
}

/// Establish the tunnel and return the public URL plus a live session handle.
///
/// The returned handle owns the session's event loop; dropping it closes the
/// tunnel.
pub async fn run(config: RelayConfig) -> Result<(TunnelHandle, Handle<Client>), RelayError> {
    let session = open(&config).await?;
    Ok((session.tunnel, session.handle))
}

/// What the next message of the banner channel does to the [`Banner`].
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Step {
    /// No URL yet: read the next message.
    More,
    /// The rule accepted this URL.
    Url(String),
    /// The channel ended with no URL.
    End,
}

/// Give a message of the session channel to the banner reader. A URL that
/// crosses two frames is whole: the [`Banner`] holds the word that a frame
/// cut. `None`: the channel is gone.
pub(crate) fn feed(banner: &mut Banner, msg: Option<ChannelMsg>) -> Step {
    match msg {
        Some(ChannelMsg::Data { data }) => {
            eprint!("  [relay] {}", String::from_utf8_lossy(&data));
            banner
                .push(&data)
                .map_or(Step::More, |u| Step::Url(u.to_string()))
        }
        Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) | None => banner
            .finish()
            .map_or(Step::End, |u| Step::Url(u.to_string())),
        _ => Step::More,
    }
}

/// Establish the tunnel and return the live [`Live`], with the banner
/// channel. [`supervise::supervise`] watches it.
pub async fn open(config: &RelayConfig) -> Result<Live, RelayError> {
    let stream = connect_transport(config).await?;

    // Generate an ephemeral Ed25519 key per run from the OS entropy source;
    // it is never persisted. A free tier takes any key without registering it.
    let private_key = Arc::new(
        PrivateKey::random(&mut UnwrapErr(SysRng), Algorithm::Ed25519)
            .map_err(|e| RelayError::Key(e.to_string()))?,
    );

    let ssh_config = Config {
        inactivity_timeout: config.inactivity,
        keepalive_interval: config.keepalive,
        preferred: russh::Preferred::DEFAULT,
        ..Config::default()
    };
    let ssh_config = Arc::new(ssh_config);

    let served = Arc::new(Mutex::new(0u64));
    let client = Client::new(config.unix_socket.clone(), served);

    let mut handle = russh::client::connect_stream(Arc::clone(&ssh_config), stream, client)
        .await
        .map_err(|e| RelayError::Ssh(format!("handshake failed: {e}")))?;

    // Try the methods in order (see `auth`): the ephemeral key, no key, and
    // an empty keyboard-interactive answer. The free tier takes one of them.
    match auth::authenticate(&mut handle, &config.username, Arc::clone(&private_key))
        .await
        .map_err(|e| RelayError::Ssh(format!("auth failed: {e}")))?
    {
        auth::Login::In(_) => {}
        auth::Login::Refused(why) => {
            return Err(RelayError::Ssh(format!(
                "the relay rejected the user {:?}: {why}",
                config.username
            )));
        }
    }

    // Ask for the remote port of the config. Port 0 asks for an ephemeral
    // port: the relay decides the number and, with it, the public URL.
    let asked = u32::from(config.remote_port.unwrap_or(0));
    let assigned = handle
        .tcpip_forward("0.0.0.0", asked)
        .await
        .map_err(|e| RelayError::Ssh(format!("remote forward refused: {e}")))?;
    // The reply names a port only for a request of port 0.
    let remote_port = if asked == 0 { assigned } else { asked };

    // The relay publishes the public URL on a session channel's data, not on
    // the forwarded-tcpip channels a visitor opens. Open a session channel and
    // request a shell so the relay writes its banner to us, then read from that
    // channel to recover the URL.
    let mut session_channel = handle
        .channel_open_session()
        .await
        .map_err(|e| RelayError::Ssh(format!("session channel refused: {e}")))?;
    session_channel
        .request_shell(true)
        .await
        .map_err(|e| RelayError::Ssh(format!("shell request failed: {e}")))?;

    // Drain the shell channel for the URL. The relay prints several URLs: an
    // upgrade pitch pointing at its dashboard, then the real tunnel URL(s).
    // The rule skips the dashboards and holds the tunnel domain.
    let mut banner = Banner::new(config.url_rule.clone());
    let url = loop {
        let msg = tokio::time::timeout(Duration::from_secs(45), session_channel.wait())
            .await
            .map_err(|_| RelayError::NoUrl)?;
        match feed(&mut banner, msg) {
            Step::More => continue,
            Step::Url(url) => break url,
            Step::End => return Err(RelayError::NoUrl),
        }
    };

    Ok(Live {
        tunnel: TunnelHandle { url, remote_port },
        handle,
        banner: session_channel,
    })
}

#[cfg(test)]
mod auth_tests;
#[cfg(test)]
mod tests;
