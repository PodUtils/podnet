//! The session end to end at the podnet surface.
//!
//! `pod-e2e` of PodUtils/podcrates does the work: the Noise XX handshake,
//! the pair binding, the destination envelope, the record layer with the
//! KEY-2 header, the hop table and the relay view. This module is the glue
//! of `podnet tunnel --e2e`.
//!
//! The handshake runs on the data stream before any application byte. The
//! sealed destination travels on a control stream, because the envelope's
//! prelude is out of band in `pod-e2e`: a relay carries it and cannot open
//! it. A relay that forwards both streams never holds a key of the channel.
//!
//! The record keys come from the handshake exporter (KEY-1), never from a
//! per-hop TLS exporter. The suite is agreed before the first record, and a
//! mismatch is a configuration error (KEY-3). The layer rekeys at 2^31
//! records or 1 GiB, whichever comes first (KEY-4).

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use pod_e2e::record::{self, Opened, Records, Settings, Suite};
use pod_e2e::{initiate, respond, Channel, Destination, Pair, Prelude, Sealed};
use pod_identity::{Identity, PublicKey};
use sha2::{Digest, Sha256};
use tokio::io::{split, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, DuplexStream};
use tokio::net::TcpStream;
use tokio::sync::Mutex;

use crate::exit::{self, Fault};

pub use pod_e2e::legs;

/// The record kind of an application byte.
pub const DATA: u8 = 0;
/// The record kind that ends one direction.
pub const END: u8 = 1;
/// The first byte of the control stream: a sealed destination.
pub const SEALED: u8 = 0;
/// The first byte of the control stream: the compatibility path, in clear.
pub const CLEAR: u8 = 1;
/// The most bytes of one record payload.
pub const CHUNK: usize = 16 * 1024;
/// The control stream carries one destination and no more.
pub const CONTROL_MAX: usize = 8 * 1024;
/// The name of the default suite.
pub const DEFAULT_SUITE: &str = "chacha20-poly1305";

/// The configuration of one end of an end-to-end session.
#[derive(Clone, Debug)]
pub struct Config {
    /// The relay's name, as the relay advertises it.
    pub relay: String,
    /// The pair's name, as the relay advertises it.
    pub pair: String,
    /// The suites that this end offers, the default first.
    pub suites: Vec<Suite>,
    /// The destination host. The far end reads it from the sealed envelope;
    /// the near end puts it there.
    pub host: String,
    /// The destination port.
    pub port: u16,
    /// The service name at the destination.
    pub service: String,
    /// The peers that this end admits. An empty list admits none.
    pub allowed: Vec<PublicKey>,
    /// The compatibility path: the destination in clear, with a warning.
    pub compat_destination: bool,
}

impl Config {
    /// The pair that the relay advertises.
    pub fn pair(&self) -> Pair {
        Pair::named(&self.relay, &self.pair)
    }

    /// The destination as `host:port`.
    pub fn destination(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    /// Whether `peer` is in the allow list.
    pub fn admits(&self, peer: &PublicKey) -> bool {
        self.allowed.iter().any(|key| key == peer)
    }
}

/// What one direction of a session carried.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stats {
    /// The application bytes that went out.
    pub out: u64,
    /// The application bytes that came in.
    pub in_bytes: u64,
    /// The records that this end sealed.
    pub records: u64,
    /// The epochs that this end sealed and opened with.
    pub epochs: (u64, u64),
    /// The suite of the records.
    pub suite: String,
}

/// The counters of a live session, shared by its two directions.
#[derive(Debug, Default)]
struct Counters {
    out: AtomicU64,
    in_bytes: AtomicU64,
    records: AtomicU64,
}

impl Counters {
    fn add_out(&self, bytes: usize) {
        self.out.fetch_add(bytes as u64, Ordering::Relaxed);
    }

    fn add_in(&self, bytes: usize) {
        self.in_bytes.fetch_add(bytes as u64, Ordering::Relaxed);
    }

    fn add_record(&self) {
        self.records.fetch_add(1, Ordering::Relaxed);
    }
}

