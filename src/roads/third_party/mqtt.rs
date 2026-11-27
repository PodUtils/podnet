//! MQTT 3.1.1 third-party road (`mqtt://HOST:PORT/TOPIC`) (T-368).
//!
//! Implements the MQTT 3.1.1 binary packet codec (`CONNECT`, `CONNACK`, `SUBSCRIBE`,
//! `SUBACK`, `PUBLISH`, `PINGREQ`, `PINGRESP`, `DISCONNECT`) over TCP and bridges
//! bidirectional byte streams.

use anyhow::{anyhow, bail, Context, Result};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, DuplexStream};
use tokio::net::TcpStream;

use super::irc::split_host_port;

/// Default MQTT TCP port when omitted from a URL.
pub const DEFAULT_MQTT_PORT: u16 = 1883;
/// Maximum payload bytes per MQTT `PUBLISH` chunk.
pub const MAX_PUBLISH_BYTES: usize = 16 * 1024;
/// Maximum remaining length accepted when decoding a packet (256 KiB).
pub const MAX_PACKET_BYTES: usize = 256 * 1024;

/// Parsed MQTT road endpoint (`mqtt://HOST:PORT/TOPIC`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MqttEndpoint {
    pub host: String,
    pub port: u16,
    pub tx_topic: String,
    pub rx_topic: String,
    pub client_id: String,
    pub keep_alive: u16,
}

impl MqttEndpoint {
    /// Parse `mqtt://HOST[:PORT]/TOPIC[?tx=T&rx=R&client_id=ID]`.
    pub fn parse(url: &str) -> Result<Self> {
        let rest = url
            .trim()
            .strip_prefix("mqtt://")
            .ok_or_else(|| anyhow!("an MQTT road URL must start with mqtt://"))?;
        let (path_part, query) = match rest.split_once('?') {
            Some((p, q)) => (p, q),
            None => (rest, ""),
        };
        let (authority, topic) = path_part
            .split_once('/')
            .ok_or_else(|| anyhow!("an MQTT road URL needs HOST:PORT/TOPIC"))?;
        let (host, port) = split_host_port(authority, DEFAULT_MQTT_PORT)?;
        let topic = topic.trim();
        if topic.is_empty() {
            bail!("an MQTT road URL needs a non-empty topic");
        }
        let mut tx_topic = topic.to_string();
        let mut rx_topic = topic.to_string();
        let mut client_id = default_client_id();
        for pair in query.split('&').filter(|p| !p.is_empty()) {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            match k {
                "tx" if !v.is_empty() => tx_topic = v.to_string(),
                "rx" if !v.is_empty() => rx_topic = v.to_string(),
                "client_id" if !v.is_empty() => client_id = v.to_string(),
                _ => {}
            }
        }
        Ok(Self {
            host,
            port,
            tx_topic,
            rx_topic,
            client_id,
            keep_alive: 60,
        })
    }

    /// Override transmit and receive topics for duplex peering.
    pub fn with_topics(mut self, tx_topic: impl Into<String>, rx_topic: impl Into<String>) -> Self {
        self.tx_topic = tx_topic.into();
        self.rx_topic = rx_topic.into();
        self
    }

