//! Cloudflare WARP, spoken directly: anonymous registration, MASQUE CONNECT-IP
//! over HTTP/2 on TCP 443, and a userspace IP stack on top of it.
//!
//! Two things that `cloudflared` cannot do from a locked-down host:
//!
//! * **A tunnel whose only egress is TCP 443.** The registration API and the
//!   MASQUE front door are both reachable on 443, so a host that permits
//!   nothing else still gets a working WARP tunnel.
//! * **A destination the host's firewall refuses.** Packets are built by
//!   the userspace stack and carried inside the tunnel, so the host never
//!   issues a `connect()` to the real address. That is how a Cloudflare
//!   tunnel edge on port 7844 becomes reachable on a host where 7844 is
//!   filtered.
//!
//! The client is `pod-warp` of PodUtils/podcrates. This module is the glue:
//! podnet's flags, variables and state files into its settings, podnet's
//! egress and relay as its dials, and its events into podnet's log.
//!
//! ```no_run
//! # async fn demo() -> anyhow::Result<()> {
//! use podnet::warp::WarpClient;
//!
//! // Register anonymously and enrol a MASQUE identity in one call.
//! let client = WarpClient::register().await?;
//! // Open the tunnel; only TCP 443 is needed.
//! let stack = client.connect().await?;
//! // Reach anything, including ports the host blocks.
//! let conn = podnet::warp::tcp_connect(&stack, "198.41.192.27:7844".parse()?).await?;
//! # let _ = conn;
//! # Ok(())
//! # }
//! ```

pub mod capsule;
pub mod exit;
pub mod export;
#[cfg(unix)]
pub mod find;
pub mod foreign;
pub mod forward;
pub mod hints;
pub mod identity;
pub mod ip;
pub mod l4;
pub mod live;
pub mod nc;
pub mod pod;
pub mod proxy;
pub mod quic;
pub mod register;
pub mod route;
pub mod rules;
pub mod session;
pub mod ssh_config;
#[cfg(target_os = "linux")]
pub mod tun;
pub mod up;
pub mod wg;

pub use identity::Identity;
pub use ip::{check_cc, check_tcp_burst, WarpStack};
pub use live::{Closable, Live};
pub use pod_warp::h3;
pub use register::Device;
pub use session::{MasqueRx, MasqueSession, MasqueTx};

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use pod_core::{BoxFuture, Connected, Dial, Failure, Target};

use crate::egress::Proxy;

/// A failure of `pod-warp` as an error of podnet: the class of the failure
/// gives the exit code (`crate::exit`).
pub fn failed(failure: Failure) -> anyhow::Error {
    use crate::exit::Fault;
    use pod_core::Class;
    let fault = match failure.class {
        Class::Usage => Fault::Usage,
        Class::Config | Class::Revoked => Fault::Config,
        Class::NotFound => Fault::NoInput,
        Class::TempFail => Fault::TempFail,
        _ => Fault::Unavailable,
    };
    crate::exit::fault(fault, anyhow::Error::new(failure))
}

/// `?` for a result of `pod-warp`, with the class of [`failed`].
pub trait Warped<T> {
    fn warped(self) -> Result<T>;
}

impl<T> Warped<T> for std::result::Result<T, Failure> {
    fn warped(self) -> Result<T> {
        self.map_err(failed)
    }
}

/// The events of `pod-warp` into podnet's log, and its decisions into
/// podnet's counters with podnet's names.
pub fn report() -> pod_core::Report {
    pod_core::Report {
        events: Arc::new(|event: pod_core::Event| match event.level {
            pod_core::Level::Info => tracing::info!(kind = event.kind, "{}", event.text),
            pod_core::Level::Warn => tracing::warn!(kind = event.kind, "{}", event.text),
        }),
        observer: Arc::new(Decisions),
    }
}

/// podnet's counters of the decisions of `pod-warp`.
struct Decisions;

