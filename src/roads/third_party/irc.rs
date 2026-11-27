//! IRC third-party road (`irc://HOST:PORT/CHANNEL`) (T-368).
//!
//! Performs `NICK`/`USER`/`JOIN` over TCP and frames bidirectional byte streams in
//! chunked, tagged base64 `PRIVMSG` lines within the 512-byte RFC 2812 line limit.

use anyhow::{anyhow, bail, Context, Result};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use tokio::io::{
    AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader, DuplexStream,
};
use tokio::net::TcpStream;

/// Default IRC TCP port when omitted from a URL.
pub const DEFAULT_IRC_PORT: u16 = 6667;
/// Maximum raw payload bytes per `PRIVMSG` chunk (320 base64 chars, well under 512 bytes).
pub const MAX_CHUNK_BYTES: usize = 240;
/// Frame tag prefix on channel messages.
pub const FRAME_PREFIX: &str = "PN1:";

/// Parsed IRC road endpoint (`irc://HOST:PORT/CHANNEL`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IrcEndpoint {
    pub host: String,
    pub port: u16,
    pub channel: String,
    pub nick: String,
    pub stream_id: u32,
}

impl IrcEndpoint {
    /// Parse `irc://HOST[:PORT]/CHANNEL[?nick=NAME&stream=ID]`.
    pub fn parse(url: &str) -> Result<Self> {
        let rest = url
            .trim()
            .strip_prefix("irc://")
            .ok_or_else(|| anyhow!("an IRC road URL must start with irc://"))?;
        let (path_part, query) = match rest.split_once('?') {
            Some((p, q)) => (p, q),
            None => (rest, ""),
        };
        let (authority, raw_channel) = path_part
            .split_once('/')
            .ok_or_else(|| anyhow!("an IRC road URL needs HOST:PORT/CHANNEL"))?;
        let (host, port) = split_host_port(authority, DEFAULT_IRC_PORT)?;
        let raw_channel = raw_channel.trim();
        if raw_channel.is_empty() {
            bail!("an IRC road URL needs a non-empty channel");
        }
        let channel = if raw_channel.starts_with('#') {
            raw_channel.to_string()
        } else {
            format!("#{raw_channel}")
        };
        reject_crlf(&host, "host")?;
        reject_crlf(&channel, "channel")?;
        let mut nick = default_nick();
        let mut stream_id: u32 = 1;
        for pair in query.split('&').filter(|p| !p.is_empty()) {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            match k {
                "nick" if !v.is_empty() => {
                    reject_crlf(v, "nick")?;
                    nick = v.to_string();
                }
                "stream" if !v.is_empty() => {
                    stream_id = v
                        .parse()
                        .with_context(|| format!("invalid IRC stream id {v:?}"))?;
                }
                _ => {}
            }
        }
        Ok(Self {
            host,
            port,
            channel,
            nick,
            stream_id,
        })
    }

    /// Return a copy using a specific nickname.
    pub fn with_nick(mut self, nick: impl Into<String>) -> Self {
        self.nick = nick.into();
        self
    }

    /// Return a copy using a specific stream id.
    pub fn with_stream_id(mut self, stream_id: u32) -> Self {
        self.stream_id = stream_id;
        self
    }

    /// `host:port` TCP target address.
    pub fn authority(&self) -> String {
        if self.host.contains(':') {
            format!("[{}]:{}", self.host, self.port)
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }
}

/// Decoded IRC payload frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IrcFrame {
    Data {
        sender: Option<String>,
        channel: String,
        stream_id: u32,
        seq: u32,
        payload: Vec<u8>,
    },
    Eof {
        sender: Option<String>,
        channel: String,
        stream_id: u32,
    },
}

/// Encode raw bytes into chunked tagged `PRIVMSG` IRC lines.
pub fn encode_privmsg_chunks(
    channel: &str,
    stream_id: u32,
    start_seq: &mut u32,
    payload: &[u8],
) -> Vec<String> {
    let mut lines = Vec::new();
    for chunk in payload.chunks(MAX_CHUNK_BYTES) {
        let seq = *start_seq;
        *start_seq = start_seq.wrapping_add(1);
        let b64 = BASE64.encode(chunk);
        lines.push(format!(
            "PRIVMSG {channel} :{FRAME_PREFIX}{stream_id:08x}:{seq:08x}:{b64}\r\n"
        ));
    }
    lines
}