/// The suite named by `text`, or a configuration error (exit 78).
pub fn suite_of(text: &str) -> Result<Suite> {
    Suite::ALL
        .into_iter()
        .find(|suite| suite.name() == text)
        .ok_or_else(|| {
            exit::fault(
                Fault::Config,
                anyhow!(
                    "no suite {:?}: use {}",
                    text,
                    Suite::ALL
                        .iter()
                        .map(|suite| suite.name())
                        .collect::<Vec<_>>()
                        .join(" or ")
                ),
            )
        })
}

/// The suite that both ends agree on. A mismatch is exit 78, and no record
/// is sent (KEY-3).
pub fn agree_suite(initiator: &[Suite], responder: &[Suite]) -> Result<Suite> {
    record::check_suites(initiator, responder)
        .map_err(|error| exit::fault(Fault::Config, anyhow!("the suite of the records: {error}")))
}

/// The destination as a host and a port: `host:port`, with an IPv6 host in
/// brackets.
pub fn parse_destination(text: &str) -> Result<(String, u16)> {
    let (host, port) = text.rsplit_once(':').ok_or_else(|| {
        exit::fault(
            Fault::Usage,
            anyhow!("--destination {text:?} is not HOST:PORT"),
        )
    })?;
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if host.is_empty() {
        return Err(exit::fault(
            Fault::Usage,
            anyhow!("--destination {text:?} has no host"),
        ));
    }
    let port: u16 = port
        .parse()
        .map_err(|_| exit::fault(Fault::Usage, anyhow!("--destination {text:?} has no port")))?;
    Ok((host.to_string(), port))
}

/// The near end: the handshake, the pair binding, the suite, the sealed
/// destination on `control`, then the records over `data` and `app`.
pub async fn client<D, C, A>(data: D, mut control: C, app: A, cfg: &Config) -> Result<Stats>
where
    D: AsyncRead + AsyncWrite + Unpin,
    C: AsyncWrite + Unpin,
    A: AsyncRead + AsyncWrite + Unpin,
{
    let me = crate::identity::create(&crate::identity::default_dir()?)
        .context("the identity of this end")?;
    client_with(data, &mut control, app, cfg, me.key()).await
}

/// [`client`] with an identity that the caller holds.
pub async fn client_with<D, C, A>(
    data: D,
    control: &mut C,
    app: A,
    cfg: &Config,
    me: &Identity,
) -> Result<Stats>
where
    D: AsyncRead + AsyncWrite + Unpin,
    C: AsyncWrite + Unpin,
    A: AsyncRead + AsyncWrite + Unpin,
{
    let offered = initiate(data, me).await?;
    if !cfg.admits(&offered.peer()) {
        return Err(exit::fault(
            Fault::NoPermission,
            anyhow!("the peer {} is not in the allow list", offered.peer()),
        ));
    }
    let mut channel = offered.accept(None).await?;
    channel.bind(cfg.pair())?;
    let suite = channel.agree(&cfg.suites).await.map_err(|error| {
        exit::fault(Fault::Config, anyhow!("the suite of the records: {error}"))
    })?;
    let sealed =
        Destination::seal(&channel, &cfg.host, cfg.port, &cfg.service).map_err(envelope_error)?;
    let records = channel
        .exporter()
        .ok_or_else(|| anyhow!("the handshake gave no exporter"))?
        .records(suite, Settings::DEFAULT, Instant::now())?;
    write_destination(control, cfg, &sealed).await?;
    let counters = Arc::new(Counters::default());
    let records = Arc::new(Mutex::new(records));
    carry(
        channel.into_transport(),
        app,
        records.clone(),
        counters.clone(),
    )
    .await?;
    let records = records.lock().await;
    Ok(Stats {
        out: counters.out.load(Ordering::Relaxed),
        in_bytes: counters.in_bytes.load(Ordering::Relaxed),
        records: records.stats().records,
        epochs: records.epochs(),
        suite: suite.name().to_string(),
    })
}

/// The far end: the handshake, the pair binding, the suite, the destination
/// from `control`, then the records over `data` and the destination.
pub async fn server<D, C>(data: D, mut control: C, cfg: &Config) -> Result<Stats>
where
    D: AsyncRead + AsyncWrite + Unpin,
    C: AsyncRead + Unpin,
{
    let me = crate::identity::create(&crate::identity::default_dir()?)
        .context("the identity of this end")?;
    server_with(data, &mut control, cfg, me.key()).await
}

