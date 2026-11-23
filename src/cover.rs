//! `podnet cover`: a listener that answers as a static site until a client
//! proves that it holds the key (T-369, GitHub #491).
//!
//! The listener completes a real TLS handshake with the cover site on one
//! address (SWITCH-1 and SWITCH-4). The first application bytes then decide
//! the connection:
//!
//! * a valid proof (an HMAC over a timestamp, a nonce and the handshake
//!   exporter) opens a session to [`Config::target`] (SWITCH-1 and SWITCH-3);
//! * each other byte goes to the cover site, with no close and no reset
//!   (SWITCH-2). A replayed proof outside the window is one of those bytes:
//!   the site answers the request and the connection goes on.
//!
//! The site and its name routing are [`pod_cover::Site`] and
//! [`pod_cover::Sites`]. The script is [`pod_cover::Script`], loaded at
//! start: a missing script or one with no date exits 78 (SCRIPT-2 and
//! SCRIPT-3). The first records of a session follow the sizes of the
//! script within the tolerance of its histogram (SCRIPT-1). An
//! authenticated client pads its first record to the cover's first record
//! with [`pad_first`]: the proof, the `PAD1` marker, a two-byte length and
//! zeros. [`strip_first_padding`] removes them before the tunnel.
//!
//! With [`Config::mux`], one connection carries a page load and a session.
//! The page goes first and both lanes share one budget (MUX-1 and MUX-2).
//!
//! pod-tls serves the certificate: this module adds no TLS stack.

use std::collections::VecDeque;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use anyhow::{anyhow, Context as _};
use pod_cover::{Frame, Lane, Mux, Outcome, Script, Site, Sites, Switch};
use pod_tls::server::TlsAcceptor;
use tokio::io::{AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::server::TlsStream;

use crate::exit::{self, Fault};

/// Connect to the origin of the cover, `HOST:PORT`. The policy of the
/// deployment decides before the socket opens (EGRESS-3 and EGRESS-4), so a
/// refusal is an error with its reason and never a timeout.
pub async fn open_target(target: &str) -> io::Result<TcpStream> {
    let (host, port) = split_host_port(target)?;
    crate::policy::permit_io(host, port)?;
    TcpStream::connect(target).await
}

/// The host and port of `HOST:PORT`, or `[V6]:PORT`.
fn split_host_port(text: &str) -> io::Result<(&str, u16)> {
    let (host, port) = if let Some(rest) = text.strip_prefix('[') {
        let (host, after) = rest
            .split_once(']')
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no closing bracket"))?;
        let port = after.strip_prefix(':').ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "no port after the bracket")
        })?;
        (host, port)
    } else {
        text.rsplit_once(':')
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no port in HOST:PORT"))?
    };
    let port: u16 = port
        .parse()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "the port is not 0 to 65535"))?;
    Ok((host, port))
}

/// The most bytes of a request head that the site reads.
pub const MAX_HEAD: usize = 16 * 1024;
/// The most requests of one connection to the site.
pub const MAX_REQUESTS: usize = 100;
/// How long the listener waits for the first 56 bytes before it treats the
/// bytes as a request of the site.
pub const FIRST_GAP: Duration = Duration::from_millis(100);
/// How long the site waits for the rest of a request head. A proof that the
/// switch refused is no request, so the site answers it after this gap.
pub const HEAD_GAP: Duration = Duration::from_millis(200);
/// The bytes of one TLS record of the tunnel before the sizes of the script
/// run out.
const SCRIPT_RECORDS: usize = 64 * 1024;
/// The most bytes of one write of the tunnel.
const WRITE_LIMIT: usize = 16 * 1024;

/// What `podnet cover` needs to start.
#[derive(Clone, Debug)]
pub struct Config {
    /// The directory of the cover site.
    pub site: String,
    /// The name of the site, for the routing of `Sites`.
    pub name: Option<String>,
    /// The cover script, as a file.
    pub script: String,
    /// The HMAC key of the switch, 32 bytes.
    pub switch_key: [u8; 32],
    /// The target of an authenticated session, `HOST:PORT`.
    pub target: String,
    /// The `Server` header of the site. `None` keeps `nginx`.
    pub server: Option<String>,
    /// Serve the page load and the session over one connection, the page
    /// first (MUX-1 and MUX-2).
    pub mux: bool,
}

