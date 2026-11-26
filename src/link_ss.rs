//! The service `ss` of a stream of a link (T-294, GitHub #471 row E5-5):
//! the accept side of the AEAD form of Shadowsocks (SIP004). This is not
//! Shadowsocks 2022 (SIP022).
//!
//! The peer that opened the stream speaks Shadowsocks on it, as a client
//! speaks to a Shadowsocks server: a salt, then chunks. The first chunk
//! starts with the target (the address form of SOCKS5); this end dials the
//! target from this host and copies the bytes both ways. podnet has no code
//! of the protocol: the codec, the key derivation (`EVP_BytesToKey` with
//! MD5, then HKDF-SHA1 with the info `ss-subkey`) and the address form are
//! `shadowsocks` of `pod-egress`. The replay window is `replay` of
//! `pod-egress`.
//!
//! * The key is `METHOD:PASSWORD`, with a method of SIP004:
//!   `chacha20-ietf-poly1305`, `aes-256-gcm` or `aes-128-gcm`. It comes
//!   from the file in [`KEY_FILE_VARIABLE`] or from [`KEY_VARIABLE`], never
//!   from `argv`. No line, no `Debug` and no status holds the password.
//! * The salt of each stream goes into a replay window of 10 minutes after
//!   its first chunk opens. A salt that comes again is refused.
//! * A stream that does not open (another key, bytes of no Shadowsocks
//!   client, a replay, no address, a target that does not answer) ends
//!   with no byte back. The log gets one line.
//! * The target is a destination of the egress policy of this host, as the
//!   target of each other service.

use std::future::Future;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use pod_core::BoxedIo;
use pod_egress::replay::{Admitted, ReplayWindow};
use pod_egress::shadowsocks::{self as ss, ChunkError, Decoder, Encoder, Method};
use pod_ws::headers::Zeroizing;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::exit::{self, Fault};

/// The name of the service.
pub const NAME: &str = "ss";

/// The variable of the file of the key.
pub const KEY_FILE_VARIABLE: &str = "PODNET_SHADOWSOCKS_SERVICE_KEY_FILE";
/// The variable of the key. The file is the better choice.
pub const KEY_VARIABLE: &str = "PODNET_SHADOWSOCKS_SERVICE_KEY";

/// The time that the salt and the first chunk can take.
pub const FIRST_LIMIT: Duration = Duration::from_secs(30);

/// The time that a connection to the target can take.
pub const CONNECT_LIMIT: Duration = Duration::from_secs(10);

/// The bytes of one read.
const READ: usize = 16 * 1024;

/// The methods of SIP004, for a line.
const METHODS: &str = "chacha20-ietf-poly1305, aes-256-gcm or aes-128-gcm";

/// The key of the service: the method and the master key of the password.
pub struct Key {
    method: Method,
    master: Zeroizing<Vec<u8>>,
}

/// The master key is not shown.
impl std::fmt::Debug for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Key")
            .field("method", &self.method.name())
            .finish_non_exhaustive()
    }
}

impl Key {
    /// The key of `text` (`METHOD:PASSWORD`), which the variable `variable`
    /// gave. A bad key exits 64; the line names the variable and never
    /// holds the password.
    pub fn parse(text: &str, variable: &str) -> Result<Key> {
        let usage = |why: String| exit::fault(Fault::Usage, anyhow!("{variable}: {why}"));
        let text = text.trim();
        if text.contains(['\n', '\r']) {
            return Err(usage("the key is one line, METHOD:PASSWORD".into()));
        }
        let Some((method, password)) = text.split_once(':') else {
            return Err(usage("the key is METHOD:PASSWORD".into()));
        };
        // The words of the crate show a name that it does not know: a part
        // of a password could be there, so the line names the methods alone.
        let method = Method::parse(method).map_err(|_| {
            usage(format!(
                "the method before the colon is not one of {METHODS}"
            ))
        })?;
        if method.is_2022() {
            return Err(usage(format!(
                "the method {} is Shadowsocks 2022; this service speaks the AEAD form of SIP004 alone: {METHODS}",
                method.name()
            )));
        }
        if password.is_empty() {
            return Err(usage("the password after the colon is empty".into()));
        }
        Ok(Key {
            method,
            master: ss::master_key(password.as_bytes(), method.key_len()),
        })
    }

