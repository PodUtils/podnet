//! The Juicity road of podnet (PodUtils/podnet#486, QUIC-10): a client of
//! the Juicity protocol over the QUIC stack of `pod-quic`.
//!
//! The layout is the one of `juicity/juicity`'s reference implementation,
//! which `docs/spec_en.md` does not describe exactly: the version and the
//! command on a unidirectional stream, the token of the TLS exporter, and
//! a proxy header. TCP is a bidirectional stream that starts with the
//! network, the address and the port. UDP is one bidirectional stream for
//! each source triplet: the stream starts with the network and the address
//! of the flow, and each datagram then repeats the address and the port,
//! with no network byte, and its 2-byte length.
//!
//! The address types are the ones of the reference implementation (the
//! Trojan types: 1 IPv4, 4 IPv6, 3 a name), not the 0, 1 and 2 of
//! `docs/spec_en.md`. The reference server of v0.5.0 reads the Trojan
//! types, and it refuses the types of the document; see `docs/quic.md`.
//!
//! The client opens no second QUIC stack (QUIC-10): it uses the endpoint,
//! the TLS and the settings of `pod-quic`. The BBR requirement of the
//! specification is a server-side requirement; the client leaves the
//! controller of noq at its default.

use std::fmt;
use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};
use pod_quic::noq;
use pod_quic::noq::crypto::rustls::QuicClientConfig;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use tokio::sync::{mpsc, Mutex};

/// The version of the protocol.
pub const VERSION: u8 = 0x00;
/// The command of the authentication.
pub const AUTHENTICATE: u8 = 0x00;
/// The networks of the reference wire.
pub const TCP: u8 = 0x01;
pub const UDP: u8 = 0x03;
/// The address types of the reference wire (the Trojan types).
pub const IPV4: u8 = 0x01;
pub const IPV6: u8 = 0x04;
pub const DOMAIN: u8 = 0x03;
/// The length of the token.
pub const TOKEN_LEN: usize = 32;
/// The ALPN of the protocol.
pub const ALPN: &str = "h3";
/// The streams that one QUIC connection carries before the client opens the
/// next one: `docs/spec_en.md` asks for a new connection after a cumulative
/// total of 30 streams on the same connection, because a server's
/// `maxOpenIncomingStreams` is not reported to the client.
pub const STREAM_LIMIT: usize = 30;

/// A link: `juicity://UUID:PASSWORD@HOST:PORT?PARAM`.
#[derive(Clone)]
pub struct Link {
    pub uuid: uuid::Uuid,
    pub password: String,
    pub host: String,
    pub port: u16,
    pub sni: String,
    pub allow_insecure: bool,
}

impl fmt::Debug for Link {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Link")
            .field("uuid", &self.uuid)
            .field("password", &"<redacted>")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("sni", &self.sni)
            .finish()
    }
}

impl Link {
    pub fn parse(text: &str) -> Result<Link> {
        let rest = text
            .strip_prefix("juicity://")
            .ok_or_else(|| anyhow!("a Juicity link starts with juicity://"))?;
        let (authority, query) = match rest.split_once('?') {
            Some((a, q)) => (a, q),
            None => (rest, ""),
        };
        let (user, hostport) = authority
            .rsplit_once('@')
            .ok_or_else(|| anyhow!("a Juicity link has UUID:PASSWORD@HOST:PORT"))?;
        let (uuid, password) = user
            .split_once(':')
            .ok_or_else(|| anyhow!("a Juicity link has UUID:PASSWORD@HOST:PORT"))?;
        let uuid =
            uuid::Uuid::parse_str(uuid).with_context(|| format!("the UUID of the link: {uuid}"))?;
        let (host, port) = split_host_port(hostport)?;
        let mut link = Link {
            uuid,
            password: percent(password),
            host,
            port,
            sni: String::new(),
            allow_insecure: false,
        };
        for pair in query.split('&').filter(|p| !p.is_empty()) {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            match key {
                "sni" | "peer" => link.sni = percent(value),
                "allow_insecure" | "allowInsecure" => link.allow_insecure = percent(value) == "1",
                "congestion_control" | "congestion_control_type" => {}
                other => bail!("the Juicity link has no parameter {other:?}"),
            }
        }
        if link.sni.is_empty() {
            link.sni = link.host.clone();
        }
        Ok(link)
    }
}