/// The listener: the site, the script and the switch.
#[derive(Debug)]
pub struct Cover {
    sites: Sites,
    script: Script,
    /// The key of the switch. Each connection binds the proof to the
    /// exporter of its own TLS handshake, so the switch is built for that
    /// connection.
    switch_key: [u8; 32],
    /// The switch for a caller that has no connection to bind (a test of
    /// the window or the nonce).
    switch: Mutex<Switch>,
    target: String,
    mux: bool,
}

impl Cover {
    /// Read the site and the script, and build the switch. A missing site,
    /// script, date or cover version is exit 78 at load (SCRIPT-2 and
    /// SCRIPT-3).
    pub fn load(config: Config) -> anyhow::Result<Cover> {
        let mut site = Site::load(&config.site)
            .map_err(|e| config_fault(format!("the cover site {}: {e}", config.site)))?;
        if let Some(server) = &config.server {
            site.set_server(server.clone())
                .map_err(|e| config_fault(format!("--server: {e}")))?;
        }
        let script = Script::load(&config.script)
            .map_err(|e| config_fault(format!("the cover script {}: {e}", config.script)))?;
        let mut sites = Sites::new(site.clone());
        if let Some(name) = &config.name {
            sites.insert(name.clone(), site);
        }
        let switch = Switch::new(config.switch_key);
        Ok(Cover {
            sites,
            script,
            switch_key: config.switch_key,
            switch: Mutex::new(switch),
            target: config.target,
            mux: config.mux,
        })
    }

    /// The script of this listener.
    pub fn script(&self) -> &Script {
        &self.script
    }

    /// The size of the cover's first record, from the script (SCRIPT-1).
    /// A client pads its first application-data record to this size, so the
    /// switch and the cover site present the same first record.
    pub fn first_record_size(&self) -> usize {
        self.script
            .replay(SCRIPT_RECORDS)
            .first()
            .map_or(1, |record| record.size)
    }

    /// The answer of the site to one request of the name `sni`.
    pub fn respond(&self, sni: Option<&str>, request: &[u8]) -> Vec<u8> {
        let mut response = self.sites.respond(sni, request);
        // SWITCH-2: the connection to the site stays open after the page.
        for (name, value) in &mut response.headers {
            if name.eq_ignore_ascii_case("connection") {
                *value = "keep-alive".to_string();
            }
        }
        response.to_bytes()
    }

    /// The outcome of the first application bytes at `now`, with no
    /// connection binding. The listener uses [`Cover::authenticate_bound`].
    pub fn authenticate(&self, bytes: &[u8], now: i64) -> Outcome {
        let mut switch = self.switch.lock().unwrap_or_else(|e| e.into_inner());
        switch.authenticate(bytes, now)
    }

    /// The outcome of the first application bytes of a connection whose TLS
    /// exporter is `exporter` at `now` (SWITCH-1). A proof of another
    /// connection fails the tag: the exporter is part of the message.
    pub fn authenticate_bound(&self, exporter: &[u8], bytes: &[u8], now: i64) -> Outcome {
        let mut switch = Switch::new(self.switch_key).bind(exporter);
        switch.authenticate(bytes, now)
    }

    /// The page load as the bytes that a session carries first with `mux`.
    pub fn page(&self, sni: Option<&str>) -> Vec<u8> {
        self.respond(
            sni,
            b"GET / HTTP/1.1\r\nHost: cover\r\nConnection: keep-alive\r\n\r\n",
        )
    }

