//! One front door over a dialer (T-061): SOCKS5 and HTTP CONNECT (and an
//! `http://` request in absolute form) on one listener, each flow through
//! the [`Dial`] of the door. The parser is the one of the pod
//! (`podnet_netns::socks`). The door has one deadline for the request of a
//! client (the first byte, the SOCKS5 request, the CONNECT head) and a cap
//! of the live clients (GitHub #384).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::socks::{self, HttpRequest, ProxyHost, ProxyRequest};
use anyhow::{Context, Result};
use base64::Engine as _;
use pod_core::{Dial, Target};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};
use tokio::sync::Semaphore;
use tokio::time::Instant;

mod udp;

/// The deadline of the request of a client (`REQUEST_LIMIT` of the WARP door).
pub const REQUEST_LIMIT: Duration = Duration::from_secs(30);
/// The default cap of the live clients of one door.
pub const CLIENT_LIMIT: usize = 1024;
/// The deadline of the dial of one flow.
pub const DIAL_LIMIT: Duration = Duration::from_secs(30);

/// A check of a user and a password of a client.
pub type Users = Arc<dyn Fn(&str, &str) -> bool + Send + Sync>;

/// A front door over `dial`.
pub struct Door {
    dial: Arc<dyn Dial>,
    users: Option<Users>,
    request_limit: Duration,
    clients: Arc<Semaphore>,
    turned_away: AtomicU64,
    udp_bind: std::net::IpAddr,
}

impl Door {
    pub fn new(dial: Arc<dyn Dial>) -> Door {
        Door {
            dial,
            users: None,
            request_limit: REQUEST_LIMIT,
            clients: Arc::new(Semaphore::new(CLIENT_LIMIT)),
            turned_away: AtomicU64::new(0),
            udp_bind: std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
        }
    }

    /// The address of the UDP socket of a UDP ASSOCIATE (T-098). The default
    /// is the loopback address: for a door on a TCP address, give that
    /// address, so that a client of another host can send its datagrams.
    pub fn with_udp_bind(mut self, ip: std::net::IpAddr) -> Door {
        self.udp_bind = ip;
        self
    }

    /// Ask each client for a user and a password (SOCKS5 RFC 1929, HTTP
    /// `Proxy-Authorization: Basic`).
    pub fn with_users(mut self, users: Users) -> Door {
        self.users = Some(users);
        self
    }

    /// The cap of the live clients.
    pub fn with_client_limit(mut self, limit: usize) -> Door {
        self.clients = Arc::new(Semaphore::new(limit.max(1)));
        self
    }

    /// The deadline of the request of a client.
    pub fn with_request_limit(mut self, limit: Duration) -> Door {
        self.request_limit = limit;
        self
    }

    /// The clients that the cap turned away.
    pub fn turned_away(&self) -> u64 {
        self.turned_away.load(Ordering::Relaxed)
    }

    /// Serve each client of `listener` until it fails.
    pub async fn serve_unix(self: Arc<Self>, listener: tokio::net::UnixListener) -> Result<()> {
        loop {
            let (stream, _) = listener.accept().await.context("the door listener")?;
            self.clone().spawn(stream);
        }
    }

    /// Serve each client of `listener` until it fails.
    pub async fn serve_tcp(self: Arc<Self>, listener: tokio::net::TcpListener) -> Result<()> {
        loop {
            let (stream, _) = listener.accept().await.context("the door listener")?;
            self.clone().spawn(stream);
        }
    }

