//! The L4 door of `warp proxy --l4` (issue 140, T-002): one HTTP/3 QUIC
//! connection carrying a Stream 0 `cf-connect-ip` session bootstrap followed
//! by one HTTP/3 `CONNECT` stream per TCP flow, matching official
//! `cloudflare-warp` 2026.8.2115.0 (`consumer-masque-proxy.cloudflareclient.com`).
//!
//! Because Cloudflare's L4 door requires IP-literal `:authority` values on
//! per-flow `CONNECT` streams, domain targets are resolved over DoH by IP
//! literal through the L4 door itself (honoring `--resolve` pins first) with
//! zero host DNS leaks.

use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, OnceLock, Weak};
use std::time::Duration;

use pod_core::{BoxFuture, Class, Connected, Dial, Failure, Stage, Target};
use pod_quic::noq;
use pod_warp::h3::{self, frame, DataFrames, H3Conn, FRAME_DATA};
use pod_warp::udp::UdpWay;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _, DuplexStream};
use tokio::time::Instant;

pub use pod_warp::l4::{authority, connect_headers_form, Form, OPEN_WAIT, PIPE_BUFFER};
pub use pod_warp::service::l4_sni;

/// The variable of the form of the CONNECT request (`plain`, `extended` or
/// `header`; another value is a configuration error).
pub const FORM_VARIABLE: &str = "PODNET_WARP_L4_FORM";

/// The most bytes of one read of the local pipe, sent as one DATA frame.
const UP_CHUNK: usize = 32 * 1024;
/// The most bytes of one read of the HTTP/3 stream.
const DOWN_CHUNK: usize = 64 * 1024;
/// Time limit for resolving a target hostname over DoH through the L4 door.
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(10);

fn failure(class: Class, text: impl Into<String>) -> Failure {
    Failure::new(Stage::Warp, class, text)
}

/// Prefixed integer (RFC 7541 section 5.1) for static-table QPACK encoding.
fn put_prefixed(buf: &mut Vec<u8>, flags: u8, bits: u8, mut v: u64) {
    let max = (1u64 << bits) - 1;
    if v < max {
        buf.push(flags | (v as u8));
        return;
    }
    buf.push(flags | (max as u8));
    v -= max;
    while v >= 128 {
        buf.push(((v as u8) & 0x7f) | 0x80);
        v >>= 7;
    }
    buf.push(v as u8);
}

/// Uncompressed QPACK string literal.
fn put_string(buf: &mut Vec<u8>, flags: u8, bits: u8, s: &str) {
    put_prefixed(buf, flags, bits, s.len() as u64);
    buf.extend_from_slice(s.as_bytes());
}

/// Host component of `pod_warp::service::connect_uri()` (`cloudflareaccess.com`).
pub fn bootstrap_authority() -> &'static str {
    let uri = pod_warp::service::connect_uri();
    let rest = uri
        .strip_prefix("https://")
        .or_else(|| uri.strip_prefix("http://"))
        .unwrap_or(uri);
    rest.split('/').next().unwrap_or(rest)
}

/// QPACK field section for the Stream 0 L4 session bootstrap, captured from
/// official `cloudflare-warp` 2026.8.2115.0 in `WarpProxy` mode:
/// `:method CONNECT`, `:protocol cf-connect-ip`, `:scheme http`,
/// `:authority <authority>`, `:path /`, `pq-enabled false`,
/// `cf-client-version <client_version>`.
pub fn bootstrap_headers(authority: &str, client_version: &str) -> Vec<u8> {
    // Required Insert Count 0, Base 0; :method CONNECT (static 15).
    let mut out = vec![0x00, 0x00, 0xc0 | 15];
    // :protocol cf-connect-ip (literal field line with literal name).
    put_string(&mut out, 0x20, 3, ":protocol");
    put_string(&mut out, 0x00, 7, "cf-connect-ip");
    // :scheme http (static 22).
    out.push(0xc0 | 22);
    // :authority (static name 0), literal value.
    put_prefixed(&mut out, 0x50, 4, 0);
    put_string(&mut out, 0x00, 7, authority);
    // :path / (static 1).
    out.push(0xc0 | 1);
    for (name, value) in [
        ("pq-enabled", "false"),
        ("cf-client-version", client_version),
    ] {
        put_string(&mut out, 0x20, 3, name);
        put_string(&mut out, 0x00, 7, value);
    }
    out
}