    /// The chunks of `payload` at the sizes of the script (SCRIPT-1). The
    /// sizes are the measured records of the cover, so the first records of
    /// the tunnel look like those of the site.
    pub fn chunks(&self, payload: &[u8]) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        let mut at = 0;
        for record in self.script.replay(payload.len()) {
            let end = (at + record.size).min(payload.len());
            out.push(payload[at..end].to_vec());
            at = end;
        }
        if at < payload.len() {
            out.push(payload[at..].to_vec());
        }
        out
    }

    /// The order of one page and one session over one connection: each
    /// cover chunk before a session chunk, and the page complete first
    /// (MUX-2). Both lanes spend one budget, the bytes of the two frames
    /// (MUX-1).
    pub fn interleave(&self, page: &[u8], session: &[u8]) -> Vec<(Lane, Vec<u8>)> {
        let budget = (page.len() + session.len()) as u64;
        let mut mux = Mux::new(budget);
        mux.push(Frame::cover(page.to_vec()));
        mux.push(Frame::session(session.to_vec()));
        let mut out = Vec::new();
        while let Some(chunk) = mux.next(WRITE_LIMIT) {
            out.push((chunk.lane, chunk.bytes));
        }
        out
    }

    /// Serve one connection that the acceptor already completed.
    pub async fn session(&self, mut tls: TlsStream<TcpStream>) {
        let sni = tls.get_ref().1.server_name().map(|name| name.to_string());
        // SWITCH-1: the proof is bound to the exporter of this handshake.
        let exporter = match exporter_of(&tls) {
            Ok(exporter) => exporter,
            Err(error) => {
                tracing::debug!(error = %error, "the cover handshake has no exporter");
                return;
            }
        };
        loop {
            let first = match first_bytes(&mut tls).await {
                Ok(bytes) if !bytes.is_empty() => bytes,
                _ => return,
            };
            let now = unix_now();
            if matches!(
                self.authenticate_bound(&exporter, &first, now),
                Outcome::Session
            ) {
                // The proof takes the first 56 bytes; the bytes after it are
                // the first bytes of the session, after the optional padding
                // that makes the first record the cover's size.
                let rest = first
                    .get(pod_cover::switch::PROOF_LEN..)
                    .unwrap_or_default();
                let rest = strip_first_padding(rest).unwrap_or(rest);
                let _ = self.tunnel(tls, sni, rest).await;
                return;
            }
            match self.site_request(&mut tls, sni.as_deref(), &first).await {
                Ok(true) => continue,
                Ok(false) => return,
                Err(_) => return,
            }
        }
    }

    async fn site_request(
        &self,
        tls: &mut TlsStream<TcpStream>,
        sni: Option<&str>,
        first: &[u8],
    ) -> io::Result<bool> {
        let mut head = first.to_vec();
        while !head.windows(4).any(|w| w == b"\r\n\r\n") {
            if head.len() >= MAX_HEAD {
                break;
            }
            let mut buf = [0u8; 4096];
            let n = match tokio::time::timeout(HEAD_GAP, tls.read(&mut buf)).await {
                Ok(Ok(n)) => n,
                Ok(Err(error)) => return Err(error),
                Err(_) => break,
            };
            if n == 0 {
                break;
            }
            head.extend_from_slice(&buf[..n]);
        }
        let answer = self.respond(sni, &head);
        tls.write_all(&answer).await?;
        tls.flush().await?;
        Ok(true)
    }

    async fn tunnel(
        &self,
        tls: TlsStream<TcpStream>,
        sni: Option<String>,
        rest: &[u8],
    ) -> io::Result<()> {
        let mut target = open_target(&self.target).await?;
        let _ = target.set_nodelay(true);
        if !rest.is_empty() {
            target.write_all(rest).await?;
        }
        let page = self.mux.then(|| self.page(sni.as_deref()));
        let (mut tls_read, mut tls_write) = tokio::io::split(tls);
        let (mut target_read, mut target_write) = target.split();
        let script = self.script.clone();
        let up = async {
            let mut shaped = Shaped::new(&mut tls_write, script.clone().replay(SCRIPT_RECORDS));
            // MUX-2: the page completes before one byte of the session. The
            // page is the cover lane, so it drains before each session frame.
            let mut mux = page.map(|page| {
                let mut mux = Mux::new(u64::MAX);
                mux.push(Frame::cover(page));
                mux
            });
            if let Some(mux) = mux.as_mut() {
                while let Some(chunk) = mux.next(WRITE_LIMIT) {
                    shaped.write_all(&chunk.bytes).await?;
                }
            }
            let mut buf = vec![0u8; WRITE_LIMIT];
            loop {
                let n = target_read.read(&mut buf).await?;
                if n == 0 {
                    break;
                }
                match mux.as_mut() {
                    Some(mux) => {
                        mux.push(Frame::session(buf[..n].to_vec()));
                        while let Some(chunk) = mux.next(WRITE_LIMIT) {
                            shaped.write_all(&chunk.bytes).await?;
                        }
                    }
                    None => shaped.write_all(&buf[..n]).await?,
                }
            }
            shaped.shutdown().await
        };
        let down = async {
            let mut shaped = Shaped::new(&mut target_write, script.replay(SCRIPT_RECORDS));
            tokio::io::copy(&mut tls_read, &mut shaped).await?;
            shaped.shutdown().await
        };
        let (up, down) = tokio::join!(up, down);
        up?;
        down
    }
}