    /// The key of the file in [`KEY_FILE_VARIABLE`], else of [`KEY_VARIABLE`],
    /// read with `get`. A missing or bad key exits 64.
    pub fn from_env(get: &dyn Fn(&str) -> Option<String>) -> Result<Key> {
        let usage = |text: String| exit::fault(Fault::Usage, anyhow!(text));
        let given = crate::networks::secret::read(get, KEY_FILE_VARIABLE, KEY_VARIABLE)
            .map_err(|e| usage(format!("--service {NAME}: {e}")))?;
        let Some(given) = given else {
            return Err(usage(format!(
                "--service {NAME}: give the key of the Shadowsocks service: set {KEY_FILE_VARIABLE} (a file with \
                 METHOD:PASSWORD) or {KEY_VARIABLE}"
            )));
        };
        Key::parse(&given.text, &given.variable)
    }

    /// The method of the key.
    pub fn method(&self) -> Method {
        self.method
    }
}

/// The service: its key and its replay window, shared by its streams.
pub struct Server {
    key: Key,
    replay: ReplayWindow,
}

impl Server {
    pub fn new(key: Key) -> Server {
        Server {
            key,
            replay: ReplayWindow::new(),
        }
    }

    /// The method of the key.
    pub fn method(&self) -> Method {
        self.key.method
    }
}

/// The method alone: the password is not shown.
impl std::fmt::Display for Server {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "shadowsocks {}", self.key.method.name())
    }
}

impl std::fmt::Debug for Server {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Server({self})")
    }
}

/// A refusal of the stream of the peer: one line, the class `class`.
fn refused(class: Fault, text: String) -> anyhow::Error {
    exit::fault(class, anyhow!("a Shadowsocks stream is refused: {text}"))
}

/// The line of a chunk of the peer that did not open.
fn chunk_fault(e: ChunkError) -> anyhow::Error {
    match e {
        ChunkError::Length | ChunkError::Payload => refused(
            Fault::NoPermission,
            "a chunk of the peer did not open: the key of the peer is not the key of this service, or the \
             bytes are no Shadowsocks stream"
                .into(),
        ),
        ChunkError::TooLong(n) => refused(
            Fault::DataError,
            format!("a chunk of the peer has the length {n}, above {}", ss::MAX_PAYLOAD),
        ),
        other => refused(Fault::DataError, other.to_string()),
    }
}

/// Serve one stream: open the request of the peer, dial its target from
/// this host, and copy the bytes both ways. Each fault is one line; the
/// peer gets no byte of a stream that does not open. A stream that ends
/// with no byte (the reach probe of a client) is no fault.
pub async fn serve<S>(server: &Server, stream: S) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    serve_with(server, stream, direct).await
}

/// [`serve`] with `connect` in place of the direct dial: a test gives a
/// target of its own.
pub async fn serve_with<S, C, F>(server: &Server, mut stream: S, connect: C) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
    C: FnOnce(String, u16) -> F,
    F: Future<Output = Result<BoxedIo>>,
{
    let method = server.key.method;
    let mut decoder = Decoder::new(method, &server.key.master);
    let opened = tokio::time::timeout(
        FIRST_LIMIT,
        first_chunk(&mut stream, &mut decoder, method.salt_len()),
    )
    .await
    .map_err(|_| {
        refused(
            Fault::Timeout,
            format!("no first chunk within {} s", FIRST_LIMIT.as_secs()),
        )
    })??;
    let Some((first, salt)) = opened else {
        return Ok(());
    };
    // The salt goes into the window only after its chunk opened: bytes of
    // another key do not fill it.
    if server.replay.admit(&salt) == Admitted::Repeat {
        return Err(refused(
            Fault::NoPermission,
            "the salt of the stream came before: a replay".into(),
        ));
    }
    let Some((host, port, used)) = ss::parse_address(&first) else {
        return Err(refused(
            Fault::DataError,
            "the first chunk holds no target address".into(),
        ));
    };
    let target = connect(host, port).await?;
    let encoder = Encoder::new(method, &server.key.master)
        .map_err(|e| refused(Fault::Unavailable, e.to_string()))?;
    splice(stream, decoder, &first[used..], target, encoder).await
}