fn split_host_port(text: &str) -> Result<(String, u16)> {
    if let Some(rest) = text.strip_prefix('[') {
        let (host, tail) = rest
            .split_once(']')
            .ok_or_else(|| anyhow!("an IPv6 link without ]"))?;
        let port = tail
            .strip_prefix(':')
            .ok_or_else(|| anyhow!("a Juicity link has UUID:PASSWORD@HOST:PORT"))?;
        return Ok((
            host.to_string(),
            port.parse().context("the port of the link")?,
        ));
    }
    let (host, port) = text
        .rsplit_once(':')
        .ok_or_else(|| anyhow!("a Juicity link has UUID:PASSWORD@HOST:PORT"))?;
    Ok((
        host.to_string(),
        port.parse().context("the port of the link")?,
    ))
}

fn percent(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3])
                .ok()
                .and_then(|h| u8::from_str_radix(h, 16).ok());
            if let Some(byte) = hex {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The proxy header of one request: the network (TCP only), the address
/// type, the address and the port, in that order and in network order. A
/// UDP datagram of the reference wire has no network byte.
pub fn proxy_header(network: u8, host: &str, port: u16) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    match network {
        TCP => out.push(TCP),
        UDP => {}
        other => bail!("the network {other} is not TCP or UDP"),
    }
    let bare = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    match bare.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(v4)) => {
            out.push(IPV4);
            out.extend_from_slice(&v4.octets());
        }
        Ok(std::net::IpAddr::V6(v6)) => {
            out.push(IPV6);
            out.extend_from_slice(&v6.octets());
        }
        Err(_) => {
            let len =
                u8::try_from(bare.len()).map_err(|_| anyhow!("a name of more than 255 bytes"))?;
            if len == 0 {
                bail!("an empty name");
            }
            out.push(DOMAIN);
            out.push(len);
            out.extend_from_slice(bare.as_bytes());
        }
    }
    out.extend_from_slice(&port.to_be_bytes());
    Ok(out)
}

/// The length of one address header at the front of `bytes`, with no
/// network byte (the shape of a UDP datagram of the reference wire).
pub fn header_len(bytes: &[u8]) -> Option<usize> {
    let kind = *bytes.first()?;
    let rest = match kind {
        IPV4 => 4,
        IPV6 => 16,
        DOMAIN => 1 + usize::from(*bytes.get(1)?),
        _ => return None,
    };
    Some(1 + rest + 2)
}

/// One authenticated QUIC connection of the pool.
struct Connection {
    conn: noq::Connection,
    /// The bidirectional streams opened on this connection: `tcp` and
    /// `udp` each take one.
    opened: usize,
}

/// The connections of one client: the current one and the count of all
/// that were opened. A retired connection is dropped; its own streams hold
/// the QUIC connection alive until they end.
struct State {
    current: Connection,
    connections: usize,
}

/// The client of one Juicity link. The first QUIC connection is opened by
/// [`Juicity::connect`]; a stream beyond [`STREAM_LIMIT`] opens another, and
/// a retired connection closes when its last stream ends.
pub struct Juicity {
    link: Link,
    endpoint: noq::Endpoint,
    state: Mutex<State>,
}

impl Juicity {
    pub async fn connect(link: &Link) -> Result<Juicity> {
        let endpoint =
            pod_quic::endpoint::system(false).context("the endpoint of the Juicity client")?;
        let current = open(link, &endpoint).await?;
        Ok(Juicity {
            link: link.clone(),
            endpoint,
            state: Mutex::new(State {
                current,
                connections: 1,
            }),
        })
    }

    /// The number of QUIC connections opened so far: one at
    /// [`Juicity::connect`], and one more at each [`STREAM_LIMIT`] streams.
    pub async fn connections(&self) -> usize {
        self.state.lock().await.connections
    }