fn config_fault(text: String) -> anyhow::Error {
    exit::fault(Fault::Config, anyhow!(text))
}

/// The exporter label and context of the cover handshake (SWITCH-1). A
/// client proves with the same label and context on its own connection.
pub const EXPORTER_LABEL: &[u8] = b"EXPORTER-pod-cover-switch";
/// The context of [`EXPORTER_LABEL`].
pub const EXPORTER_CONTEXT: Option<&[u8]> = Some(b"cover-v1");

/// The exporter of a completed server handshake. It is the channel binding
/// of the switch: a proof replayed on another connection fails the tag.
pub fn exporter_of(tls: &TlsStream<TcpStream>) -> Result<[u8; 32], rustls::Error> {
    let mut out = [0u8; 32];
    tls.get_ref()
        .1
        .export_keying_material(&mut out, EXPORTER_LABEL, EXPORTER_CONTEXT)?;
    Ok(out)
}

/// The seconds of the clock that the switch reads.
pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// The marker of the optional padding of an authenticated client's first
/// application-data record. The padding follows the 56-byte proof, so a
/// client can make that record the size of the cover's first record
/// (SCRIPT-1). The server strips it, so the tunnel sees only session bytes.
pub const PAD_MARKER: &[u8; 4] = b"PAD1";

/// The first application-data record of an authenticated client: `proof`,
/// the mark, a two-byte padding length and the padding, then `session`.
/// `None` when `size` cannot hold those bytes.
pub fn pad_first(proof: &[u8], session: &[u8], size: usize) -> Option<Vec<u8>> {
    let fixed = proof.len() + PAD_MARKER.len() + 2 + session.len();
    if size < fixed || size - fixed > u16::MAX as usize {
        return None;
    }
    let pad = size - fixed;
    let mut out = Vec::with_capacity(size);
    out.extend_from_slice(proof);
    out.extend_from_slice(PAD_MARKER);
    out.extend_from_slice(&(pad as u16).to_be_bytes());
    out.resize(out.len() + pad, 0);
    out.extend_from_slice(session);
    Some(out)
}

/// The session bytes of an authenticated first record, after the proof and
/// its padding. `None` when the bytes carry no padding mark.
pub fn strip_first_padding(bytes: &[u8]) -> Option<&[u8]> {
    let rest = bytes.strip_prefix(PAD_MARKER)?;
    let len = rest.get(..2)?;
    let pad = u16::from_be_bytes([len[0], len[1]]) as usize;
    rest.get(2..)?.get(pad..)
}

/// Whether `after_proof` may still become a padding mark whose length and
/// padding are not all present yet. An empty rest counts: a first record
/// can end exactly at the proof, and the padding can arrive in the next
/// TLS record. Waiting the first gap for it is what keeps those bytes out
/// of the tunnel.
fn incomplete_padding(after_proof: &[u8]) -> bool {
    if after_proof.len() < PAD_MARKER.len() {
        return PAD_MARKER.starts_with(after_proof);
    }
    if !after_proof.starts_with(PAD_MARKER) {
        return false;
    }
    let rest = &after_proof[PAD_MARKER.len()..];
    let Some(len) = rest.get(..2) else {
        return true;
    };
    let pad = u16::from_be_bytes([len[0], len[1]]) as usize;
    rest.len() < 2 + pad
}

/// The first bytes of a connection: 56 for a proof, or the start of a
/// request. The read stops early when the bytes are plainly a method, or
/// when a proof and its whole padding are present. The padding of a proof
/// makes the first record the size of the cover's first record (SCRIPT-1).
async fn first_bytes(tls: &mut TlsStream<TcpStream>) -> io::Result<Vec<u8>> {
    let mut head = Vec::new();
    loop {
        if is_method(&head) {
            return Ok(head);
        }
        if head.len() >= pod_cover::switch::PROOF_LEN
            && !incomplete_padding(&head[pod_cover::switch::PROOF_LEN..])
        {
            return Ok(head);
        }
        let mut buf = [0u8; 512];
        let read = if head.is_empty() {
            tls.read(&mut buf).await
        } else {
            match tokio::time::timeout(FIRST_GAP, tls.read(&mut buf)).await {
                Ok(read) => read,
                Err(_) => return Ok(head),
            }
        };
        let n = read?;
        if n == 0 {
            return Ok(head);
        }
        head.extend_from_slice(&buf[..n]);
        if head.len() > MAX_HEAD {
            return Ok(head);
        }
    }
}