/// An open QUIC/HTTP/3 connection to the L4 door with its Stream 0 bootstrap
/// kept alive for the lifetime of the connection.
struct L4Conn {
    h3: H3Conn,
    _bootstrap: Option<noq::SendStream>,
}

struct L4Inner {
    door: SocketAddr,
    udp: Arc<dyn UdpWay>,
    tls: Arc<rustls::ClientConfig>,
    sni: String,
    form: Form,
    wait: Duration,
    bootstrap: bool,
    resolve_names: bool,
    conn: tokio::sync::Mutex<Option<L4Conn>>,
    resolver: OnceLock<pod_dns::Resolver>,
}

/// Internal `Dial` adapter used by DoH queries inside the L4 door. Only dials
/// IP literals (`Target::Addr`) so DNS lookups can never recurse.
struct AddrOnlyL4(Weak<L4Inner>);

impl Dial for AddrOnlyL4 {
    fn label(&self) -> String {
        "the L4 door of WARP (DoH)".into()
    }

    fn dial<'a>(
        &'a self,
        target: &'a Target,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(async move {
            let Target::Addr(addr) = *target else {
                return Err(failure(
                    Class::Usage,
                    format!("DoH inside the L4 door takes an IP address, not {target}"),
                ));
            };
            let Some(inner) = self.0.upgrade() else {
                return Err(failure(Class::Unavailable, "the L4 door closed"));
            };
            let pipe = tokio::time::timeout_at(
                deadline,
                inner.open_authority(&authority(&addr.ip().to_string(), addr.port())),
            )
            .await
            .map_err(|_| {
                failure(
                    Class::Timeout,
                    format!("DoH CONNECT {addr} through the L4 door: no answer in time"),
                )
            })??;
            Ok(Connected::new(Box::new(pipe)))
        })
    }

    fn names_inside(&self) -> bool {
        false
    }
}

impl L4Inner {
    async fn connection(&self) -> Result<noq::Connection, Failure> {
        let mut slot = self.conn.lock().await;
        if let Some(open) = slot.as_ref() {
            if open.h3.conn.close_reason().is_none() {
                return Ok(open.h3.conn.clone());
            }
        }
        let datagrams = self.udp.datagrams(self.door).await?;
        let fresh = h3::open(self.door, datagrams, self.tls.clone(), &self.sni, self.wait).await?;
        let bootstrap = if self.bootstrap {
            let headers =
                bootstrap_headers(bootstrap_authority(), pod_warp::service::client_version());
            let (send, mut recv, _frames, status) =
                h3::request(&fresh.conn, &headers).await.map_err(|e| {
                    Failure::caused(
                        e.stage,
                        e.class,
                        "establishing the L4 MASQUE session on stream 0",
                        e,
                    )
                })?;
            if status != 200 {
                let class = match status {
                    400..=499 => Class::Refused,
                    500..=599 => Class::Unavailable,
                    _ => Class::Protocol,
                };
                return Err(failure(
                    class,
                    format!(
                        "the L4 door refused the stream 0 cf-connect-ip bootstrap with {status}"
                    ),
                ));
            }
            tokio::spawn(
                async move { while let Ok(Some(_)) = recv.read_chunk(DOWN_CHUNK).await {} },
            );
            Some(send)
        } else {
            None
        };
        let conn = fresh.conn.clone();
        *slot = Some(L4Conn {
            h3: fresh,
            _bootstrap: bootstrap,
        });
        let way = self.udp.label();
        tracing::info!(
            door = %self.door,
            sni = %self.sni,
            %way,
            bootstrap = self.bootstrap,
            "the QUIC connection of the L4 door is open"
        );
        Ok(conn)
    }