/// Encode an end-of-stream `PRIVMSG` marker.
pub fn encode_eof_line(channel: &str, stream_id: u32) -> String {
    format!("PRIVMSG {channel} :{FRAME_PREFIX}{stream_id:08x}:EOF\r\n")
}

/// Decode one IRC line into an [`IrcFrame`] when it holds a tagged `PRIVMSG`.
pub fn decode_privmsg_line(line: &str) -> Option<IrcFrame> {
    let line = line.trim_end_matches(['\r', '\n']);
    let (sender, rest) = if let Some(after_colon) = line.strip_prefix(':') {
        let (prefix, tail) = after_colon.split_once(' ')?;
        let nick = prefix.split('!').next().unwrap_or(prefix).to_string();
        (Some(nick), tail.trim_start())
    } else {
        (None, line)
    };
    let after_cmd = rest.strip_prefix("PRIVMSG ")?;
    let (channel, trailing) = after_cmd.split_once(" :")?;
    let body = trailing.strip_prefix(FRAME_PREFIX)?;
    let (id_hex, tail) = body.split_once(':')?;
    let stream_id = u32::from_str_radix(id_hex, 16).ok()?;
    if tail == "EOF" {
        return Some(IrcFrame::Eof {
            sender,
            channel: channel.to_string(),
            stream_id,
        });
    }
    let (seq_hex, b64) = tail.split_once(':')?;
    let seq = u32::from_str_radix(seq_hex, 16).ok()?;
    let payload = BASE64.decode(b64).ok()?;
    Some(IrcFrame::Data {
        sender,
        channel: channel.to_string(),
        stream_id,
        seq,
        payload,
    })
}

/// Extract a `PING` token if the IRC line is a server ping.
pub fn parse_ping(line: &str) -> Option<&str> {
    let line = line.trim_end_matches(['\r', '\n']);
    let rest = line.strip_prefix("PING ")?;
    Some(rest.strip_prefix(':').unwrap_or(rest))
}

/// Bidirectional byte stream carried over an IRC channel.
pub struct IrcStream {
    inner: DuplexStream,
}

impl IrcStream {
    /// Connect to `endpoint`, perform `NICK`/`USER`/`JOIN`, and bridge a duplex stream.
    pub async fn connect(endpoint: &IrcEndpoint) -> Result<Self> {
        crate::policy::permit_fault(&endpoint.host, endpoint.port)?;
        let tcp = TcpStream::connect(endpoint.authority())
            .await
            .with_context(|| format!("connect IRC road {}", endpoint.authority()))?;
        Self::from_stream(tcp, endpoint.clone()).await
    }

    /// Perform the IRC handshake over an existing stream and bridge a duplex stream.
    pub async fn from_stream<S>(stream: S, endpoint: IrcEndpoint) -> Result<Self>
    where
        S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        let (read_half, mut write_half) = tokio::io::split(stream);
        let handshake = format!(
            "NICK {nick}\r\nUSER {nick} 0 * :{nick}\r\nJOIN {chan}\r\n",
            nick = endpoint.nick,
            chan = endpoint.channel,
        );
        write_half
            .write_all(handshake.as_bytes())
            .await
            .context("write IRC NICK/USER/JOIN")?;
        write_half.flush().await.context("flush IRC handshake")?;

        let mut reader = BufReader::new(read_half);
        let mut line = String::new();
        loop {
            line.clear();
            let n = reader
                .read_line(&mut line)
                .await
                .context("read IRC banner")?;
            if n == 0 {
                bail!("IRC server closed connection during JOIN handshake");
            }
            if let Some(token) = parse_ping(&line) {
                let pong = format!("PONG :{token}\r\n");
                write_half
                    .write_all(pong.as_bytes())
                    .await
                    .context("write IRC PONG")?;
                write_half.flush().await?;
                continue;
            }
            if line.contains(" JOIN ") || line.contains(" 001 ") || line.contains(" 366 ") {
                break;
            }
        }

        let (client_io, bridge_io) = tokio::io::duplex(64 * 1024);
        let (mut bridge_read, mut bridge_write) = tokio::io::split(bridge_io);
        let (pong_tx, mut pong_rx) = tokio::sync::mpsc::channel::<String>(16);