/// Whether `bytes` start with an HTTP method: the site, not a proof.
fn is_method(bytes: &[u8]) -> bool {
    ["GET", "HEAD", "POST", "PUT", "OPTIONS", "DELETE", "PATCH"]
        .iter()
        .any(|method| {
            bytes.starts_with(method.as_bytes()) && bytes.get(method.len()) == Some(&b' ')
        })
}

/// A writer that limits each write to the next size of the script, so that
/// the first records of a tunnel have the sizes of the cover (SCRIPT-1).
pub struct Shaped<W> {
    inner: W,
    sizes: VecDeque<usize>,
}

impl<W> Shaped<W> {
    /// The writer over `inner`, with the sizes of a script replay.
    pub fn new(inner: W, records: Vec<pod_cover::Record>) -> Shaped<W> {
        let sizes = records
            .into_iter()
            .map(|record| record.size.clamp(1, WRITE_LIMIT))
            .collect();
        Shaped { inner, sizes }
    }

    /// The sizes that are left.
    pub fn left(&self) -> &VecDeque<usize> {
        &self.sizes
    }
}

impl<W: AsyncWrite + Unpin> AsyncWrite for Shaped<W> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let limit = this.sizes.front().copied().unwrap_or(buf.len());
        let take = limit.min(buf.len());
        match Pin::new(&mut this.inner).poll_write(cx, &buf[..take]) {
            Poll::Ready(Ok(0)) => Poll::Ready(Ok(0)),
            Poll::Ready(Ok(n)) => {
                if let Some(front) = this.sizes.front_mut() {
                    *front -= n;
                    if *front == 0 {
                        this.sizes.pop_front();
                    }
                }
                Poll::Ready(Ok(n))
            }
            other => other,
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

/// Read a switch key: 64 hexadecimal characters.
pub fn key_from_hex(text: &str) -> anyhow::Result<[u8; 32]> {
    let text = text.trim();
    let bad = || config_fault("the switch key is 64 hexadecimal characters".into());
    if text.len() != 64 {
        return Err(bad());
    }
    let mut out = [0u8; 32];
    for (at, chunk) in text.as_bytes().chunks(2).enumerate() {
        let pair = std::str::from_utf8(chunk).map_err(|_| bad())?;
        out[at] = u8::from_str_radix(pair, 16).map_err(|_| bad())?;
    }
    Ok(out)
}

/// Accept TLS connections on `listener` and serve each one with `cover`.
pub async fn serve(
    listener: TcpListener,
    acceptor: TlsAcceptor,
    cover: Arc<Cover>,
) -> io::Result<()> {
    loop {
        let (tcp, _) = listener.accept().await?;
        let acceptor = acceptor.clone();
        let cover = cover.clone();
        tokio::spawn(async move {
            match acceptor.accept(tcp).await {
                Ok(tls) => cover.session(tls).await,
                Err(error) => {
                    tracing::debug!(error = %error, "the cover handshake failed");
                }
            }
        });
    }
}

/// Bind `address` and serve `cover` with the certificate of the two files.
pub async fn run(
    address: SocketAddr,
    cert_pem: Vec<u8>,
    key_pem: Vec<u8>,
    cover: Arc<Cover>,
) -> anyhow::Result<()> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let acceptor =
        pod_tls::server::acceptor(provider, &cert_pem, &key_pem, &[pod_tls::server::HTTP1])
            .map_err(|e| config_fault(format!("the certificate or the key: {e}")))?;
    let listener = TcpListener::bind(address)
        .await
        .with_context(|| format!("the cover listener at {address}"))
        .map_err(crate::net::bind_fault)?;
    serve(listener, acceptor, cover)
        .await
        .map_err(|e| exit::fault(Fault::Unavailable, anyhow!("the cover listener: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bad_key_is_refused_at_load() {
        assert!(key_from_hex("00").is_err());
        assert!(key_from_hex(&"zz".repeat(32)).is_err());
        assert_eq!(key_from_hex(&"01".repeat(32)).unwrap()[0], 1);
    }

    #[test]
    fn a_method_is_not_a_proof() {
        assert!(is_method(b"GET / HTTP/1.1\r\n"));
        assert!(is_method(b"POST /x HTTP/1.1\r\n"));
        assert!(!is_method(b"GE"));
        assert!(!is_method(&[0u8; 56]));
    }
}