impl pod_core::Observer for Decisions {
    fn decision(&self, decision: &'static str, outcome: &str) {
        match decision {
            "warp.leg" => crate::decisions::count("masque", &outcome.replace('-', "_")),
            "warp.carry" => crate::decisions::count("masque", "carry"),
            "warp.dispatch" => crate::decisions::count("dispatch", outcome),
            "warp.rule" => crate::decisions::count("rule", outcome),
            other => crate::decisions::count(other, outcome),
        }
    }
}

/// The TLS provider of podnet.
pub(crate) fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    crate::trust::context().provider().clone()
}

/// The TLS client of podnet for HTTPS and DNS over TLS inside the tunnel.
pub(crate) fn tls_client() -> Result<Arc<rustls::ClientConfig>> {
    Ok(Arc::new(
        crate::trust::context().client_config_with(&[], &[b"http/1.1"])?,
    ))
}

/// A TCP stream to `addr` through `net`.
pub async fn tcp_connect(net: &dyn Dial, addr: SocketAddr) -> Result<pod_core::BoxedIo> {
    let deadline = tokio::time::Instant::now() + pod_warp::session::CONNECT_TIMEOUT;
    let connected = net.dial(&Target::Addr(addr), deadline).await.warped()?;
    Ok(connected.io)
}

/// The dial of a MASQUE session: podnet's egress, with the split of the
/// first ClientHello of `--fragment-client-hello` (issue 135).
struct MasqueDial(Arc<dyn Dial>);

impl Dial for MasqueDial {
    fn label(&self) -> String {
        self.0.label()
    }

    fn dial<'a>(
        &'a self,
        target: &'a Target,
        deadline: tokio::time::Instant,
    ) -> BoxFuture<'a, std::result::Result<Connected, Failure>> {
        Box::pin(async move {
            let mut connected = self.0.dial(target, deadline).await?;
            connected.io = pod_tls::fragment::wrap(connected.io, live::split());
            Ok(connected)
        })
    }

    fn names_inside(&self) -> bool {
        self.0.names_inside()
    }
}

/// The WebSocket relay of podnet as the relay leg of a session (issue 76).
struct RelayDial(Arc<crate::ws_relay::Relay>);

impl Dial for RelayDial {
    fn label(&self) -> String {
        format!("the relay {}", self.0.url())
    }

    fn dial<'a>(
        &'a self,
        target: &'a Target,
        deadline: tokio::time::Instant,
    ) -> BoxFuture<'a, std::result::Result<Connected, Failure>> {
        Box::pin(async move {
            let Target::Addr(addr) = target else {
                let text = format!("the relay takes an address, not {target}");
                return Err(Failure::new(
                    pod_core::Stage::Connect,
                    pod_core::Class::Usage,
                    text,
                ));
            };
            let limit = deadline.saturating_duration_since(tokio::time::Instant::now());
            let io = self.0.connect(*addr, limit).await.map_err(|e| {
                let text = format!("{target} through the relay {}: {e:#}", self.0.url());
                Failure::caused(
                    pod_core::Stage::Connect,
                    pod_core::Class::Unavailable,
                    text,
                    e,
                )
            })?;
            let io = pod_tls::fragment::wrap(Box::new(io), live::split());
            Ok(Connected::new(io))
        })
    }
}

/// The relay of the fallback: the relay of `--transport relay` when it is
/// set, else the relay of `PODNET_RELAY` or the default relay. One for the
/// process.
fn masque_relay() -> Option<Arc<crate::ws_relay::Relay>> {
    static RELAY: std::sync::OnceLock<Option<Arc<crate::ws_relay::Relay>>> =
        std::sync::OnceLock::new();
    RELAY
        .get_or_init(|| match crate::ws_relay::Relay::from_env(None) {
            Ok(relay) => Some(Arc::new(relay)),
            Err(err) => {
                tracing::warn!(error = %format!("{err:#}"), "no WebSocket relay for MASQUE");
                None
            }
        })
        .clone()
}