/// [`server`] with an identity that the caller holds. `dial` opens the
/// destination: the tests use it to count the dials.
pub async fn server_with<D, C>(
    data: D,
    control: &mut C,
    cfg: &Config,
    me: &Identity,
) -> Result<Stats>
where
    D: AsyncRead + AsyncWrite + Unpin,
    C: AsyncRead + Unpin,
{
    let asked = respond(data, me).await?;
    if !cfg.admits(&asked.peer()) {
        return Err(exit::fault(
            Fault::NoPermission,
            anyhow!("the peer {} is not in the allow list", asked.peer()),
        ));
    }
    let mut channel = asked.admit().await?;
    channel.bind(cfg.pair())?;
    let suite = channel.agree(&cfg.suites).await.map_err(|error| {
        exit::fault(Fault::Config, anyhow!("the suite of the records: {error}"))
    })?;
    let (host, port, compat) = read_destination(control, &channel).await?;
    if compat && !cfg.compat_destination {
        return Err(exit::fault(
            Fault::Config,
            anyhow!("the relay sent the destination in clear, but --compat-destination is off"),
        ));
    }
    if compat {
        eprintln!(
            "warning: /connect/{host}/{port} is the compatibility path: the relay learns the destination"
        );
    }
    // EGRESS-3 and EGRESS-4: the destination comes from the relay's control
    // stream, so the deployment's policy decides before the socket opens.
    crate::policy::permit_fault(host.as_str(), port)?;
    let mut target = TcpStream::connect((host.as_str(), port))
        .await
        .with_context(|| format!("the destination {host}:{port}"))?;
    let records = channel
        .exporter()
        .ok_or_else(|| anyhow!("the handshake gave no exporter"))?
        .records(suite, Settings::DEFAULT, Instant::now())?;
    let counters = Arc::new(Counters::default());
    let records = Arc::new(Mutex::new(records));
    carry_records(
        channel.into_transport(),
        &mut target,
        records.clone(),
        counters.clone(),
    )
    .await?;
    let records = records.lock().await;
    Ok(Stats {
        out: counters.out.load(Ordering::Relaxed),
        in_bytes: counters.in_bytes.load(Ordering::Relaxed),
        records: records.stats().records,
        epochs: records.epochs(),
        suite: suite.name().to_string(),
    })
}

/// The warning of the compatibility path (OPAQUE-2): the destination goes in
/// clear, so every hop of the control stream learns it.
pub fn compat_warning(host: &str, port: u16) -> String {
    format!(
        "warning: --compat-destination sends /connect/{host}/{port} in clear: every hop learns the destination"
    )
}

/// The control stream of the near end: one byte of kind, then the envelope
/// or the clear destination.
async fn write_destination<C: AsyncWrite + Unpin>(
    control: &mut C,
    cfg: &Config,
    sealed: &Sealed,
) -> Result<()> {
    let mut out = Vec::with_capacity(1 + pod_e2e::envelope::SIZE);
    if cfg.compat_destination {
        eprintln!("{}", compat_warning(&cfg.host, cfg.port));
        out.push(CLEAR);
        out.extend_from_slice(format!("/connect/{}/{}", cfg.host, cfg.port).as_bytes());
    } else {
        out.push(SEALED);
        out.extend_from_slice(sealed.bytes());
    }
    let len = u32::try_from(out.len()).expect("a control message below 4 GiB");
    control.write_all(&len.to_be_bytes()).await?;
    control.write_all(&out).await?;
    control.flush().await?;
    Ok(())
}

