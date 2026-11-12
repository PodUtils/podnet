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
use pod_core::{BoxFuture, Class, Dial, Failure, Target};
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

/// The buffer of each direction of a flow (T-151).
const RELAY_BUFFER: usize = 64 * 1024;

/// The most bytes that the door holds for a client whose dial is not done
/// (`--early-reply`). The read of the client stops when the hold is full.
pub const EARLY_HOLD: usize = 64 * 1024;

/// The body of the 405 answer of a door that takes `CONNECT` only.
const CONNECT_ONLY: &str =
    "this proxy speaks HTTP CONNECT only: use it as https_proxy, or as socks5h://\n";

/// The control stream of a client, as a trait object.
pub trait Duplex: AsyncRead + AsyncWrite + Unpin + Send {}

impl<T: AsyncRead + AsyncWrite + Unpin + Send> Duplex for T {}

/// What a command of a door adds to the way: a SOCKS5 command that the
/// dialer cannot carry (UDP ASSOCIATE through the association of an upstream
/// proxy, UDP in the stream).
pub trait Extension: Send + Sync {
    /// Serve `command` on `control` and return true, or return false: the
    /// door then serves UDP ASSOCIATE over the datagrams of its dialer, and
    /// answers "command not supported" to any other command. `control` has
    /// the request read already.
    fn command<'a>(
        &'a self,
        command: u8,
        control: &'a mut dyn Duplex,
    ) -> BoxFuture<'a, Result<bool>>;
}

/// The local rule check of `--early-reply`: true when the rules take the
/// target, so that the door can answer before the dial.
pub type Admit = Arc<dyn Fn(&Target) -> bool + Send + Sync>;

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
    default_port: u16,
    flow_limit: Option<Duration>,
    connect_only: bool,
    extension: Option<Arc<dyn Extension>>,
    early: Option<Admit>,
    on_error: Option<ErrorHook>,
}

/// A function that gets the error of a client of a door.
pub type ErrorHook = Arc<dyn Fn(&anyhow::Error) + Send + Sync>;

impl Door {
    pub fn new(dial: Arc<dyn Dial>) -> Door {
        Door {
            dial,
            users: None,
            request_limit: REQUEST_LIMIT,
            clients: Arc::new(Semaphore::new(CLIENT_LIMIT)),
            turned_away: AtomicU64::new(0),
            udp_bind: std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
            default_port: 0,
            flow_limit: None,
            connect_only: false,
            extension: None,
            early: None,
            on_error: None,
        }
    }

    /// Call `hook` with the error of each client that ends in an error (a
    /// bad request, a client that is too slow, a failed copy of a reply).
    pub fn on_error(mut self, hook: ErrorHook) -> Door {
        self.on_error = Some(hook);
        self
    }

    /// The port of a request that has port 0 (the option `--port`).
    pub fn with_default_port(mut self, port: u16) -> Door {
        self.default_port = port;
        self
    }

    /// End each flow after `limit`, also when it is busy. Without it, a flow
    /// lives until a side closes.
    pub fn with_flow_limit(mut self, limit: Duration) -> Door {
        self.flow_limit = Some(limit);
        self
    }

    /// Take `CONNECT` and SOCKS5 only: a plain `http://` request gets 405.
    pub fn connect_only(mut self) -> Door {
        self.connect_only = true;
        self
    }

    /// Give the commands of SOCKS5 that the dialer cannot carry to
    /// `extension`.
    pub fn with_extension(mut self, extension: Arc<dyn Extension>) -> Door {
        self.extension = Some(extension);
        self
    }