    /// Override the MQTT client identifier.
    pub fn with_client_id(mut self, client_id: impl Into<String>) -> Self {
        self.client_id = client_id.into();
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

/// MQTT 3.1.1 control packets used by the road client and stand-in broker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Packet {
    Connect {
        client_id: String,
        keep_alive: u16,
        clean_session: bool,
    },
    ConnAck {
        session_present: bool,
        return_code: u8,
    },
    Publish {
        topic: String,
        payload: Vec<u8>,
    },
    Subscribe {
        packet_id: u16,
        topic: String,
        qos: u8,
    },
    SubAck {
        packet_id: u16,
        return_code: u8,
    },
    PingReq,
    PingResp,
    Disconnect,
}

/// Encode an MQTT 3.1.1 packet into wire bytes.
pub fn encode_packet(packet: &Packet) -> Result<Vec<u8>> {
    let (header_byte, body) = match packet {
        Packet::Connect {
            client_id,
            keep_alive,
            clean_session,
        } => {
            let mut body = Vec::new();
            write_mqtt_str(&mut body, "MQTT")?;
            body.push(4); // Protocol level 4 = MQTT 3.1.1.
            body.push(if *clean_session { 0x02 } else { 0x00 });
            body.extend_from_slice(&keep_alive.to_be_bytes());
            write_mqtt_str(&mut body, client_id)?;
            (0x10u8, body)
        }
        Packet::ConnAck {
            session_present,
            return_code,
        } => (
            0x20u8,
            vec![if *session_present { 0x01 } else { 0x00 }, *return_code],
        ),
        Packet::Publish { topic, payload } => {
            let mut body = Vec::new();
            write_mqtt_str(&mut body, topic)?;
            body.extend_from_slice(payload);
            (0x30u8, body)
        }
        Packet::Subscribe {
            packet_id,
            topic,
            qos,
        } => {
            let mut body = Vec::new();
            body.extend_from_slice(&packet_id.to_be_bytes());
            write_mqtt_str(&mut body, topic)?;
            body.push(*qos & 0x03);
            (0x82u8, body)
        }
        Packet::SubAck {
            packet_id,
            return_code,
        } => {
            let mut body = Vec::with_capacity(3);
            body.extend_from_slice(&packet_id.to_be_bytes());
            body.push(*return_code);
            (0x90u8, body)
        }
        Packet::PingReq => (0xC0u8, Vec::new()),
        Packet::PingResp => (0xD0u8, Vec::new()),
        Packet::Disconnect => (0xE0u8, Vec::new()),
    };

    let mut out = Vec::with_capacity(5 + body.len());
    out.push(header_byte);
    encode_remaining_length(body.len(), &mut out)?;
    out.extend_from_slice(&body);
    Ok(out)
}

/// Read and decode one MQTT 3.1.1 packet from `reader`.
pub async fn read_packet<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Packet> {
    let mut head = [0u8; 1];
    reader
        .read_exact(&mut head)
        .await
        .context("read MQTT fixed header")?;
    let packet_type = head[0] >> 4;
    let flags = head[0] & 0x0F;
    let remaining = read_remaining_length(reader).await?;
    if remaining > MAX_PACKET_BYTES {
        bail!("MQTT packet length {remaining} exceeds limit {MAX_PACKET_BYTES}");
    }
    let mut body = vec![0u8; remaining];
    reader
        .read_exact(&mut body)
        .await
        .context("read MQTT packet body")?;

    match packet_type {
        1 => {
            let (proto, mut pos) = read_mqtt_str(&body, 0)?;
            if proto != "MQTT" {
                bail!("unsupported MQTT protocol name {proto:?}");
            }
            if pos + 4 > body.len() {
                bail!("truncated MQTT CONNECT header");
            }
            let level = body[pos];
            if level != 4 {
                bail!("unsupported MQTT protocol level {level}");
            }
            let connect_flags = body[pos + 1];
            let keep_alive = u16::from_be_bytes([body[pos + 2], body[pos + 3]]);
            pos += 4;
            let (client_id, _) = read_mqtt_str(&body, pos)?;
            Ok(Packet::Connect {
                client_id,
                keep_alive,
                clean_session: (connect_flags & 0x02) != 0,
            })
        }
        2 => {
            if body.len() != 2 {
                bail!("invalid MQTT CONNACK length {}", body.len());
            }
            Ok(Packet::ConnAck {
                session_present: (body[0] & 0x01) != 0,
                return_code: body[1],
            })
        }
        3 => {
            let qos = (flags >> 1) & 0x03;
            let (topic, mut pos) = read_mqtt_str(&body, 0)?;
            if qos > 0 {
                if pos + 2 > body.len() {
                    bail!("truncated MQTT PUBLISH packet id");
                }
                pos += 2;
            }
            Ok(Packet::Publish {
                topic,
                payload: body[pos..].to_vec(),
            })
        }
        8 => {
            if body.len() < 4 {
                bail!("truncated MQTT SUBSCRIBE packet");
            }
            let packet_id = u16::from_be_bytes([body[0], body[1]]);
            let (topic, pos) = read_mqtt_str(&body, 2)?;
            let qos = *body
                .get(pos)
                .ok_or_else(|| anyhow!("missing QoS in MQTT SUBSCRIBE"))?;
            Ok(Packet::Subscribe {
                packet_id,
                topic,
                qos,
            })
        }
        9 => {
            if body.len() < 3 {
                bail!("truncated MQTT SUBACK packet");
            }
            Ok(Packet::SubAck {
                packet_id: u16::from_be_bytes([body[0], body[1]]),
                return_code: body[2],
            })
        }
        12 => Ok(Packet::PingReq),
        13 => Ok(Packet::PingResp),
        14 => Ok(Packet::Disconnect),
        other => bail!("unsupported MQTT packet type {other}"),
    }
}

/// Write one MQTT 3.1.1 packet to `writer`.
pub async fn write_packet<W: AsyncWrite + Unpin>(writer: &mut W, packet: &Packet) -> Result<()> {
    let bytes = encode_packet(packet)?;
    writer
        .write_all(&bytes)
        .await
        .context("write MQTT packet")?;
    writer.flush().await.context("flush MQTT packet")
}

/// Connected MQTT 3.1.1 road client ready for ping, publish, receive, or stream bridging.
pub struct MqttClient {
    stream: TcpStream,
    endpoint: MqttEndpoint,
}

impl MqttClient {
    /// Connect to an MQTT 3.1.1 broker, run `CONNECT`/`CONNACK` and `SUBSCRIBE`/`SUBACK`.
    pub async fn connect(endpoint: &MqttEndpoint) -> Result<Self> {
        let mut stream = TcpStream::connect(endpoint.authority())
            .await
            .with_context(|| format!("connect MQTT road {}", endpoint.authority()))?;
        write_packet(
            &mut stream,
            &Packet::Connect {
                client_id: endpoint.client_id.clone(),
                keep_alive: endpoint.keep_alive,
                clean_session: true,
            },
        )
        .await?;
        match read_packet(&mut stream).await? {
            Packet::ConnAck { return_code: 0, .. } => {}
            Packet::ConnAck { return_code, .. } => {
                bail!("MQTT broker refused CONNECT with code {return_code}");
            }
            other => bail!("expected MQTT CONNACK, got {other:?}"),
        }
        write_packet(
            &mut stream,
            &Packet::Subscribe {
                packet_id: 1,
                topic: endpoint.rx_topic.clone(),
                qos: 0,
            },
        )
        .await?;
        match read_packet(&mut stream).await? {
            Packet::SubAck {
                packet_id: 1,
                return_code,
            } if return_code != 0x80 => {}
            Packet::SubAck { return_code, .. } => {
                bail!("MQTT broker refused SUBSCRIBE with code {return_code:#x}");
            }
            other => bail!("expected MQTT SUBACK, got {other:?}"),
        }
        Ok(Self {
            stream,
            endpoint: endpoint.clone(),
        })
    }