    /// The connection for the next stream: the current one, or a new one
    /// when the current one has carried [`STREAM_LIMIT`] streams.
    async fn stream_connection(&self) -> Result<noq::Connection> {
        let mut state = self.state.lock().await;
        if state.current.opened >= STREAM_LIMIT {
            let next = open(&self.link, &self.endpoint).await?;
            let old = std::mem::replace(&mut state.current, next);
            drop(old);
            state.connections += 1;
        }
        state.current.opened += 1;
        Ok(state.current.conn.clone())
    }

    /// A TCP stream to `host:port`, its proxy header already written.
    pub async fn tcp(&self, host: &str, port: u16) -> Result<(noq::SendStream, noq::RecvStream)> {
        let conn = self.stream_connection().await?;
        let (mut send, recv) = conn.open_bi().await.context("a stream of Juicity")?;
        send.write_all(&proxy_header(TCP, host, port)?)
            .await
            .context("the proxy header of Juicity")?;
        Ok((send, recv))
    }

    /// A UDP flow to `host:port`: one stream. The stream starts with the
    /// network and the address of the flow; each datagram then repeats the
    /// address and its length (the reference wire).
    pub async fn udp(&self, host: &str, port: u16) -> Result<Udp> {
        let conn = self.stream_connection().await?;
        let (mut send, mut recv) = conn.open_bi().await.context("a UDP stream of Juicity")?;
        let header = proxy_header(UDP, host, port)?;
        let mut flow = Vec::with_capacity(1 + header.len());
        flow.push(UDP);
        flow.extend_from_slice(&header);
        send.write_all(&flow)
            .await
            .context("the flow header of Juicity")?;
        let (tx, rx) = mpsc::channel(64);
        let reader = tokio::spawn(async move {
            loop {
                // The address type, the address and the port, then the
                // 2-byte length and the payload. A UDP datagram of the
                // reference wire has no network byte.
                let mut kind = [0u8; 1];
                if recv.read_exact(&mut kind).await.is_err() {
                    return;
                }
                let extra = match kind[0] {
                    IPV4 => 4,
                    IPV6 => 16,
                    DOMAIN => {
                        let mut len = [0u8; 1];
                        if recv.read_exact(&mut len).await.is_err() {
                            return;
                        }
                        usize::from(len[0])
                    }
                    _ => return,
                };
                let mut rest = vec![0u8; extra + 2];
                if recv.read_exact(&mut rest).await.is_err() {
                    return;
                }
                let mut len = [0u8; 2];
                if recv.read_exact(&mut len).await.is_err() {
                    return;
                }
                let mut payload = vec![0u8; usize::from(u16::from_be_bytes(len))];
                if recv.read_exact(&mut payload).await.is_err() {
                    return;
                }
                if tx.send(payload).await.is_err() {
                    return;
                }
            }
        });
        Ok(Udp {
            send: Mutex::new(send),
            header,
            rx: Mutex::new(rx),
            reader,
        })
    }
}

/// One UDP flow of Juicity.
pub struct Udp {
    send: Mutex<noq::SendStream>,
    header: Vec<u8>,
    rx: Mutex<mpsc::Receiver<Vec<u8>>>,
    reader: tokio::task::JoinHandle<()>,
}

impl Udp {
    pub async fn send(&self, payload: &[u8]) -> Result<()> {
        if payload.len() > u16::MAX as usize {
            bail!(
                "a Juicity datagram of {} bytes (the most is 65535)",
                payload.len()
            );
        }
        let mut out = Vec::with_capacity(self.header.len() + 2 + payload.len());
        out.extend_from_slice(&self.header);
        out.extend_from_slice(&(payload.len() as u16).to_be_bytes());
        out.extend_from_slice(payload);
        self.send
            .lock()
            .await
            .write_all(&out)
            .await
            .context("a Juicity datagram")
    }

    pub async fn recv(&self) -> Option<Vec<u8>> {
        self.rx.lock().await.recv().await
    }
}

impl Drop for Udp {
    fn drop(&mut self) {
        self.reader.abort();
    }
}