        let rx_channel = endpoint.channel.clone();
        let rx_nick = endpoint.nick.clone();
        let rx_stream_id = endpoint.stream_id;
        tokio::spawn(async move {
            let mut buf = String::new();
            loop {
                buf.clear();
                match reader.read_line(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        if let Some(token) = parse_ping(&buf) {
                            let _ = pong_tx.try_send(format!("PONG :{token}\r\n"));
                            continue;
                        }
                        if let Some(frame) = decode_privmsg_line(&buf) {
                            match frame {
                                IrcFrame::Data {
                                    sender,
                                    channel,
                                    stream_id,
                                    payload,
                                    ..
                                } => {
                                    if channel.eq_ignore_ascii_case(&rx_channel)
                                        && stream_id == rx_stream_id
                                        && sender.as_deref() != Some(rx_nick.as_str())
                                        && bridge_write.write_all(&payload).await.is_err()
                                    {
                                        break;
                                    }
                                }
                                IrcFrame::Eof {
                                    sender,
                                    channel,
                                    stream_id,
                                } => {
                                    if channel.eq_ignore_ascii_case(&rx_channel)
                                        && stream_id == rx_stream_id
                                        && sender.as_deref() != Some(rx_nick.as_str())
                                    {
                                        let _ = bridge_write.shutdown().await;
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            let _ = bridge_write.shutdown().await;
        });

        let tx_channel = endpoint.channel;
        let tx_stream_id = endpoint.stream_id;
        tokio::spawn(async move {
            let mut seq: u32 = 0;
            let mut buf = vec![0u8; MAX_CHUNK_BYTES * 8];
            loop {
                tokio::select! {
                    Some(pong) = pong_rx.recv() => {
                        if write_half.write_all(pong.as_bytes()).await.is_err() {
                            break;
                        }
                        let _ = write_half.flush().await;
                    }
                    read = bridge_read.read(&mut buf) => {
                        match read {
                            Ok(0) => {
                                let eof = encode_eof_line(&tx_channel, tx_stream_id);
                                let _ = write_half.write_all(eof.as_bytes()).await;
                                let _ = write_half.flush().await;
                                break;
                            }
                            Ok(n) => {
                                let lines = encode_privmsg_chunks(
                                    &tx_channel,
                                    tx_stream_id,
                                    &mut seq,
                                    &buf[..n],
                                );
                                for line in lines {
                                    if write_half.write_all(line.as_bytes()).await.is_err() {
                                        return;
                                    }
                                }
                                if write_half.flush().await.is_err() {
                                    break;
                                }
                            }
                            Err(_) => break,
                        }
                    }
                }
            }
        });

        Ok(Self { inner: client_io })
    }
}

impl AsyncRead for IrcStream {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for IrcStream {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        std::pin::Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

fn default_nick() -> String {
    let mut raw = [0u8; 4];
    let _ = getrandom::fill(&mut raw);
    format!("pn{:02x}{:02x}{:02x}{:02x}", raw[0], raw[1], raw[2], raw[3])
}

fn reject_crlf(value: &str, field: &str) -> Result<()> {
    if value.contains(['\r', '\n', ' ']) {
        bail!("IRC {field} must not contain whitespace or line breaks");
    }
    Ok(())
}

pub(crate) fn split_host_port(authority: &str, default_port: u16) -> Result<(String, u16)> {
    let authority = authority.trim();
    if authority.is_empty() {
        bail!("missing host in road URL");
    }
    if let Some(rest) = authority.strip_prefix('[') {
        let (host, tail) = rest
            .split_once(']')
            .ok_or_else(|| anyhow!("unclosed IPv6 bracket in {authority:?}"))?;
        let port = match tail.strip_prefix(':') {
            Some(p) => p
                .parse::<u16>()
                .with_context(|| format!("invalid port in {authority:?}"))?,
            None if tail.is_empty() => default_port,
            None => bail!("invalid authority {authority:?}"),
        };
        return Ok((host.to_string(), port));
    }
    match authority.rsplit_once(':') {
        Some((host, port_str)) if !host.is_empty() => {
            let port = port_str
                .parse::<u16>()
                .with_context(|| format!("invalid port in {authority:?}"))?;
            Ok((host.to_string(), port))
        }
        _ => Ok((authority.to_string(), default_port)),
    }
}