/// The control stream of the far end: the sealed destination, or the clear
/// compatibility path.
async fn read_destination<C: AsyncRead + Unpin, T>(
    control: &mut C,
    channel: &Channel<T>,
) -> Result<(String, u16, bool)> {
    let mut size = [0u8; 4];
    tokio::time::timeout(Duration::from_secs(30), control.read_exact(&mut size))
        .await
        .map_err(|_| anyhow!("the control stream gave no destination within 30 s"))??;
    let size = u32::from_be_bytes(size) as usize;
    if size == 0 || size > CONTROL_MAX {
        return Err(exit::fault(
            Fault::DataError,
            anyhow!("a control message of {size} bytes"),
        ));
    }
    let mut body = vec![0u8; size];
    control.read_exact(&mut body).await?;
    match body[0] {
        SEALED => {
            let prelude = Prelude::of(&body[1..]).map_err(envelope_error)?;
            let destination =
                Destination::open(channel, prelude.bytes()).map_err(envelope_error)?;
            Ok((destination.host().to_string(), destination.port(), false))
        }
        CLEAR => {
            let text = String::from_utf8(body[1..].to_vec()).map_err(|_| {
                exit::fault(
                    Fault::DataError,
                    anyhow!("the clear destination is not text"),
                )
            })?;
            let path = text.strip_prefix("/connect/").ok_or_else(|| {
                exit::fault(
                    Fault::DataError,
                    anyhow!("the clear destination is not /connect/HOST/PORT: {text:?}"),
                )
            })?;
            let (host, port) = path.rsplit_once('/').ok_or_else(|| {
                exit::fault(
                    Fault::DataError,
                    anyhow!("the clear destination is not /connect/HOST/PORT: {text:?}"),
                )
            })?;
            let host = host.trim_start_matches('[').trim_end_matches(']');
            let port: u16 = port.parse().map_err(|_| {
                exit::fault(
                    Fault::DataError,
                    anyhow!("the clear destination has no port: {text:?}"),
                )
            })?;
            Ok((host.to_string(), port, true))
        }
        other => Err(exit::fault(
            Fault::DataError,
            anyhow!("a control message of kind {other}"),
        )),
    }
}

fn envelope_error(error: pod_e2e::EnvelopeError) -> anyhow::Error {
    exit::fault(
        Fault::DataError,
        anyhow!("the destination envelope: {error}"),
    )
}

/// The records over `transport`, with the application on both sides.
async fn carry<T, A>(
    transport: T,
    app: A,
    records: Arc<Mutex<Records>>,
    counters: Arc<Counters>,
) -> Result<()>
where
    T: AsyncRead + AsyncWrite + Unpin,
    A: AsyncRead + AsyncWrite + Unpin,
{
    let (mut transport_read, mut transport_write) = split(transport);
    let (mut app_read, mut app_write) = split(app);
    let up = async {
        let mut buf = vec![0u8; CHUNK];
        loop {
            let n = app_read.read(&mut buf).await?;
            let end = n == 0;
            let payload = &buf[..n];
            let record = {
                let mut records = records.lock().await;
                records
                    .seal(Instant::now(), if end { END } else { DATA }, 0, 0, payload)
                    .map_err(|error| anyhow!("the record of this end: {error}"))?
            };
            counters.add_record();
            transport_write.write_all(&record).await?;
            if end {
                transport_write.flush().await?;
                break;
            }
            counters.add_out(n);
        }
        Ok::<(), anyhow::Error>(())
    };
    let down = async {
        loop {
            let Some(bytes) = read_record(&mut transport_read).await? else {
                break;
            };
            let opened = {
                let mut records = records.lock().await;
                records
                    .open(Instant::now(), &bytes)
                    .map_err(|error| anyhow!("the record of the peer: {error}"))?
            };
            match opened {
                Opened::Resume { .. } => continue,
                Opened::Record(record) if record.kind == END => {
                    app_write.shutdown().await?;
                    break;
                }
                Opened::Record(record) if record.kind == DATA => {
                    app_write.write_all(&record.payload).await?;
                    counters.add_in(record.payload.len());
                }
                Opened::Record(record) => {
                    return Err(anyhow!("a record of kind {}", record.kind));
                }
            }
        }
        Ok::<(), anyhow::Error>(())
    };
    tokio::try_join!(up, down)?;
    app_write.flush().await?;
    Ok(())
}