    /// Serve one client on a task, in the cap of the live clients.
    pub fn spawn<S>(self: Arc<Self>, stream: S)
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let Ok(permit) = self.clients.clone().try_acquire_owned() else {
            self.turned_away.fetch_add(1, Ordering::Relaxed);
            tracing::debug!("the door turned a client away: the cap of the live clients");
            return;
        };
        tokio::spawn(async move {
            if let Err(e) = self.handle(stream).await {
                tracing::debug!("a client of the door: {e:#}");
            }
            drop(permit);
        });
    }

    /// One client: its request in the deadline, the dial, then the copy.
    pub async fn handle<S>(&self, mut stream: S) -> Result<()>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send,
    {
        let mut first = [0u8; 1];
        let request = tokio::time::timeout(self.request_limit, async {
            stream
                .read_exact(&mut first)
                .await
                .context("the first byte")?;
            let mut prefixed = Prefixed::new(first[0], &mut stream);
            self.read_request(&mut prefixed).await
        })
        .await
        .map_err(|_| anyhow::anyhow!("the client sent no request in {:?}", self.request_limit))??;
        match request {
            Request::Socks(req) => {
                let target = target_of(&req);
                match self.dial.dial(&target, Instant::now() + DIAL_LIMIT).await {
                    Ok(mut conn) => {
                        socks::socks5_reply(&mut stream, 0, conn.local).await?;
                        let _ = tokio::io::copy_bidirectional(&mut stream, &mut conn.io).await;
                    }
                    Err(e) => {
                        socks::socks5_reply(
                            &mut stream,
                            socks::socks5_code_for_error(&e.to_string()),
                            None,
                        )
                        .await?;
                    }
                }
            }
            Request::SocksUdp => udp::associate(self, &mut stream).await?,
            Request::Socks5Unsupported => {
                // BIND: the command is not supported.
                socks::socks5_reply(&mut stream, 0x07, None).await?;
            }
            Request::Http(HttpRequest::Connect(req)) => {
                let target = target_of(&req);
                match self.dial.dial(&target, Instant::now() + DIAL_LIMIT).await {
                    Ok(mut conn) => {
                        socks::http_connect_ok(&mut stream).await?;
                        let _ = tokio::io::copy_bidirectional(&mut stream, &mut conn.io).await;
                    }
                    Err(e) => {
                        socks::http_connect_reply(
                            &mut stream,
                            502,
                            &e.text().replace(['\r', '\n'], " "),
                        )
                        .await?
                    }
                }
            }
            Request::Http(HttpRequest::Forward { request, head }) => {
                let target = target_of(&request);
                match self
                    .dial
                    .dial_http(&target, Instant::now() + DIAL_LIMIT)
                    .await
                {
                    Ok(mut conn) => {
                        conn.io.write_all(&head).await?;
                        let _ = tokio::io::copy_bidirectional(&mut stream, &mut conn.io).await;
                    }
                    Err(e) => {
                        socks::http_connect_reply(
                            &mut stream,
                            502,
                            &e.text().replace(['\r', '\n'], " "),
                        )
                        .await?
                    }
                }
            }
            Request::Refused => {}
        }
        Ok(())
    }

    async fn read_request<S>(&self, stream: &mut Prefixed<'_, S>) -> Result<Request>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send,
    {
        let users = self.users.clone();
        if socks::looks_like_socks5(stream.first()) {
            let check = users.as_ref().map(|u| u.as_ref() as socks::CheckUser<'_>);
            let (command, req) = socks::socks5_read_command_auth(stream, check).await?;
            return Ok(match command {
                1 => Request::Socks(req),
                3 => Request::SocksUdp,
                _ => Request::Socks5Unsupported,
            });
        }
        let (request, auth) = socks::http_read_request_auth(stream).await?;
        if let Some(users) = users {
            if !basic_ok(auth.as_deref(), users.as_ref()) {
                let head = "HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"podnet\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                stream.write_all(head.as_bytes()).await?;
                return Ok(Request::Refused);
            }
        }
        Ok(Request::Http(request))
    }
}

enum Request {
    Socks(ProxyRequest),
    /// UDP ASSOCIATE; the address of the request is the expected source of
    /// the client, which the relay does not need (it pins the first source).
    SocksUdp,
    Socks5Unsupported,
    Http(HttpRequest),
    Refused,
}

/// The target of a request of a client: a name stays a name (the dialer
/// resolves it where its way says).
fn target_of(req: &ProxyRequest) -> Target {
    match &req.host {
        ProxyHost::Ip(ip) => Target::Addr(std::net::SocketAddr::new(*ip, req.port)),
        ProxyHost::Domain(name) => Target::Name {
            host: name.clone(),
            port: req.port,
        },
    }
}

/// True when `value` is `Basic BASE64(user:password)` of a user that
/// `users` takes.
fn basic_ok(value: Option<&str>, users: &(dyn Fn(&str, &str) -> bool + Send + Sync)) -> bool {
    let Some(encoded) = value.and_then(|v| v.strip_prefix("Basic ")) else {
        return false;
    };
    let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(encoded.trim()) else {
        return false;
    };
    let Ok(text) = String::from_utf8(decoded) else {
        return false;
    };
    match text.split_once(':') {
        Some((user, password)) => users(user, password),
        None => false,
    }
}

/// A stream with its first byte back in front.
struct Prefixed<'a, S> {
    first: Option<u8>,
    byte: u8,
    inner: &'a mut S,
}

impl<'a, S> Prefixed<'a, S> {
    fn new(byte: u8, inner: &'a mut S) -> Self {
        Prefixed {
            first: Some(byte),
            byte,
            inner,
        }
    }

    fn first(&self) -> u8 {
        self.byte
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for Prefixed<'_, S> {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let this = self.get_mut();
        if let Some(byte) = this.first.take() {
            if buf.remaining() > 0 {
                buf.put_slice(&[byte]);
                return std::task::Poll::Ready(Ok(()));
            }
            this.first = Some(byte);
        }
        std::pin::Pin::new(&mut *this.inner).poll_read(cx, buf)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Prefixed<'_, S> {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        data: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        std::pin::Pin::new(&mut *self.get_mut().inner).poll_write(cx, data)
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut *self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut *self.get_mut().inner).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests;