    /// Answer a CONNECT before the dial (T-237, `--early-reply`) when
    /// `admit` takes the target. The door then holds the first bytes of the
    /// client (at most [`EARLY_HOLD`]) until the dial is done, and closes the
    /// connection when the dial fails: the client gets no error code. A
    /// target that `admit` refuses gets its real answer, after the dial
    /// call, as with no option.
    pub fn with_early_reply(mut self, admit: Admit) -> Door {
        self.early = Some(admit);
        self
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
                if let Some(hook) = &self.on_error {
                    hook(&e);
                }
            }
            drop(permit);
        });
    }

    /// One client: its request in the deadline, the dial, then the copy.
    /// A client that is too slow gets SOCKS5 4 or HTTP 504.
    pub async fn handle<S>(&self, mut stream: S) -> Result<()>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send,
    {
        let mut first = [0u8; 1];
        let mut seen = false;
        let request = tokio::time::timeout(self.request_limit, async {
            stream
                .read_exact(&mut first)
                .await
                .context("the first byte")?;
            seen = true;
            let mut prefixed = Prefixed::new(first[0], &mut stream);
            self.read_request(&mut prefixed).await
        })
        .await;
        let request = match request {
            Ok(request) => request?,
            Err(_) => {
                let is_socks = seen && socks::looks_like_socks5(first[0]);
                // The answer is one write, tried once: a client that reads
                // nothing must not hold this task after its deadline.
                let _ = tokio::time::timeout(
                    Duration::ZERO,
                    crate::opening::refuse_slow(&mut stream, is_socks),
                )
                .await;
                anyhow::bail!("the client sent no request in {:?}", self.request_limit);
            }
        };
        match request {
            Request::Socks(req) => {
                let target = self.target_of(&req);
                if self.admits_early(&target) {
                    return self.dial_early(&mut stream, &target, true).await;
                }
                match self.dial.dial(&target, Instant::now() + DIAL_LIMIT).await {
                    Ok(mut conn) => {
                        socks::socks5_reply(&mut stream, 0, conn.local).await?;
                        self.splice(&mut stream, &mut conn.io).await;
                    }
                    Err(e) => {
                        socks::socks5_reply(&mut stream, socks_code(&e), None).await?;
                    }
                }
            }
            Request::Command(command) => {
                let served = match &self.extension {
                    Some(extension) => extension.command(command, &mut stream).await?,
                    None => false,
                };
                if !served {
                    if command == 3 {
                        udp::associate(self, &mut stream).await?;
                    } else {
                        // BIND, or a command that nobody serves.
                        socks::socks5_reply(&mut stream, 0x07, None).await?;
                    }
                }
            }
            Request::Http(HttpRequest::Connect(req)) => {
                let target = self.target_of(&req);
                if self.admits_early(&target) {
                    return self.dial_early(&mut stream, &target, false).await;
                }
                match self.dial.dial(&target, Instant::now() + DIAL_LIMIT).await {
                    Ok(mut conn) => {
                        socks::http_connect_ok(&mut stream).await?;
                        self.splice(&mut stream, &mut conn.io).await;
                    }
                    Err(e) => http_failure(&mut stream, &e).await?,
                }
            }
            Request::Http(HttpRequest::Forward { .. }) if self.connect_only => {
                socks::http_reply_with_body(&mut stream, 405, "Method Not Allowed", CONNECT_ONLY)
                    .await?;
            }
            Request::Http(HttpRequest::Forward { request, head }) => {
                let target = self.target_of(&request);
                match self
                    .dial
                    .dial_http(&target, Instant::now() + DIAL_LIMIT)
                    .await
                {
                    Ok(mut conn) => {
                        conn.io.write_all(&head).await?;
                        self.splice(&mut stream, &mut conn.io).await;
                    }
                    Err(e) => http_failure(&mut stream, &e).await?,
                }
            }
            Request::Refused => {}
        }
        Ok(())
    }

    /// True when the option is on and the rules take `target`.
    fn admits_early(&self, target: &Target) -> bool {
        self.early.as_ref().is_some_and(|admit| admit(target))
    }

    /// `--early-reply`: answer the client first (SOCKS5 success, or HTTP
    /// 200), then dial and hold the bytes of the client in the meantime. A
    /// failed dial closes the connection.
    async fn dial_early<S>(&self, client: &mut S, target: &Target, socks5: bool) -> Result<()>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send,
    {
        if socks5 {
            socks::socks5_reply(client, 0, None).await?;
        } else {
            socks::http_connect_ok(client).await?;
        }
        let deadline = Instant::now() + DIAL_LIMIT;
        let dial = tokio::time::timeout_at(deadline, self.dial.dial(target, deadline));
        tokio::pin!(dial);
        let mut held: Vec<u8> = Vec::new();
        let mut chunk = vec![0u8; 16 * 1024];
        let mut open = true;
        let dialed = loop {
            let room = EARLY_HOLD - held.len();
            let part = room.min(chunk.len());
            tokio::select! {
                dialed = &mut dial => break dialed,
                read = client.read(&mut chunk[..part]), if open && room > 0 => match read {
                    // The client ends its side: the splice sends the end on
                    // after the held bytes.
                    Ok(0) => open = false,
                    Ok(n) => held.extend_from_slice(&chunk[..n]),
                    Err(_) => return Ok(()),
                },
            }
        };
        let failure = match dialed {
            Ok(Ok(mut conn)) => {
                if conn.io.write_all(&held).await.is_err() {
                    return Ok(());
                }
                self.splice(client, &mut conn.io).await;
                return Ok(());
            }
            Ok(Err(e)) => e.text().to_string(),
            Err(_) => "the dial timed out".to_string(),
        };
        let _ = client.shutdown().await;
        anyhow::bail!("an early-replied dial failed: {failure}")
    }

    /// Copy both ways until a side ends, or until the flow limit.
    async fn splice<A, B>(&self, client: &mut A, upstream: &mut B)
    where
        A: AsyncRead + AsyncWrite + Unpin + ?Sized,
        B: AsyncRead + AsyncWrite + Unpin + ?Sized,
    {
        let copy =
            tokio::io::copy_bidirectional_with_sizes(client, upstream, RELAY_BUFFER, RELAY_BUFFER);
        match self.flow_limit {
            Some(limit) => {
                let _ = tokio::time::timeout(limit, copy).await;
            }
            None => {
                let _ = copy.await;
            }
        }
    }

    /// The target of a request of a client: a name stays a name (the dialer
    /// resolves it where its way says). Port 0 takes the default port.
    fn target_of(&self, req: &ProxyRequest) -> Target {
        let port = if req.port == 0 {
            self.default_port
        } else {
            req.port
        };
        match &req.host {
            ProxyHost::Ip(ip) => Target::Addr(std::net::SocketAddr::new(*ip, port)),
            ProxyHost::Domain(name) => Target::Name {
                host: name.clone(),
                port,
            },
        }
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
                other => Request::Command(other),
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
    /// Another SOCKS5 command: UDP ASSOCIATE (3), BIND (2) or one that an
    /// extension knows. The address of a UDP ASSOCIATE is the expected
    /// source of the client, which the relay does not need (it pins the
    /// first source).
    Command(u8),
    Http(HttpRequest),
    Refused,
}

/// The SOCKS5 reply code of a failed dial: "not allowed" for a policy
/// refusal, "host unreachable" for a timeout, else by the words of the text.
fn socks_code(e: &Failure) -> u8 {
    match e.class {
        Class::Policy => 0x02,
        Class::Timeout => 0x04,
        _ => socks::socks5_code_for_error(&e.to_string()),
    }
}

/// The HTTP answer of a failed dial: 403 for a policy refusal, 504 for a
/// timeout, else 502, with the text of the failure in the body.
async fn http_failure<S>(stream: &mut S, e: &Failure) -> Result<()>
where
    S: AsyncWrite + Unpin,
{
    let (status, reason) = match e.class {
        Class::Policy => (403, "Forbidden"),
        Class::Timeout => (504, "Gateway Timeout"),
        _ => (502, "Bad Gateway"),
    };
    let body = format!("{}\n", e.text().replace(['\r', '\n'], " "));
    socks::http_reply_with_body(stream, status, reason, &body).await
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

#[cfg(test)]
mod early_tests;