/// [`carry`] with the application already split, for the far end.
async fn carry_records<T, A>(
    transport: T,
    app: &mut A,
    records: Arc<Mutex<Records>>,
    counters: Arc<Counters>,
) -> Result<()>
where
    T: AsyncRead + AsyncWrite + Unpin,
    A: AsyncRead + AsyncWrite + Unpin,
{
    let (mut transport_read, mut transport_write) = split(transport);
    let (mut app_read, mut app_write) = split(app);
    let up = async {
        let mut buf = vec![0u8; CHUNK];
        loop {
            let n = app_read.read(&mut buf).await?;
            let end = n == 0;
            let record = {
                let mut records = records.lock().await;
                records
                    .seal(
                        Instant::now(),
                        if end { END } else { DATA },
                        0,
                        0,
                        &buf[..n],
                    )
                    .map_err(|error| anyhow!("the record of this end: {error}"))?
            };
            counters.add_record();
            transport_write.write_all(&record).await?;
            if end {
                transport_write.flush().await?;
                break;
            }
            counters.add_out(n);
        }
        Ok::<(), anyhow::Error>(())
    };
    let down = async {
        loop {
            let Some(bytes) = read_record(&mut transport_read).await? else {
                break;
            };
            let opened = {
                let mut records = records.lock().await;
                records
                    .open(Instant::now(), &bytes)
                    .map_err(|error| anyhow!("the record of the peer: {error}"))?
            };
            match opened {
                Opened::Resume { .. } => continue,
                Opened::Record(record) if record.kind == END => {
                    app_write.shutdown().await?;
                    break;
                }
                Opened::Record(record) if record.kind == DATA => {
                    app_write.write_all(&record.payload).await?;
                    counters.add_in(record.payload.len());
                }
                Opened::Record(record) => {
                    return Err(anyhow!("a record of kind {}", record.kind));
                }
            }
        }
        Ok::<(), anyhow::Error>(())
    };
    tokio::try_join!(up, down)?;
    app_write.flush().await?;
    Ok(())
}

/// One whole record from `read`, or none at the clean end of the stream.
async fn read_record<R: AsyncRead + Unpin>(read: &mut R) -> Result<Option<Vec<u8>>> {
    let mut header = [0u8; record::HEADER];
    match read.read_exact(&mut header).await {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error.into()),
    }
    let whole =
        record::record_len(&header).map_err(|error| anyhow!("the header of a record: {error}"))?;
    let mut rest = vec![0u8; whole - record::HEADER];
    read.read_exact(&mut rest).await?;
    let mut whole = header.to_vec();
    whole.extend_from_slice(&rest);
    Ok(Some(whole))
}

/// The token of a session at one relay and one service (LEARN-2): 64 hex
/// digits, from `secret`, `relay` and `service`.
pub fn mint_token(secret: &[u8; 32], relay: &str, service: &str) -> String {
    let seed = token_seed(secret, relay, service);
    pod_e2e::legs::token(&seed, 1).to_string()
}

/// Whether `token` is the token of `relay` and `service` under `secret`.
pub fn verify_token(secret: &[u8; 32], relay: &str, service: &str, token: &str) -> bool {
    let seed = token_seed(secret, relay, service);
    let expected = pod_e2e::legs::token(&seed, 1);
    constant_time_eq(token.as_bytes(), expected.as_bytes())
}

/// Compare two byte strings without a branch on their contents.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut difference = 0u8;
    for (left, right) in a.iter().zip(b) {
        difference |= left ^ right;
    }
    difference == 0
}

fn token_seed(secret: &[u8; 32], relay: &str, service: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"podnet e2e token, version 1");
    hasher.update(secret);
    hasher.update(relay.as_bytes());
    hasher.update([0]);
    hasher.update(service.as_bytes());
    hasher.finalize().into()
}