/// The relay leg of `pod-warp` over `relay`, with the budget of a relay
/// session (the owner ruling of 2026-11-04).
fn relay_leg(relay: &Arc<crate::ws_relay::Relay>) -> pod_warp::Relay {
    let mut leg = pod_warp::Relay::new(Arc::new(RelayDial(relay.clone())), relay.url());
    leg.meter = Some(Arc::new(|io| {
        let (io, budget) = crate::ws_relay::meter(Box::new(io));
        let renew: pod_warp::net::Renew = Arc::new(move || budget.spent());
        (Box::new(io) as pod_core::BoxedIo, renew)
    }));
    leg
}

/// The API of WARP through `proxy`, or the egress of the process. A bad
/// [`register::API_VARIABLE`] is a configuration error (exit 78).
pub(crate) fn api(proxy: Option<&Proxy>) -> Result<pod_warp::Api> {
    let bases = register::api_bases(std::env::var(register::API_VARIABLE).ok().as_deref())
        .map_err(|e| crate::exit::fault(crate::exit::Fault::Config, e))?;
    Ok(api_client(proxy).with_bases(bases))
}

/// The API of WARP through `proxy` at the bases of the variable, or at the
/// default bases when the variable is bad: a constructor that reads the
/// variable reports it first ([`api`]).
fn api_or_default(proxy: Option<&Proxy>) -> pod_warp::Api {
    api(proxy).unwrap_or_else(|_| api_client(proxy))
}

/// The API of WARP at the default bases, through `proxy`.
fn api_client(proxy: Option<&Proxy>) -> pod_warp::Api {
    let client = pod_proxy::Client::new(
        crate::trust::context().clone(),
        crate::egress::egress(proxy.cloned()),
    )
    .with_user_agent(format!("podnet/{}", env!("CARGO_PKG_VERSION")));
    pod_warp::Api::new(client)
}

/// The MTU of the stack of a carried device (issues 154 and 155).
pub const CARRY_MTU: usize = pod_warp::CARRY_MTU;

/// The door file of an identity file: `warp.json` gives `warp-door.json`.
fn door_file_of(identity: &Path) -> PathBuf {
    let stem = identity
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "warp".into());
    identity.with_file_name(format!("{stem}-door.json"))
}

/// The leg file of an identity file: `warp.json` gives `warp-leg.json`.
fn leg_file_of(identity: &Path) -> PathBuf {
    let stem = identity
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "warp".into());
    identity.with_file_name(format!("{stem}-leg.json"))
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// How podnet reaches the MASQUE front door (issue 76).
#[derive(Clone, Debug, Default)]
pub struct MasqueOptions {
    /// No WebSocket relay when the direct path fails.
    pub no_relay: bool,
    /// The front door in place of the one of the device record.
    pub endpoint: Option<SocketAddr>,
    /// The TLS name in place of `consumer-masque.cloudflareclient.com`.
    pub sni: Option<String>,
    /// HTTP/3 on UDP 443 in place of HTTP/2 on TCP 443 (issue 157). No
    /// relay and no proxy: they carry TCP only.
    pub h3: bool,
}

/// A registered WARP device with a MASQUE identity ready to dial: a
/// `pod_warp::Warp` with podnet's settings and state files.
pub struct WarpClient {
    warp: pod_warp::Warp,
    proxy: Option<Proxy>,
    masque: MasqueOptions,
    wireguard: Option<wg::Peer>,
    carry: Option<wg::Peer>,
    /// The file that keeps the leg that worked between two runs (issue
    /// 126), next to the identity file.
    leg_file: Option<PathBuf>,
    /// The identity file belongs to another user: podnet writes no door
    /// file and no leg file next to it.
    foreign: bool,
}

impl WarpClient {
    /// Register anonymously and enrol the MASQUE identity.
    pub async fn register() -> Result<Self> {
        Self::register_with(None).await
    }

    /// Register through a specific proxy, or directly when `proxy` is `None`.
    pub async fn register_with(proxy: Option<Proxy>) -> Result<Self> {
        api(proxy.as_ref())?;
        let settings = Self::settings_of(proxy.as_ref(), &MasqueOptions::default(), None, None);
        let warp = pod_warp::Warp::register(settings)
            .await
            .warped()
            .context("registering a WARP device")?;
        Ok(Self::of(warp, proxy, None))
    }