    async fn open_authority(&self, auth: &str) -> Result<DuplexStream, Failure> {
        let conn = self.connection().await?;
        let (send, recv, frames, status) =
            h3::request(&conn, &connect_headers_form(auth, self.form))
                .await
                .map_err(|e| {
                    Failure::caused(
                        e.stage,
                        e.class,
                        format!("CONNECT {auth} through the L4 door"),
                        e,
                    )
                })?;
        if status != 200 {
            let class = match status {
                400..=499 => Class::Refused,
                500..=599 => Class::Unavailable,
                _ => Class::Protocol,
            };
            return Err(failure(
                class,
                format!("the L4 door refused CONNECT {auth} with {status}"),
            ));
        }
        let (local, remote) = tokio::io::duplex(PIPE_BUFFER);
        tokio::spawn(pump(send, recv, frames.into_rest(), remote));
        Ok(local)
    }

    fn doh_resolver(self: &Arc<Self>) -> &pod_dns::Resolver {
        self.resolver.get_or_init(|| {
            let mut settings = crate::resolve::settings();
            // Do not leak names to the host's system resolver; resolve over
            // IP-literal DoH through the L4 door itself, cached in memory.
            settings.system = None;
            settings.memory = true;
            settings.names = pod_dns::Names::Last;
            let dial: Arc<dyn Dial> = Arc::new(AddrOnlyL4(Arc::downgrade(self)));
            pod_dns::Resolver::new(settings).with_doh(crate::trust::context().clone(), dial)
        })
    }

    async fn resolve_host(self: &Arc<Self>, host: &str) -> Result<Vec<IpAddr>, Failure> {
        if let Ok(ip) = host.parse::<IpAddr>() {
            return Ok(vec![ip]);
        }
        let pins = crate::resolve::pinned(host);
        if !pins.is_empty() {
            return Ok(pins);
        }
        let deadline = Instant::now() + RESOLVE_TIMEOUT;
        let ips = self.doh_resolver().ips(host, deadline).await.map_err(|e| {
            failure(
                Class::Unavailable,
                format!("resolving {host} through the L4 door: {e}"),
            )
        })?;
        if ips.is_empty() {
            return Err(failure(
                Class::NotFound,
                format!("no address for {host} through the L4 door"),
            ));
        }
        // Prefer IPv4 first to match the tunnel stack ordering.
        let mut ordered = ips;
        ordered.sort_by_key(|ip| ip.is_ipv6());
        Ok(ordered)
    }
}

/// The L4 door: one QUIC/HTTP/3 connection with a Stream 0 `cf-connect-ip`
/// bootstrap, opened again when it closes. Each TCP flow is one HTTP/3
/// `CONNECT` stream with an IP-literal `:authority`.
#[derive(Clone)]
pub struct L4 {
    inner: Arc<L4Inner>,
}

impl std::fmt::Debug for L4 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let way = self.inner.udp.label();
        f.debug_struct("L4")
            .field("door", &self.inner.door)
            .field("way", &way)
            .field("sni", &self.inner.sni)
            .field("bootstrap", &self.inner.bootstrap)
            .finish_non_exhaustive()
    }
}

impl L4 {
    /// The door at `door` on `udp`, with TLS `tls` (client certificate,
    /// pinned peer key, ALPN `h3`), SNI `l4_sni()`, `Form::Plain`,
    /// `OPEN_WAIT`, Stream 0 `cf-connect-ip` bootstrap, and DoH name
    /// resolution through the L4 door enabled.
    pub fn new(door: SocketAddr, udp: Arc<dyn UdpWay>, tls: Arc<rustls::ClientConfig>) -> Self {
        Self {
            inner: Arc::new(L4Inner {
                door,
                udp,
                tls,
                sni: l4_sni().to_string(),
                form: Form::Plain,
                wait: OPEN_WAIT,
                bootstrap: true,
                resolve_names: true,
                conn: tokio::sync::Mutex::new(None),
                resolver: OnceLock::new(),
            }),
        }
    }