    /// Send `PINGREQ` and wait for `PINGRESP`.
    pub async fn ping(&mut self) -> Result<()> {
        write_packet(&mut self.stream, &Packet::PingReq).await?;
        match read_packet(&mut self.stream).await? {
            Packet::PingResp => Ok(()),
            other => bail!("expected MQTT PINGRESP, got {other:?}"),
        }
    }

    /// Convert this connected MQTT client into a bidirectional `AsyncRead + AsyncWrite` stream.
    pub fn into_stream(self) -> MqttStream {
        let (mut tcp_read, mut tcp_write) = tokio::io::split(self.stream);
        let (client_io, bridge_io) = tokio::io::duplex(64 * 1024);
        let (mut bridge_read, mut bridge_write) = tokio::io::split(bridge_io);
        let tx_topic = self.endpoint.tx_topic;
        let rx_topic = self.endpoint.rx_topic;

        tokio::spawn(async move {
            while let Ok(pkt) = read_packet(&mut tcp_read).await {
                match pkt {
                    Packet::Publish { topic, payload } if topic == rx_topic => {
                        if payload.is_empty() {
                            let _ = bridge_write.shutdown().await;
                            break;
                        }
                        if bridge_write.write_all(&payload).await.is_err() {
                            break;
                        }
                    }
                    Packet::Disconnect => break,
                    _ => {}
                }
            }
            let _ = bridge_write.shutdown().await;
        });

        tokio::spawn(async move {
            let mut buf = vec![0u8; MAX_PUBLISH_BYTES];
            loop {
                match bridge_read.read(&mut buf).await {
                    Ok(0) => {
                        let _ = write_packet(
                            &mut tcp_write,
                            &Packet::Publish {
                                topic: tx_topic.clone(),
                                payload: Vec::new(),
                            },
                        )
                        .await;
                        let _ = write_packet(&mut tcp_write, &Packet::Disconnect).await;
                        break;
                    }
                    Ok(n) => {
                        let pkt = Packet::Publish {
                            topic: tx_topic.clone(),
                            payload: buf[..n].to_vec(),
                        };
                        if write_packet(&mut tcp_write, &pkt).await.is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        MqttStream { inner: client_io }
    }
}

/// Bidirectional byte stream carried over MQTT 3.1.1 `PUBLISH` packets.
pub struct MqttStream {
    inner: DuplexStream,
}

impl MqttStream {
    /// Connect and immediately return a bidirectional byte stream.
    pub async fn connect(endpoint: &MqttEndpoint) -> Result<Self> {
        Ok(MqttClient::connect(endpoint).await?.into_stream())
    }
}

impl AsyncRead for MqttStream {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for MqttStream {
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

fn write_mqtt_str(out: &mut Vec<u8>, s: &str) -> Result<()> {
    let len = u16::try_from(s.len()).map_err(|_| anyhow!("MQTT string exceeds 65535 bytes"))?;
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(s.as_bytes());
    Ok(())
}

fn read_mqtt_str(buf: &[u8], pos: usize) -> Result<(String, usize)> {
    if pos + 2 > buf.len() {
        bail!("truncated MQTT string length");
    }
    let len = u16::from_be_bytes([buf[pos], buf[pos + 1]]) as usize;
    let start = pos + 2;
    let end = start + len;
    if end > buf.len() {
        bail!("truncated MQTT string body");
    }
    let text = std::str::from_utf8(&buf[start..end])
        .context("invalid UTF-8 in MQTT string")?
        .to_string();
    Ok((text, end))
}

fn encode_remaining_length(mut len: usize, out: &mut Vec<u8>) -> Result<()> {
    if len > 268_435_455 {
        bail!("MQTT remaining length exceeds protocol maximum");
    }
    loop {
        let mut byte = (len % 128) as u8;
        len /= 128;
        if len > 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if len == 0 {
            break;
        }
    }
    Ok(())
}

async fn read_remaining_length<R: AsyncRead + Unpin>(reader: &mut R) -> Result<usize> {
    let mut multiplier: usize = 1;
    let mut value: usize = 0;
    for _ in 0..4 {
        let mut byte = [0u8; 1];
        reader
            .read_exact(&mut byte)
            .await
            .context("read MQTT remaining length")?;
        value += usize::from(byte[0] & 0x7F) * multiplier;
        if (byte[0] & 0x80) == 0 {
            return Ok(value);
        }
        multiplier *= 128;
    }
    bail!("malformed MQTT remaining length")
}

fn default_client_id() -> String {
    let mut raw = [0u8; 4];
    let _ = getrandom::fill(&mut raw);
    format!(
        "podnet-{:02x}{:02x}{:02x}{:02x}",
        raw[0], raw[1], raw[2], raw[3]
    )
}