    /// Build a client from an already-enrolled device and identity, for callers
    /// that persist them.
    pub fn from_parts(device: Device, identity: Identity) -> Self {
        let settings = Self::settings_of(None, &MasqueOptions::default(), None, None);
        Self::of(
            pod_warp::Warp::from_parts(settings, device, identity),
            None,
            None,
        )
    }

    fn of(warp: pod_warp::Warp, proxy: Option<Proxy>, leg_file: Option<PathBuf>) -> Self {
        Self {
            warp,
            proxy,
            masque: MasqueOptions::default(),
            wireguard: None,
            carry: None,
            leg_file,
            foreign: false,
        }
    }

    /// The settings of `pod-warp` for podnet's options.
    fn settings_of(
        proxy: Option<&Proxy>,
        masque: &MasqueOptions,
        wireguard: Option<&wg::Peer>,
        carry: Option<&wg::Peer>,
    ) -> pod_warp::Settings {
        let dial: Arc<dyn Dial> = Arc::new(MasqueDial(crate::egress::egress(proxy.cloned())));
        let mut way = pod_warp::Way::new(dial, provider());
        way.sni = masque.sni.clone();
        way.h3 = masque.h3;
        if masque.h3 || wireguard.is_some() {
            way.udp = Some(Arc::new(pod_warp::SystemUdp));
        }
        let mut settings = pod_warp::Settings::new(api_or_default(proxy), way);
        // Issue 157: HTTP/3 has no relay (the relay carries TCP).
        if !masque.no_relay && !masque.h3 && wireguard.is_none() {
            settings.relay = masque_relay().as_ref().map(relay_leg);
        }
        settings.wireguard = wireguard.cloned();
        settings.carry = carry.cloned();
        if let Some(door) = masque.endpoint {
            settings.door = pod_warp::Door::Pinned(Target::Addr(door));
        }
        // A proxy reaches both families for podnet.
        settings.routes = pod_warp::Routes {
            v4: proxy.is_some() || crate::resolve::has_default_route(false),
            v6: proxy.is_some() || crate::resolve::has_default_route(true),
        };
        settings.hints = hints::current();
        settings.report = report();
        settings.net = ip::net_settings();
        settings
    }

    /// The same device with the options of now; the door that worked and
    /// the leg of the files go with it.
    fn rebuild(self) -> Self {
        let settings = Self::settings_of(
            self.proxy.as_ref(),
            &self.masque,
            self.wireguard.as_ref(),
            self.carry.as_ref(),
        );
        let won = self.warp.won_door();
        let relay_first = match (&settings.relay, &self.leg_file) {
            (Some(relay), Some(path)) => std::fs::read_to_string(path)
                .map(|text| pod_warp::leg::relay_first_from(&text, unix_now(), &relay.name))
                .unwrap_or(false),
            _ => false,
        };
        if relay_first {
            tracing::debug!("the last run used the relay; it goes first");
        }
        let device = self.warp.device().clone();
        let identity = self.warp.identity().clone();
        let warp = pod_warp::Warp::from_parts(settings, device, identity)
            .with_won_door(won)
            .with_relay_first(relay_first);
        Self { warp, ..self }
    }

    /// How to reach the MASQUE front door: the relay fallback and the
    /// overrides of the endpoint and the TLS name.
    pub fn with_masque(mut self, masque: MasqueOptions) -> Self {
        self.masque = masque;
        self.rebuild()
    }

    /// Carry the tunnel on WireGuard with `peer` (issue 152), in place of
    /// MASQUE.
    pub fn with_wireguard(mut self, peer: Option<wg::Peer>) -> Self {
        self.wireguard = peer;
        self.rebuild()
    }

    /// Carry a second WireGuard device inside the session (issues 154 and
    /// 155).
    pub fn with_carry(mut self, peer: Option<wg::Peer>) -> Self {
        self.carry = peer;
        self.rebuild()
    }