    /// Build an L4 door from a WARP device, identity, crypto provider, door
    /// selection, SNI, and optional `PODNET_WARP_L4_FORM` override.
    pub fn from_device(
        device: &super::Device,
        identity: &super::Identity,
        provider: Arc<rustls::crypto::CryptoProvider>,
        door_setting: &pod_warp::Door,
        sni: &str,
        form_override: Option<&str>,
    ) -> anyhow::Result<Self> {
        use super::Warped as _;
        use anyhow::Context as _;
        let door = match door_setting {
            pod_warp::Door::Pinned(Target::Addr(addr)) => *addr,
            pod_warp::Door::Pinned(name) => {
                anyhow::bail!("the L4 door takes the address of a door, not {name}");
            }
            pod_warp::Door::Record => device
                .masque_endpoints()
                .into_iter()
                .next()
                .context("the device has no front door for the L4 door")?,
        };
        let pinned = device
            .peer_public_key_pem()
            .context("the device has no peer key to pin; podnet does not connect without one")
            .and_then(|pem| super::identity::spki_from_pem(pem).warped())
            .context("parsing the pinned peer key")?;
        let tls = pod_warp::tls::client_config(provider, identity, pinned, h3::ALPN).warped()?;
        let udp: Arc<dyn UdpWay> = Arc::new(pod_warp::SystemUdp);
        let mut l4 = Self::new(door, udp, tls).with_sni(sni);
        if let Some(value) = form_override.filter(|v| !v.is_empty()) {
            l4 = l4.with_form(Form::parse(value).warped()?);
        }
        Ok(l4)
    }

    fn inner_mut(&mut self) -> &mut L4Inner {
        Arc::get_mut(&mut self.inner).expect("L4 builder methods run before cloning")
    }

    /// Set the TLS server name (`--l4-sni`).
    pub fn with_sni(mut self, sni: &str) -> Self {
        self.inner_mut().sni = sni.to_string();
        self
    }

    /// Set the form of each per-flow `CONNECT`.
    pub fn with_form(mut self, form: Form) -> Self {
        self.inner_mut().form = form;
        self
    }

    /// Set the handshake timeout.
    pub fn with_wait(mut self, wait: Duration) -> Self {
        self.inner_mut().wait = wait;
        self
    }

    /// Enable or disable the Stream 0 `cf-connect-ip` session bootstrap.
    pub fn with_bootstrap(mut self, bootstrap: bool) -> Self {
        self.inner_mut().bootstrap = bootstrap;
        self
    }

    /// Enable or disable DoH-over-L4 IP resolution for `Target::Name`.
    pub fn with_resolve_names(mut self, resolve_names: bool) -> Self {
        self.inner_mut().resolve_names = resolve_names;
        self
    }

    /// The address of the L4 door.
    pub fn door(&self) -> SocketAddr {
        self.inner.door
    }

    /// The open QUIC connection: opens a new one (and runs Stream 0 bootstrap
    /// when enabled) if none is open or the previous one closed.
    pub async fn connection(&self) -> Result<noq::Connection, Failure> {
        self.inner.connection().await
    }

    /// Open a TCP flow to `target` over a dedicated HTTP/3 `CONNECT` stream.
    pub async fn open(&self, target: &Target) -> Result<DuplexStream, Failure> {
        match target {
            Target::Addr(addr) => {
                let auth = authority(&addr.ip().to_string(), addr.port());
                self.inner.open_authority(&auth).await
            }
            Target::Name { host, port } if self.inner.resolve_names => {
                let ips = self.inner.resolve_host(host).await?;
                let mut last_err = None;
                for ip in ips {
                    let auth = authority(&ip.to_string(), *port);
                    match self.inner.open_authority(&auth).await {
                        Ok(pipe) => return Ok(pipe),
                        Err(err) => last_err = Some(err),
                    }
                }
                Err(last_err.unwrap_or_else(|| {
                    failure(
                        Class::Unavailable,
                        format!("CONNECT {host}:{port} through the L4 door failed"),
                    )
                }))
            }
            Target::Name { host, port } => {
                let auth = authority(host, *port);
                self.inner.open_authority(&auth).await
            }
        }
    }
}