/// Read until the first chunk opens: its plain text, and the salt of the
/// stream; `None` when the peer ended the stream with no byte.
async fn first_chunk<S: AsyncRead + Unpin>(
    stream: &mut S,
    decoder: &mut Decoder,
    salt_len: usize,
) -> Result<Option<(Vec<u8>, Vec<u8>)>> {
    let mut salt = Vec::with_capacity(salt_len);
    let mut buf = vec![0u8; READ];
    loop {
        if let Some(plain) = decoder.chunk().map_err(chunk_fault)? {
            return Ok(Some((plain, salt)));
        }
        let n = stream
            .read(&mut buf)
            .await
            .context("a Shadowsocks stream: reading the peer")?;
        if n == 0 && salt.is_empty() {
            return Ok(None);
        }
        if n == 0 {
            let what = match salt.len() < salt_len {
                true => "its salt",
                false => "its first chunk",
            };
            return Err(refused(
                Fault::DataError,
                format!("the peer ended the stream before {what}"),
            ));
        }
        let want = salt_len - salt.len();
        salt.extend_from_slice(&buf[..n.min(want)]);
        decoder.feed(&buf[..n]);
    }
}

/// Copy the opened chunks of the peer to the target, and the bytes of the
/// target, sealed, to the peer. An end of one side closes the write of the
/// other side; the copy ends when both sides ended.
async fn splice<S>(
    stream: S,
    mut decoder: Decoder,
    early: &[u8],
    target: BoxedIo,
    mut encoder: Encoder,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    let (mut peer_read, mut peer_write) = tokio::io::split(stream);
    let (mut target_read, mut target_write) = tokio::io::split(target);
    let up = async {
        let write = "a Shadowsocks stream: writing to the target";
        if !early.is_empty() {
            target_write.write_all(early).await.context(write)?;
        }
        let mut buf = vec![0u8; READ];
        loop {
            while let Some(plain) = decoder.chunk().map_err(chunk_fault)? {
                target_write.write_all(&plain).await.context(write)?;
            }
            let n = peer_read
                .read(&mut buf)
                .await
                .context("a Shadowsocks stream: reading the peer")?;
            if n == 0 {
                if decoder.buffered() > 0 {
                    return Err(refused(
                        Fault::DataError,
                        "the peer ended the stream inside a chunk".into(),
                    ));
                }
                let _ = target_write.shutdown().await;
                return Ok(());
            }
            decoder.feed(&buf[..n]);
        }
    };
    let down = async {
        let write = "a Shadowsocks stream: writing to the peer";
        let mut buf = vec![0u8; READ];
        let mut wire = Vec::with_capacity(READ + 64);
        loop {
            let n = target_read
                .read(&mut buf)
                .await
                .context("a Shadowsocks stream: reading the target")?;
            if n == 0 {
                let _ = peer_write.shutdown().await;
                return Ok::<(), anyhow::Error>(());
            }
            wire.clear();
            encoder
                .seal(&buf[..n], &mut wire)
                .map_err(|e| refused(Fault::Unavailable, e.to_string()))?;
            peer_write.write_all(&wire).await.context(write)?;
            peer_write.flush().await.context(write)?;
        }
    };
    tokio::try_join!(up, down)?;
    Ok(())
}

/// A TCP connection from this host to the target of a stream. The egress
/// policy decides before the socket opens.
async fn direct(host: String, port: u16) -> Result<BoxedIo> {
    let shown = pod_proxy::host::authority(&host, port);
    crate::policy::permit_fault(&host, port)
        .map_err(|e| exit::fault(Fault::Unavailable, anyhow!("the target {shown}: {e:#}")))?;
    let unavailable = |text: String| {
        exit::fault(
            Fault::Unavailable,
            anyhow!("a Shadowsocks stream: the target {shown}: {text}"),
        )
    };
    let work = tokio::net::TcpStream::connect((host.as_str(), port));
    match tokio::time::timeout(CONNECT_LIMIT, work).await {
        Err(_) => Err(unavailable(format!(
            "no answer in {} s",
            CONNECT_LIMIT.as_secs()
        ))),
        Ok(Err(e)) => Err(unavailable(e.to_string())),
        Ok(Ok(tcp)) => {
            let _ = tcp.set_nodelay(true);
            Ok(Box::new(tcp))
        }
    }
}