    /// Use this proxy for the registration calls and the MASQUE dial, or none
    /// for a direct connection.
    pub fn with_proxy(mut self, proxy: Option<Proxy>) -> Self {
        self.proxy = proxy;
        self.rebuild()
    }

    /// The L4 door of `warp proxy --l4` (issue 140): the front door of the
    /// device (or `--masque-endpoint`), the TLS name `sni`, the client
    /// certificate and the pinned key.
    pub fn l4(&self, sni: &str) -> Result<pod_warp::L4> {
        let mut masque = self.masque.clone();
        masque.h3 = true;
        let settings = Self::settings_of(self.proxy.as_ref(), &masque, None, None);
        let warp = pod_warp::Warp::from_parts(
            settings,
            self.warp.device().clone(),
            self.warp.identity().clone(),
        );
        let mut door = warp.l4().warped()?.with_sni(sni);
        if let Some(value) = std::env::var(l4::FORM_VARIABLE)
            .ok()
            .filter(|v| !v.is_empty())
        {
            door = door.with_form(pod_warp::l4::Form::parse(&value).warped()?);
        }
        Ok(door)
    }

    /// True when the tunnel goes on WireGuard.
    pub fn is_wireguard(&self) -> bool {
        self.wireguard.is_some()
    }

    /// The proxy in use, if any.
    pub fn proxy(&self) -> Option<&Proxy> {
        self.proxy.as_ref()
    }

    /// Write the device and the identity to one JSON file (mode 0600), so a
    /// later run can resume the tunnel without registering a second device.
    pub fn save(&self, path: &Path) -> Result<()> {
        self.warp.save(path).warped()
    }

    /// Read a client written by [`WarpClient::save`]. A record with the
    /// state `created` (a device with no MASQUE key) is refused.
    pub fn load(path: &Path) -> Result<Self> {
        let stored = pod_warp::store::load(&foreign::identity_file(path))
            .warped()
            .map_err(|e| foreign::hint(e, path))?;
        api(None)?;
        let settings = Self::settings_of(None, &MasqueOptions::default(), None, None);
        let mut client = Self::of(
            pod_warp::Warp::from_stored(settings, stored),
            None,
            Some(path.to_path_buf()),
        );
        client.foreign = foreign::owner(path).is_some();
        client.with_files(path)
    }

    /// The door of the door file (issue 133) and the leg of the leg file
    /// (issue 126) of the identity file `path`.
    fn with_files(mut self, path: &Path) -> Result<Self> {
        self.leg_file = Some(leg_file_of(path));
        if let Ok(text) = std::fs::read_to_string(door_file_of(path)) {
            let doors = self.warp.device().masque_endpoints();
            let door = pod_warp::door::door_from(&text, unix_now(), &doors);
            self.warp = self.warp.with_won_door(door);
        }
        Ok(self.rebuild())
    }

    /// Register if `path` holds nothing yet, otherwise resume what it holds.
    ///
    /// The call holds the lock `PATH.lock` and saves the device after the
    /// first remote call (T-241). A file that cannot be read, or that other
    /// users can read, is an error: podnet does not write over it.
    pub async fn register_or_load(path: &Path) -> Result<Self> {
        api(None)?;
        let settings = Self::settings_of(None, &MasqueOptions::default(), None, None);
        let warp = pod_warp::Warp::register_or_load(settings, &foreign::identity_file(path))
            .await
            .warped()
            .map_err(|e| foreign::hint(e, path))?;
        foreign::warn(path);
        let mut client = Self::of(warp, None, None);
        client.foreign = foreign::owner(path).is_some();
        client.with_files(path)
    }

    /// The default place for the saved identity: `warp.json` in the state
    /// directory of [`crate::state`], or `None` with `PODNET_STATE_DIR=none`.
    pub fn default_identity_path() -> Result<Option<PathBuf>> {
        Ok(crate::state::state_dir()?.map(|dir| dir.join("warp.json")))
    }

    /// Read the device record from the API again. Keep the token, which only
    /// the registration answer has.
    pub async fn refresh(&mut self) -> Result<()> {
        self.warp.refresh().await.warped()
    }