/// The exchange as a hop sees it, for `tunnel --e2e-inspect` (LEARN-3): the
/// hop table, and the relay's view of a prelude and the first record. The
/// call runs a handshake in memory and opens no socket.
pub async fn inspect(cfg: &Config, peer: &PublicKey) -> Result<String> {
    let (a, b) = tokio::io::duplex(64 * 1024);
    let client_id = Identity::from_seed(&[1u8; 32]);
    let server_id = Identity::from_seed(&[2u8; 32]);
    let pair = cfg.pair();
    let suites = cfg.suites.clone();
    let host = cfg.host.clone();
    let port = cfg.port;
    let service = cfg.service.clone();
    let near = async {
        let offered = initiate(a, &client_id).await?;
        let mut channel = offered.accept(None).await?;
        channel.bind(pair)?;
        let suite = channel.agree(&suites).await?;
        let sealed = Destination::seal(&channel, &host, port, &service)?;
        let mut records = channel
            .exporter()
            .ok_or_else(|| anyhow!("no exporter"))?
            .records(suite, Settings::DEFAULT, Instant::now())?;
        let wire = records.seal(Instant::now(), DATA, 0, 0, b"inspect")?;
        Ok::<_, anyhow::Error>((sealed, wire))
    };
    let far = async {
        let asked = respond(b, &server_id).await?;
        let mut channel = asked.admit().await?;
        channel.bind(pair)?;
        let _suite = channel.agree(&suites).await?;
        Ok::<_, anyhow::Error>(())
    };
    let (near, far) = tokio::join!(near, far);
    let (sealed, wire) = near?;
    far?;
    let prelude = sealed.prelude();
    let table = pod_e2e::Knowledge::table(
        "127.0.0.1:0",
        &format!("{}/{}", cfg.relay, cfg.pair),
        &peer.to_string(),
        &cfg.destination(),
        &cfg.service,
    );
    let mut out = pod_e2e::Knowledge::render(&table);
    out.push_str(&pod_e2e::Inspect::relay(&prelude, &wire).render());
    Ok(out)
}

/// A pair of records from two ends of a handshake that the caller runs:
/// the near end's and the far end's. The tests use it for the 1 GiB rekey.
pub async fn records_with(
    near: &Identity,
    far: &Identity,
    pair: Pair,
    suite: Suite,
) -> Result<(Records, Records)> {
    records_with_settings(near, far, pair, suite, Settings::DEFAULT).await
}

/// As [`records_with`], with the rekey settings of the caller. The test of
/// the record ceiling uses a small `rekey_records` so that the ceiling is
/// reached without 1 GiB.
pub async fn records_with_settings(
    near: &Identity,
    far: &Identity,
    pair: Pair,
    suite: Suite,
    settings: Settings,
) -> Result<(Records, Records)> {
    let (a, b) = tokio::io::duplex(64 * 1024);
    let near_task = async {
        let offered = initiate(a, near).await?;
        let mut channel = offered.accept(None).await?;
        channel.bind(pair)?;
        let _ = channel.agree(&[suite]).await?;
        let records = channel
            .exporter()
            .ok_or_else(|| anyhow!("no exporter"))?
            .records(suite, settings, Instant::now())?;
        Ok::<_, anyhow::Error>(records)
    };
    let far_task = async {
        let asked = respond(b, far).await?;
        let mut channel = asked.admit().await?;
        channel.bind(pair)?;
        let _ = channel.agree(&[suite]).await?;
        let records = channel
            .exporter()
            .ok_or_else(|| anyhow!("no exporter"))?
            .records(suite, settings, Instant::now())?;
        Ok::<_, anyhow::Error>(records)
    };
    let (near_records, far_records) = tokio::join!(near_task, far_task);
    Ok((near_records?, far_records?))
}

/// A TCP address for a listener, as `host:port`.
pub fn socket_address(text: &str) -> Result<SocketAddr> {
    text.parse().map_err(|_| {
        exit::fault(
            Fault::Usage,
            anyhow!("{text:?} is not an address: use IP:PORT"),
        )
    })
}

/// A member of the allow list from its 64 hex digits, or its display form.
pub fn public_key(text: &str) -> Result<PublicKey> {
    PublicKey::parse(text).ok_or_else(|| {
        exit::fault(
            Fault::Usage,
            anyhow!("--peer {text:?} is not an Ed25519 key"),
        )
    })
}

/// The two halves of a duplex stream, for a test or a pipe.
pub fn duplex() -> (DuplexStream, DuplexStream) {
    tokio::io::duplex(CHUNK)
}