/// Open one QUIC connection on `endpoint` and authenticate it with the
/// token of its TLS exporter.
async fn open(link: &Link, endpoint: &noq::Endpoint) -> Result<Connection> {
    let settings = pod_quic::Settings::default();
    let tls = tls(link)?;
    let crypto = QuicClientConfig::try_from(tls).context("the TLS of Juicity with QUIC")?;
    let mut config = noq::ClientConfig::new(Arc::new(crypto));
    config.transport_config(Arc::new(settings.transport(false)));
    let addr = resolve(&link.host, link.port).await?;
    let connecting = endpoint
        .connect_with(config, addr, &link.sni)
        .with_context(|| format!("the Juicity connection to {addr}"))?;
    let conn = tokio::time::timeout(settings.handshake_wait, connecting)
        .await
        .context("no answer of the Juicity server")?
        .with_context(|| format!("the Juicity handshake with {addr}"))?;
    let mut token = [0u8; TOKEN_LEN];
    conn.export_keying_material(&mut token, link.uuid.as_bytes(), link.password.as_bytes())
        .map_err(|e| anyhow!("the token of Juicity: {e:?}"))?;
    let mut auth = conn
        .open_uni()
        .await
        .context("the authentication stream of Juicity")?;
    let mut bytes = Vec::with_capacity(2 + 16 + TOKEN_LEN);
    bytes.extend_from_slice(&[VERSION, AUTHENTICATE]);
    bytes.extend_from_slice(link.uuid.as_bytes());
    bytes.extend_from_slice(&token);
    auth.write_all(&bytes)
        .await
        .context("the authentication of Juicity")?;
    // The authentication is one message: its end tells the server that no
    // more comes, and the connection lives with its data streams.
    auth.finish()
        .context("the end of the authentication of Juicity")?;
    Ok(Connection { conn, opened: 0 })
}

fn tls(link: &Link) -> Result<Arc<rustls::ClientConfig>> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let alpn = vec![ALPN.as_bytes().to_vec()];
    if link.allow_insecure {
        let mut config = rustls::ClientConfig::builder_with_provider(provider.clone())
            .with_protocol_versions(&[&rustls::version::TLS13])
            .context("the TLS 1.3 of Juicity")?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AcceptAny(provider)))
            .with_no_client_auth();
        config.alpn_protocols = alpn;
        return Ok(Arc::new(config));
    }
    let roots = Arc::new(rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    });
    pod_quic::connect::tls(provider, roots, &[ALPN.as_bytes()]).map_err(|e| anyhow!(e))
}

#[derive(Debug)]
struct AcceptAny(Arc<rustls::crypto::CryptoProvider>);

impl ServerCertVerifier for AcceptAny {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

async fn resolve(host: &str, port: u16) -> Result<std::net::SocketAddr> {
    if let Ok(addr) = host.parse::<std::net::IpAddr>() {
        return Ok(std::net::SocketAddr::new(addr, port));
    }
    let mut addrs = tokio::net::lookup_host((host, port))
        .await
        .with_context(|| format!("the address of {host}"))?;
    addrs.next().ok_or_else(|| anyhow!("no address of {host}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_link_and_the_header_have_the_layout_of_the_specification() {
        let link = Link::parse(
            "juicity://123e4567-e89b-12d3-a456-426614174000:pw@example.test:443?sni=edge.test",
        )
        .unwrap();
        assert_eq!(link.sni, "edge.test");
        assert_eq!(
            proxy_header(TCP, "192.0.2.7", 443).unwrap(),
            [0x01, 0x01, 192, 0, 2, 7, 0x01, 0xbb]
        );
        let v6 = proxy_header(UDP, "[2001:db8::1]", 53).unwrap();
        assert_eq!(v6[0], IPV6);
        assert_eq!(
            v6[1..17],
            "2001:db8::1"
                .parse::<std::net::Ipv6Addr>()
                .unwrap()
                .octets()
        );
        assert_eq!(v6[17..], [0, 53]);
        let mut name = proxy_header(TCP, "example.test", 22).unwrap();
        assert_eq!(name[..3], [0x01, 0x03, 12]);
        name.drain(..3 + 12);
        assert_eq!(name, [0, 22]);
        assert_eq!(
            header_len(&proxy_header(UDP, "example.test", 53).unwrap()),
            Some(16)
        );
        assert!(Link::parse("juicity://h:1").is_err());
    }
}