    /// The account record: type, WARP+ state and quota. The license key is
    /// hidden.
    pub async fn account(&self) -> Result<serde_json::Value> {
        self.warp.account().await.warped()
    }

    /// Attach a WARP+ license key, then read the device again.
    pub async fn set_license(&mut self, license: &str) -> Result<serde_json::Value> {
        self.warp.set_license(license).await.warped()
    }

    /// Make a new MASQUE key and enrol it. The old key stops working.
    pub async fn rotate_key(&mut self) -> Result<()> {
        self.warp.rotate_key().await.warped()
    }

    /// Remove the device from the WARP service.
    pub async fn delete(&self) -> Result<()> {
        self.warp.delete().await.warped()
    }

    /// The device record, including the assigned addresses and the pinned peer
    /// key.
    pub fn device(&self) -> &Device {
        self.warp.device()
    }

    /// The identity presented to the front door.
    pub fn identity(&self) -> &Identity {
        self.warp.identity()
    }

    /// The address the tunnel assigned to us.
    pub fn local_address(&self) -> Result<std::net::IpAddr> {
        if let Some(peer) = &self.wireguard {
            return peer
                .v4
                .map(std::net::IpAddr::V4)
                .or(peer.v6.map(std::net::IpAddr::V6))
                .context("the WireGuard device has no address");
        }
        let device = self.device();
        device
            .ipv4()
            .map(std::net::IpAddr::V4)
            .or_else(|| device.ipv6().map(std::net::IpAddr::V6))
            .context("the tunnel assigned no address")
    }

    /// Open the MASQUE session and start the userspace stack on top of it
    /// (with `--carry`, the stack of the carried device).
    pub async fn connect(&self) -> Result<WarpStack> {
        let net = self.warp.connect().await.warped()?;
        self.keep_files();
        Ok(net)
    }

    /// Keep the door that carried data (issue 133) and the leg that changed
    /// (issue 126). A failure to write is not an error.
    fn keep_files(&self) {
        let Some(leg_path) = self.leg_file.as_ref().filter(|_| !self.foreign) else {
            return;
        };
        let now = unix_now();
        if let Some(door) = self.warp.won_door() {
            let path = leg_path.with_file_name(
                leg_path
                    .file_name()
                    .map(|n| n.to_string_lossy().replace("-leg.json", "-door.json"))
                    .unwrap_or_else(|| "warp-door.json".into()),
            );
            let old = std::fs::read_to_string(&path).ok();
            if old
                .and_then(|t| pod_warp::door::door_from(&t, now, &[door]))
                .is_none()
            {
                let text = pod_warp::door::door_text(door, now);
                if let Err(err) = write_private(&path, text.as_bytes()) {
                    tracing::debug!(error = %err, path = %path.display(), "the door file is not written");
                }
            }
        }
        if let (Some(leg), Some(relay)) = (
            self.warp.take_leg_change(),
            self.warp.settings().relay.as_ref(),
        ) {
            let text = pod_warp::leg::leg_text(leg, now, &relay.name);
            if let Err(err) = write_private(leg_path, text.as_bytes()) {
                tracing::debug!(error = %err, path = %leg_path.display(), "the leg file is not written");
            }
        }
    }

    /// The address of the front door of the last session: the override of
    /// `--masque-endpoint`, else the address that worked, else the first of
    /// the device record (the IPv6-only kit, 0001).
    pub fn endpoint_in_use(&self) -> Option<SocketAddr> {
        match self.warp.endpoint_in_use()? {
            Target::Addr(addr) => Some(addr),
            Target::Name { .. } => None,
        }
    }

    /// Open the session without the userspace stack, for callers that only want
    /// the raw packet pipe.
    pub async fn connect_raw(&self) -> Result<MasqueSession> {
        if self.carry.is_some() {
            anyhow::bail!(
                "--carry is for the userspace stack (warp proxy, run, nc, probe), not for warp tun"
            );
        }
        let session = self.warp.open_session().await.warped()?;
        self.keep_files();
        Ok(session)
    }

