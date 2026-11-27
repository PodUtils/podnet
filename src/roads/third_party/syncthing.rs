//! Syncthing Relay Protocol v1 third-party road (`syncthing://HOST:PORT/?id=DEVICE_ID`) (T-368).
//!
//! Implements the binary XDR framing of Relay Protocol v1 (`0x9E79BC40` magic,
//! `JoinRelayRequest`, `JoinSessionRequest`, `Response`, `ConnectRequest`, and
//! `SessionInvitation`) over TCP or TLS, switching to a raw bidirectional byte stream
//! after both peers join the session.

use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};
use rustls::pki_types::ServerName;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

use super::irc::split_host_port;

/// Syncthing Relay Protocol v1 header magic.
pub const RELAY_MAGIC: u32 = 0x9E79_BC40;
/// Default Syncthing relay port when omitted from a URL.
pub const DEFAULT_SYNCTHING_PORT: u16 = 22067;
/// Maximum accepted control frame length (64 KiB).
pub const MAX_FRAME_LEN: usize = 64 * 1024;

/// Protocol v1 message type identifiers.
pub const MSG_PING: i32 = 0;
pub const MSG_PONG: i32 = 1;
pub const MSG_JOIN_RELAY_REQUEST: i32 = 2;
pub const MSG_JOIN_SESSION_REQUEST: i32 = 3;
pub const MSG_RESPONSE: i32 = 4;
pub const MSG_CONNECT_REQUEST: i32 = 5;
pub const MSG_SESSION_INVITATION: i32 = 6;

/// Parsed Syncthing relay road endpoint (`syncthing://HOST:PORT/?id=DEVICE_ID`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncthingEndpoint {
    pub host: String,
    pub port: u16,
    pub device_id: String,
    pub tls: bool,
}