impl Dial for L4 {
    fn label(&self) -> String {
        format!("the L4 door of WARP {}", self.inner.door)
    }

    fn dial<'a>(
        &'a self,
        target: &'a Target,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(async move {
            let pipe = tokio::time::timeout_at(deadline, self.open(target))
                .await
                .map_err(|_| {
                    failure(
                        Class::Timeout,
                        format!(
                            "CONNECT {target} through the L4 door {}: no answer in time",
                            self.inner.door
                        ),
                    )
                })??;
            Ok(Connected::new(Box::new(pipe)))
        })
    }

    fn names_inside(&self) -> bool {
        true
    }
}

/// Copy bytes between the local duplex pipe and HTTP/3 DATA frames on the
/// CONNECT stream, preserving half-close.
async fn pump(
    mut send: noq::SendStream,
    mut recv: noq::RecvStream,
    rest: Vec<u8>,
    pipe: DuplexStream,
) {
    let (mut pipe_read, mut pipe_write) = tokio::io::split(pipe);
    let up = async {
        let mut buf = vec![0u8; UP_CHUNK];
        loop {
            match pipe_read.read(&mut buf).await {
                Ok(0) | Err(_) => {
                    let _ = send.finish();
                    break;
                }
                Ok(n) => {
                    if send.write_all(&frame(FRAME_DATA, &buf[..n])).await.is_err() {
                        break;
                    }
                }
            }
        }
    };
    let down = async {
        let mut data = DataFrames::default();
        let mut out = Vec::new();
        data.feed(&rest, &mut out);
        loop {
            if !out.is_empty() {
                if pipe_write.write_all(&out).await.is_err() {
                    let _ = recv.stop(0u32.into());
                    return;
                }
                out.clear();
            }
            match recv.read_chunk(DOWN_CHUNK).await {
                Ok(Some(chunk)) => data.feed(&chunk, &mut out),
                _ => break,
            }
        }
        let _ = pipe_write.shutdown().await;
    };
    tokio::join!(up, down);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_headers_match_official_wire_format() {
        let auth = bootstrap_authority();
        assert!(!auth.is_empty() && !auth.contains("://") && !auth.contains('/'));
        let raw = bootstrap_headers("cloudflareaccess.com", "a-6.35-5120");
        // Prefix: RIC=0, Base=0, :method CONNECT (static 15 = 0xcf)
        assert_eq!(&raw[..3], &[0x00, 0x00, 0xcf]);
        let text = String::from_utf8_lossy(&raw);
        assert!(text.contains(":protocol"), "{text:?}");
        assert!(text.contains("cf-connect-ip"), "{text:?}");
        assert!(text.contains("cloudflareaccess.com"), "{text:?}");
        assert!(text.contains("pq-enabled"), "{text:?}");
        assert!(text.contains("cf-client-version"), "{text:?}");
        assert!(text.contains("a-6.35-5120"), "{text:?}");
        // :scheme http (static 22 = 0xd6) and :path / (static 1 = 0xc1)
        assert!(raw.contains(&(0xc0 | 22)));
        assert!(raw.contains(&(0xc0 | 1)));
    }

    // Issue 140 (T-002): end-to-end loopback HTTP/3 stand-in verifying Stream 0
    // cf-connect-ip bootstrap, Stream 4 plain CONNECT with IP-literal :authority,
    // bidirectional DATA frame carriage, and clean half-close.
    #[tokio::test]
    async fn l4_bootstrap_and_connect_round_trip() {
        crate::test_host::need_loopback!();
        let provider = crate::warp::provider();
        let cert = super::super::masque::standin::generate_cert();
        let client_id = super::super::Identity::generate().unwrap();
        let srv_cfg = pod_quic::endpoint::server_config(cert.quic.clone()).unwrap();
        let endpoint = pod_quic::endpoint::server(srv_cfg, "127.0.0.1:0".parse().unwrap()).unwrap();
        let door_addr = endpoint.local_addr().unwrap();

        // QPACK static 25 = :status 200 (0x00, 0x00, 0xd9).
        let ok_headers = frame(h3::FRAME_HEADERS, &[0x00, 0x00, 0xc0 | 25]);
        let server_task = tokio::spawn(async move {
            let incoming = endpoint.accept().await.unwrap();
            let conn = incoming.await.unwrap();
            // Stream 0: cf-connect-ip bootstrap.
            let (mut s0_tx, mut s0_rx) = conn.accept_bi().await.unwrap();
            let mut frames0 = h3::Frames::default();
            let boot_payload = loop {
                if let Some((kind, payload)) = frames0.next_frame().unwrap() {
                    if kind == h3::FRAME_HEADERS {
                        break payload;
                    }
                }
                let chunk = s0_rx.read_chunk(4096).await.unwrap().unwrap();
                frames0.push(&chunk);
            };
            let boot_text = String::from_utf8_lossy(&boot_payload);
            assert!(boot_text.contains("cf-connect-ip"), "{boot_text:?}");
            assert!(boot_text.contains("cf-client-version"), "{boot_text:?}");
            s0_tx.write_all(&ok_headers).await.unwrap();

            // Stream 4: per-flow plain CONNECT with IP-literal :authority.
            let (mut s4_tx, mut s4_rx) = conn.accept_bi().await.unwrap();
            let mut frames4 = h3::Frames::default();
            let req_payload = loop {
                if let Some((kind, payload)) = frames4.next_frame().unwrap() {
                    if kind == h3::FRAME_HEADERS {
                        break payload;
                    }
                }
                let chunk = s4_rx.read_chunk(4096).await.unwrap().unwrap();
                frames4.push(&chunk);
            };
            let req_text = String::from_utf8_lossy(&req_payload);
            assert!(req_text.contains("198.51.100.1:443"), "{req_text:?}");
            s4_tx.write_all(&ok_headers).await.unwrap();

            // Echo DATA frames until client half-closes.
            let mut data = DataFrames::default();
            let mut out = Vec::new();
            data.feed(&frames4.into_rest(), &mut out);
            loop {
                if !out.is_empty() {
                    s4_tx.write_all(&frame(FRAME_DATA, &out)).await.unwrap();
                    out.clear();
                }
                match s4_rx.read_chunk(4096).await.unwrap() {
                    Some(chunk) => data.feed(&chunk, &mut out),
                    None => break,
                }
            }
            let _ = s4_tx.finish();
            // Keep stream 0 alive until client finishes reading.
            let _ = s0_rx.read_chunk(64).await;
            drop(s0_tx);
        });

        let tls = pod_warp::tls::client_config(provider, &client_id, cert.leaf_spki_der, h3::ALPN)
            .unwrap();
        let udp: Arc<dyn UdpWay> = Arc::new(pod_warp::SystemUdp);
        let l4 = L4::new(door_addr, udp, tls).with_sni("localhost");
        let mut pipe = l4
            .open(&Target::Addr("198.51.100.1:443".parse().unwrap()))
            .await
            .unwrap();
        pipe.write_all(b"ping-l4").await.unwrap();
        pipe.shutdown().await.unwrap();
        let mut got = Vec::new();
        pipe.read_to_end(&mut got).await.unwrap();
        assert_eq!(got, b"ping-l4");
        drop(l4);
        let _ = tokio::time::timeout(Duration::from_secs(2), server_task).await;
    }
}