    /// Open the stack, and keep it open: see [`Live`]. Each new session
    /// keeps the door file and the leg file.
    pub async fn connect_live(self: Arc<Self>) -> Result<Arc<Live<WarpStack>>> {
        let first = self.connect().await?;
        let client = self.clone();
        let connect: pod_warp::live::Connect<WarpStack> = Box::new(move || {
            let client = client.clone();
            Box::pin(async move {
                let net = client.warp.connect().await?;
                client.keep_files();
                Ok(net)
            })
        });
        Ok(live::start(first, connect))
    }

    /// Measure each of `candidates` and the front doors of the device
    /// record (issue 139), at most 8 at the same time. The result is in
    /// the order of the total time; a failure goes last.
    pub async fn scan(
        &self,
        candidates: &[SocketAddr],
        timeout: std::time::Duration,
    ) -> Result<Vec<(SocketAddr, Result<session::DoorTimes>)>> {
        use futures::StreamExt as _;
        let pinned = self
            .device()
            .peer_public_key_pem()
            .context("the device has no peer key to pin; podnet does not connect without one")
            .and_then(|pem| pod_warp::identity::spki_from_pem(pem).warped())
            .context("parsing the pinned peer key")?;
        let mut all: Vec<SocketAddr> = Vec::new();
        for a in candidates
            .iter()
            .copied()
            .chain(self.device().masque_endpoints())
        {
            if !all.contains(&a) {
                all.push(a);
            }
        }
        let sni = self.masque.sni.as_deref();
        let proxy = self.proxy.as_ref();
        let identity = self.identity();
        let mut out: Vec<_> = futures::stream::iter(all)
            .map(|a| {
                let pinned = pinned.clone();
                async move {
                    let r = session::probe_door(a, identity, pinned, proxy, sni, timeout).await;
                    (a, r)
                }
            })
            .buffer_unordered(8)
            .collect()
            .await;
        out.sort_by_key(|(a, r)| match r {
            Ok(t) => (0, t.total(), *a),
            Err(_) => (1, std::time::Duration::ZERO, *a),
        });
        Ok(out)
    }
}

/// Write `bytes` to `path` with mode 0600 (`pod-warp`'s private file).
pub(crate) use pod_warp::files::write_private;

/// `write_private` for the commands of the binary.
pub fn write_private_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    write_private(path, bytes)
}

/// Refuse `path` when its owner is another user, or when its group or other
/// users may read or write it.
pub(crate) fn check_private(path: &Path) -> Result<()> {
    pod_warp::files::check_private(path, pod_warp::Foreign::Refuse)
        .map(drop)
        .warped()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_files_beside_an_identity_file() {
        assert_eq!(
            door_file_of(Path::new("/s/warp.json")),
            Path::new("/s/warp-door.json")
        );
        assert_eq!(
            leg_file_of(Path::new("/s/warp.json")),
            Path::new("/s/warp-leg.json")
        );
    }

    // The classes of `pod-warp` give podnet's exit codes.
    #[test]
    fn a_failure_keeps_its_exit_code() {
        use pod_core::{Class, Stage};
        let code = |class| crate::exit::code_of(&failed(Failure::new(Stage::Warp, class, "x")));
        assert_eq!(code(Class::Usage), crate::exit::USAGE);
        assert_eq!(code(Class::Config), crate::exit::CONFIG);
        assert_eq!(code(Class::Unavailable), crate::exit::UNAVAILABLE);
        assert_eq!(code(Class::Protocol), crate::exit::UNAVAILABLE);
    }

    // A loose identity file is refused with the remedy (issue 253: a
    // directory is a usage error).
    #[test]
    fn a_loose_identity_file_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state/podnet/warp.json");
        write_private(&path, b"{}").unwrap();
        check_private(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let err = format!("{:#}", check_private(&path).unwrap_err());
        assert!(err.contains("chmod 600"), "{err}");
        let err = check_private(dir.path()).unwrap_err();
        assert_eq!(crate::exit::code_of(&err), crate::exit::USAGE, "{err:#}");
    }
}