impl SyncthingEndpoint {
    /// Parse `syncthing://HOST[:PORT]/?id=DEVICE_ID[&tls=1]` or `syncthing+tls://...`.
    pub fn parse(url: &str) -> Result<Self> {
        let trimmed = url.trim();
        let (rest, mut tls) = if let Some(r) = trimmed.strip_prefix("syncthing+tls://") {
            (r, true)
        } else if let Some(r) = trimmed.strip_prefix("syncthing://") {
            (r, false)
        } else {
            bail!("a Syncthing road URL must start with syncthing:// or syncthing+tls://");
        };
        let (path_part, query) = match rest.split_once('?') {
            Some((p, q)) => (p, q),
            None => (rest, ""),
        };
        let authority = path_part.split('/').next().unwrap_or("");
        let (host, port) = split_host_port(authority, DEFAULT_SYNCTHING_PORT)?;
        let mut device_id = String::new();
        for pair in query.split('&').filter(|p| !p.is_empty()) {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            match k {
                "id" => device_id = v.to_string(),
                "tls" => tls = v == "1" || v.eq_ignore_ascii_case("true"),
                _ => {}
            }
        }
        if device_id.is_empty() {
            bail!("a Syncthing road URL needs ?id=DEVICE_ID");
        }
        Ok(Self {
            host,
            port,
            device_id,
            tls,
        })
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

/// Relay Protocol v1 control messages.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RelayMessage {
    Ping,
    Pong,
    JoinRelayRequest {
        subprotocols: Vec<String>,
    },
    JoinSessionRequest {
        key: Vec<u8>,
    },
    Response {
        code: i32,
        message: String,
    },
    ConnectRequest {
        id: Vec<u8>,
    },
    SessionInvitation {
        from: Vec<u8>,
        key: Vec<u8>,
        address: Vec<u8>,
        port: u16,
        server_socket: bool,
    },
}

/// Encode a Relay Protocol v1 message with its 12-byte big-endian header.
pub fn encode_message(msg: &RelayMessage) -> Result<Vec<u8>> {
    let mut payload = Vec::new();
    let msg_type = match msg {
        RelayMessage::Ping => MSG_PING,
        RelayMessage::Pong => MSG_PONG,
        RelayMessage::JoinRelayRequest { subprotocols } => {
            let count =
                u32::try_from(subprotocols.len()).map_err(|_| anyhow!("too many subprotocols"))?;
            payload.extend_from_slice(&count.to_be_bytes());
            for proto in subprotocols {
                write_xdr_bytes(&mut payload, proto.as_bytes())?;
            }
            MSG_JOIN_RELAY_REQUEST
        }
        RelayMessage::JoinSessionRequest { key } => {
            write_xdr_bytes(&mut payload, key)?;
            MSG_JOIN_SESSION_REQUEST
        }
        RelayMessage::Response { code, message } => {
            payload.extend_from_slice(&code.to_be_bytes());
            write_xdr_bytes(&mut payload, message.as_bytes())?;
            MSG_RESPONSE
        }
        RelayMessage::ConnectRequest { id } => {
            write_xdr_bytes(&mut payload, id)?;
            MSG_CONNECT_REQUEST
        }
        RelayMessage::SessionInvitation {
            from,
            key,
            address,
            port,
            server_socket,
        } => {
            write_xdr_bytes(&mut payload, from)?;
            write_xdr_bytes(&mut payload, key)?;
            write_xdr_bytes(&mut payload, address)?;
            payload.extend_from_slice(&u32::from(*port).to_be_bytes());
            payload.extend_from_slice(&u32::from(u8::from(*server_socket)).to_be_bytes());
            MSG_SESSION_INVITATION
        }
    };

    let msg_len = i32::try_from(payload.len())
        .map_err(|_| anyhow!("Syncthing relay message exceeds i32 length"))?;
    let mut out = Vec::with_capacity(12 + payload.len());
    out.extend_from_slice(&RELAY_MAGIC.to_be_bytes());
    out.extend_from_slice(&msg_type.to_be_bytes());
    out.extend_from_slice(&msg_len.to_be_bytes());
    out.extend_from_slice(&payload);
    Ok(out)
}

/// Write a Relay Protocol v1 message to `writer`.
pub async fn write_message<W: AsyncWrite + Unpin>(
    writer: &mut W,
    msg: &RelayMessage,
) -> Result<()> {
    let bytes = encode_message(msg)?;
    writer
        .write_all(&bytes)
        .await
        .context("write Syncthing relay frame")?;
    writer.flush().await.context("flush Syncthing relay frame")
}

/// Read and decode one Relay Protocol v1 message from `reader`.
pub async fn read_message<R: AsyncRead + Unpin>(reader: &mut R) -> Result<RelayMessage> {
    let mut header = [0u8; 12];
    reader
        .read_exact(&mut header)
        .await
        .context("read Syncthing relay header")?;
    let magic = u32::from_be_bytes([header[0], header[1], header[2], header[3]]);
    if magic != RELAY_MAGIC {
        bail!("invalid Syncthing relay magic {magic:#010x}");
    }
    let msg_type = i32::from_be_bytes([header[4], header[5], header[6], header[7]]);
    let msg_len = i32::from_be_bytes([header[8], header[9], header[10], header[11]]);
    if msg_len < 0 || (msg_len as usize) > MAX_FRAME_LEN {
        bail!("invalid Syncthing relay frame length {msg_len}");
    }
    let mut body = vec![0u8; msg_len as usize];
    reader
        .read_exact(&mut body)
        .await
        .context("read Syncthing relay body")?;

    match msg_type {
        MSG_PING => Ok(RelayMessage::Ping),
        MSG_PONG => Ok(RelayMessage::Pong),
        MSG_JOIN_RELAY_REQUEST => {
            if body.len() < 4 {
                bail!("truncated JoinRelayRequest");
            }
            let count = u32::from_be_bytes([body[0], body[1], body[2], body[3]]) as usize;
            let mut pos = 4;
            let mut subprotocols = Vec::with_capacity(count.min(32));
            for _ in 0..count {
                let (raw, next) = read_xdr_bytes(&body, pos)?;
                let proto = String::from_utf8(raw).context("subprotocol UTF-8")?;
                subprotocols.push(proto);
                pos = next;
            }
            Ok(RelayMessage::JoinRelayRequest { subprotocols })
        }
        MSG_JOIN_SESSION_REQUEST => {
            let (key, _) = read_xdr_bytes(&body, 0)?;
            Ok(RelayMessage::JoinSessionRequest { key })
        }
        MSG_RESPONSE => {
            if body.len() < 4 {
                bail!("truncated Response");
            }
            let code = i32::from_be_bytes([body[0], body[1], body[2], body[3]]);
            let (raw, _) = read_xdr_bytes(&body, 4)?;
            let message = String::from_utf8(raw).context("response message UTF-8")?;
            Ok(RelayMessage::Response { code, message })
        }
        MSG_CONNECT_REQUEST => {
            let (id, _) = read_xdr_bytes(&body, 0)?;
            Ok(RelayMessage::ConnectRequest { id })
        }
        MSG_SESSION_INVITATION => {
            let (from, pos) = read_xdr_bytes(&body, 0)?;
            let (key, pos) = read_xdr_bytes(&body, pos)?;
            let (address, pos) = read_xdr_bytes(&body, pos)?;
            if pos + 8 > body.len() {
                bail!("truncated SessionInvitation tail");
            }
            let port_u32 =
                u32::from_be_bytes([body[pos], body[pos + 1], body[pos + 2], body[pos + 3]]);
            let port = u16::try_from(port_u32).context("SessionInvitation port overflow")?;
            let server_socket =
                u32::from_be_bytes([body[pos + 4], body[pos + 5], body[pos + 6], body[pos + 7]])
                    != 0;
            Ok(RelayMessage::SessionInvitation {
                from,
                key,
                address,
                port,
                server_socket,
            })
        }
        other => bail!("unsupported Syncthing relay message type {other}"),
    }
}

/// Register as a relay peer using `JoinRelayRequest` and verify `Response(0)`.
pub async fn join_relay<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    subprotocols: Vec<String>,
) -> Result<()> {
    write_message(stream, &RelayMessage::JoinRelayRequest { subprotocols }).await?;
    expect_ok_response(stream, "JoinRelayRequest").await
}

/// Request a session to `target_id` using `ConnectRequest` and return the `SessionInvitation`.
pub async fn request_session<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    target_id: &[u8],
) -> Result<RelayMessage> {
    write_message(
        stream,
        &RelayMessage::ConnectRequest {
            id: target_id.to_vec(),
        },
    )
    .await?;
    match read_message(stream).await? {
        inv @ RelayMessage::SessionInvitation { .. } => Ok(inv),
        RelayMessage::Response { code, message } => {
            bail!("Syncthing relay refused ConnectRequest ({code}): {message}");
        }
        other => bail!("expected SessionInvitation, got {other:?}"),
    }
}

/// Join a relay session with `key`, verify `Response(0)`, and return the raw stream.
pub async fn join_session<S: AsyncRead + AsyncWrite + Unpin>(
    mut stream: S,
    key: &[u8],
) -> Result<S> {
    write_message(
        &mut stream,
        &RelayMessage::JoinSessionRequest { key: key.to_vec() },
    )
    .await?;
    expect_ok_response(&mut stream, "JoinSessionRequest").await?;
    Ok(stream)
}

/// Dial a Syncthing relay over TCP, request a session for `endpoint.device_id`,
/// and open a joined bidirectional session stream.
pub async fn dial_tcp(endpoint: &SyncthingEndpoint) -> Result<TcpStream> {
    crate::policy::permit_fault(&endpoint.host, endpoint.port)?;
    let mut ctrl = TcpStream::connect(endpoint.authority())
        .await
        .with_context(|| format!("connect Syncthing relay {}", endpoint.authority()))?;
    let invitation = request_session(&mut ctrl, endpoint.device_id.as_bytes()).await?;
    let key = match invitation {
        RelayMessage::SessionInvitation { key, .. } => key,
        _ => unreachable!(),
    };
    crate::policy::permit_fault(&endpoint.host, endpoint.port)?;
    let data = TcpStream::connect(endpoint.authority())
        .await
        .with_context(|| format!("open Syncthing session socket {}", endpoint.authority()))?;
    join_session(data, &key).await
}

/// Dial a Syncthing relay over TLS using a caller-supplied rustls client configuration,
/// request a session for `endpoint.device_id`, and return the joined TLS byte stream.
pub async fn dial_tls(
    endpoint: &SyncthingEndpoint,
    tls_config: Arc<rustls::ClientConfig>,
    server_name: ServerName<'static>,
) -> Result<tokio_rustls::client::TlsStream<TcpStream>> {
    let connector = TlsConnector::from(tls_config);
    crate::policy::permit_fault(&endpoint.host, endpoint.port)?;
    let tcp_ctrl = TcpStream::connect(endpoint.authority())
        .await
        .with_context(|| format!("connect Syncthing TLS relay {}", endpoint.authority()))?;
    let mut ctrl = connector
        .connect(server_name.clone(), tcp_ctrl)
        .await
        .context("TLS handshake for Syncthing control socket")?;
    let invitation = request_session(&mut ctrl, endpoint.device_id.as_bytes()).await?;
    let key = match invitation {
        RelayMessage::SessionInvitation { key, .. } => key,
        _ => unreachable!(),
    };
    crate::policy::permit_fault(&endpoint.host, endpoint.port)?;
    let tcp_data = TcpStream::connect(endpoint.authority())
        .await
        .with_context(|| format!("connect Syncthing TLS session {}", endpoint.authority()))?;
    let data = connector
        .connect(server_name, tcp_data)
        .await
        .context("TLS handshake for Syncthing session socket")?;
    join_session(data, &key).await
}

async fn expect_ok_response<S: AsyncRead + Unpin>(stream: &mut S, op: &str) -> Result<()> {
    match read_message(stream).await? {
        RelayMessage::Response { code: 0, .. } => Ok(()),
        RelayMessage::Response { code, message } => {
            bail!("Syncthing relay refused {op} ({code}): {message}");
        }
        other => bail!("expected Syncthing Response for {op}, got {other:?}"),
    }
}

fn write_xdr_bytes(out: &mut Vec<u8>, data: &[u8]) -> Result<()> {
    let len = u32::try_from(data.len()).map_err(|_| anyhow!("XDR field exceeds u32"))?;
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(data);
    let pad = (4 - (data.len() % 4)) % 4;
    out.resize(out.len() + pad, 0);
    Ok(())
}

fn read_xdr_bytes(buf: &[u8], pos: usize) -> Result<(Vec<u8>, usize)> {
    if pos + 4 > buf.len() {
        bail!("truncated XDR length");
    }
    let len = u32::from_be_bytes([buf[pos], buf[pos + 1], buf[pos + 2], buf[pos + 3]]) as usize;
    let start = pos + 4;
    let end = start + len;
    let pad = (4 - (len % 4)) % 4;
    let next = end + pad;
    if next > buf.len() {
        bail!("truncated XDR payload");
    }
    Ok((buf[start..end].to_vec(), next))
}
