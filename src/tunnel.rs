//! The tunnel is pod-cloudflare's. This file gives it the host of podnet:
//! the TLS context and the way out of the process, the resolver with the
//! pins of `--resolve`, the system resolver for the SRV record of the edge,
//! the relay of `--transport relay`, the WARP device of `--warp-identity`,
//! the services of podnet (files, git, shares), its log and its counters,
//! and the settings from its variables (`PODNET_EDGE`, `PODNET_UDP`,
//! `PODNET_VERIFY_STRICT`, `PODNET_READY_TIMEOUT`).

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use futures::future::BoxFuture;
use pod_cloudflare::host::{EdgeRelay, SystemSrv, WarpSession, WarpSessions};
use pod_core::{Class, Connected, Dial, Failure, Stage, Target};

use crate::config::Config;
use crate::metrics::Metrics;
use crate::service::ServiceRuntime;
use crate::warp::WarpClient;

pub use pod_cloudflare::tunnel::*;
pub use pod_cloudflare::{Host, Settings, Words};

/// The name and the version of podnet that the edge sees.
pub const CLIENT_VERSION: &str = concat!("podnet/", env!("CARGO_PKG_VERSION"));

/// The host of one tunnel of podnet, its services and its counters.
pub struct Hosted {
    pub host: Arc<Host>,
    pub services: Arc<ServiceRuntime>,
    pub metrics: Metrics,
}

impl Hosted {
    /// A builder of a quick tunnel over this host, with its counters.
    pub fn quick(&self) -> TunnelBuilder {
        TunnelBuilder::quick(self.host.clone()).metrics(self.metrics.clone())
    }
}

/// The host of a new tunnel of podnet: give it to [`Tunnel::quick`].
pub fn host() -> Result<Arc<Host>> {
    Ok(hosted()?.host)
}

/// The host of a new tunnel of podnet, with the services of podnet that
/// it serves and the counters of the tunnel.
pub fn hosted() -> Result<Hosted> {
    let settings = settings()?;
    let metrics = crate::metrics::podnet();
    let services = Arc::new(ServiceRuntime::new(metrics.clone()));
    let mut host = base_host();
    host.services = Arc::new(crate::proxy::Services(services.clone()));
    host.relay = crate::ws_relay::active().map(|_| EdgeRelay {
        dial: Arc::new(RelayDial),
        context: crate::ws_relay::shared(),
    });
    host.warp = Some(Arc::new(Warp::default()));
    host.settings = settings;
    Ok(Hosted {
        host: Arc::new(host),
        services,
        metrics,
    })
}

/// The host of podnet with no services, no relay, no WARP and the default
/// settings with the words of podnet: for the discovery of the edge.
pub fn base_host() -> Host {
    crate::net::init_crypto();
    crate::resolve::refresh_pins();
    let mut host = Host::new(crate::trust::context().clone(), crate::egress::egress(None));
    host.resolver = crate::egress::resolver();
    host.system_srv = Some(Arc::new(SystemResolver));
    host.origin_no_verify = Some(crate::origin::dial::no_verify_config());
    host.report = crate::decisions::report();
    host.settings.client_version = CLIENT_VERSION.to_string();
    host.settings.words = crate::udp::words();
    host
}

/// Start the tunnel of `config` on a new host of podnet. Gives the handle
/// and the services (the controls of the shares). A refused registration
/// that a new start can fix gets one more start ([`again`]).
pub async fn start(config: Config) -> Result<(TunnelHandle, Arc<ServiceRuntime>)> {
    let tunnel = match config.credentials {
        crate::config::Credentials::Quick => crate::hop::TunnelKind::Quick,
        _ => crate::hop::TunnelKind::Named,
    };
    let wait = pod_core::backoff::backoff(AGAIN_ROUND);
    again(tunnel, wait, || start_once(config.clone())).await
}

/// The round of `pod_core::backoff` of the wait before the second start:
/// 2 s to 6 s.
const AGAIN_ROUND: u32 = 3;

/// One start of the tunnel of `config` on a new host of podnet.
async fn start_once(config: Config) -> Result<(TunnelHandle, Arc<ServiceRuntime>)> {
    let hosted = hosted()?;
    let handle = hosted.quick().config(config).build()?.start().await?;
    Ok((handle, hosted.services))
}

/// Run `attempt`. When the edge refuses the registration and a new start
/// can fix it ([`crate::hop::EdgeRefusal::transient`]), wait `wait` and
/// run `attempt` one more time (live 45169011007). pod-cloudflare already
/// tries the registration 3 times in about 2 s. For a quick tunnel, the
/// second start makes a new quick tunnel: a second request to the
/// quick-tunnel service only after a failure. Each other failure ends the
/// start at once.
pub async fn again<T, F, Fut>(
    tunnel: crate::hop::TunnelKind,
    wait: Duration,
    mut attempt: F,
) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let err = match attempt().await {
        Ok(value) => return Ok(value),
        Err(err) => crate::hop::read_refusal(err, tunnel),
    };
    if !crate::hop::refusal_of(&err).is_some_and(|r| r.transient()) {
        return Err(err);
    }
    let what = match tunnel {
        crate::hop::TunnelKind::Quick => "a new quick tunnel",
        crate::hop::TunnelKind::Named => "the registration",
    };
    tracing::warn!(
        kind = "tunnel.again",
        "{err:#}; podnet tries {what} again in {} s",
        wait.as_secs()
    );
    tokio::time::sleep(wait).await;
    attempt()
        .await
        .map_err(|e| crate::hop::read_refusal(e, tunnel))
}

/// The settings of a tunnel from the variables of podnet.
pub fn settings() -> Result<Settings> {
    Ok(Settings {
        edge: edge_override()?,
        udp: crate::udp::choice_from_env()?,
        provision_budget: crate::detach::provision_budget(
            pod_cloudflare::quicktunnel::PROVISION_BUDGET,
        ),
        client_version: CLIENT_VERSION.to_string(),
        verify_strict: verify_strict(),
        words: crate::udp::words(),
        ..Settings::default()
    })
}

/// Read `PODNET_EDGE`, a comma-separated `host:port` list that replaces edge
/// discovery (as `cloudflared --edge` does).
pub fn edge_override() -> Result<Option<Vec<SocketAddr>>> {
    let Ok(list) = std::env::var("PODNET_EDGE") else {
        return Ok(None);
    };
    let mut overridden = Vec::new();
    for item in list.split(',').map(str::trim).filter(|i| !i.is_empty()) {
        let socket: SocketAddr = item
            .parse()
            .with_context(|| format!("PODNET_EDGE entry {item:?} is not host:port"))
            .map_err(|e| crate::exit::fault(crate::exit::Fault::Config, e))?;
        overridden.push(socket);
    }
    Ok((!overridden.is_empty()).then_some(overridden))
}

/// True when a check of the URL that is not proved is an error
/// (`PODNET_VERIFY_STRICT` is set to a value that is not empty or `0`).
pub fn verify_strict() -> bool {
    std::env::var("PODNET_VERIFY_STRICT").is_ok_and(|v| !v.is_empty() && v != "0")
}

/// The leg plan of `protocol` over `transport`, in the words of podnet.
pub fn leg_plan(
    protocol: crate::config::Protocol,
    transport: crate::config::Transport,
) -> Result<Vec<LegKind>> {
    Ok(pod_cloudflare::tunnel::leg_plan(
        protocol,
        transport,
        &crate::udp::words(),
    )?)
}

/// The WARP device of the leg `quic+warp`: `--warp-identity` and
/// `--masque-endpoint`.
#[derive(Clone, Debug, Default)]
pub struct WarpOptions {
    pub identity: Option<PathBuf>,
    pub masque_endpoint: Option<SocketAddr>,
}

static WARP_OPTIONS: Mutex<Option<WarpOptions>> = Mutex::new(None);

/// Use `options` for the WARP device of each tunnel of this process.
pub fn set_warp(options: WarpOptions) {
    *WARP_OPTIONS.lock().unwrap_or_else(|e| e.into_inner()) = Some(options);
}

/// The WARP options of this process.
pub fn warp_options() -> WarpOptions {
    WARP_OPTIONS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .unwrap_or_default()
}

/// The WARP sessions of a tunnel: the first one registers the device (or
/// loads it from `--warp-identity`); a later one uses the same device.
#[derive(Default)]
struct Warp {
    client: tokio::sync::OnceCell<WarpClient>,
}

impl Warp {
    async fn client(&self) -> Result<&WarpClient> {
        self.client
            .get_or_try_init(|| async {
                let options = warp_options();
                let client = match &options.identity {
                    Some(path) => WarpClient::register_or_load(path).await,
                    None => WarpClient::register().await,
                }
                .context("registering with WARP")?;
                Ok::<_, anyhow::Error>(client.with_masque(crate::warp::MasqueOptions {
                    endpoint: options.masque_endpoint,
                    ..Default::default()
                }))
            })
            .await
    }

    async fn open_session(&self) -> Result<WarpSession> {
        let stack = Arc::new(self.client().await?.connect().await?);
        let endpoint = crate::warp::quic::endpoint(&stack)
            .await
            .context("building the QUIC socket inside the WARP session")?;
        let watched = stack.clone();
        Ok(WarpSession {
            endpoint,
            closed: Box::new(move || watched.is_closed()),
            keep: Box::new(stack),
        })
    }
}

impl WarpSessions for Warp {
    fn open(&self) -> BoxFuture<'_, Result<WarpSession, Failure>> {
        Box::pin(async move {
            self.open_session()
                .await
                .map_err(|e| failure(Stage::Warp, Class::Unavailable, e))
        })
    }
}

/// The SRV records of the system resolver (`/etc/resolv.conf`).
struct SystemResolver;

impl SystemSrv for SystemResolver {
    fn srv<'a>(&'a self, name: &'a str) -> BoxFuture<'a, Result<Vec<(String, u16)>, Failure>> {
        Box::pin(async move {
            system_srv(name)
                .await
                .map_err(|e| failure(Stage::Dns, Class::Unavailable, e))
        })
    }
}

async fn system_srv(name: &str) -> Result<Vec<(String, u16)>> {
    let resolver = hickory_resolver::TokioResolver::builder_tokio()
        .context("reading the system resolver configuration")?
        .build()
        .context("starting the system resolver")?;
    let srv = resolver
        .srv_lookup(name)
        .await
        .map_err(|e| anyhow::anyhow!(srv_words(&e)))?;
    Ok(srv
        .answers()
        .iter()
        .filter_map(|record| match &record.data {
            hickory_resolver::proto::rr::RData::SRV(srv) => Some((srv.target.to_utf8(), srv.port)),
            _ => None,
        })
        .collect())
}

/// The words of a failed SRV lookup of the system resolver (GitHub #696):
/// hickory writes the Debug form of its query (`Query { name: Name(..) }`)
/// and its cause again; the caller names the record.
fn srv_words(err: &hickory_resolver::net::NetError) -> String {
    use hickory_resolver::net::{DnsError, NetError};
    use hickory_resolver::proto::op::ResponseCode;
    match err {
        NetError::Dns(DnsError::NoRecordsFound(none))
            if none.response_code == ResponseCode::NXDomain =>
        {
            "the name does not exist (NXDOMAIN)".to_string()
        }
        NetError::Dns(DnsError::NoRecordsFound(_)) => "no SRV records".to_string(),
        NetError::Dns(DnsError::ResponseCode(code)) => format!("the resolver answered {code}"),
        NetError::Timeout => "no answer in time".to_string(),
        other => crate::exit::tidy(&other.to_string()),
    }
}

/// A TCP connection to the edge through the relay of `--transport relay`.
struct RelayDial;

impl Dial for RelayDial {
    fn label(&self) -> String {
        crate::ws_relay::active().map_or_else(|| "relay".to_string(), |r| r.url())
    }

    fn dial<'a>(
        &'a self,
        target: &'a Target,
        deadline: tokio::time::Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(async move {
            let Some(relay) = crate::ws_relay::active() else {
                return Err(Failure::new(Stage::Relay, Class::Config, "no relay is set"));
            };
            let Target::Addr(addr) = target else {
                let text = format!("the relay takes an address, not the name {}", target.host());
                return Err(Failure::new(Stage::Relay, Class::Unsupported, text));
            };
            let limit = deadline
                .saturating_duration_since(tokio::time::Instant::now())
                .max(Duration::from_secs(1));
            let io = relay
                .connect(*addr, limit)
                .await
                .map_err(|e| failure(Stage::Relay, Class::Unavailable, e))?;
            Ok(Connected::new(Box::new(io)))
        })
    }
}

/// A failure of the glue with the text of `err` and its chain.
fn failure(stage: Stage, class: Class, err: anyhow::Error) -> Failure {
    Failure::caused(stage, class, crate::exit::text(&err), err)
}

#[cfg(test)]
mod tests {
    use crate::hop::TunnelKind::{self, Named, Quick};

    /// GitHub #696: the words of a failed SRV lookup of the system
    /// resolver have no Debug form of the query of hickory, and say the
    /// cause once.
    #[test]
    fn a_failed_srv_lookup_has_no_debug_form() {
        use hickory_resolver::net::{DnsError, NetError, NoRecords};
        use hickory_resolver::proto::op::{Query, ResponseCode};
        use hickory_resolver::proto::rr::{Name, RecordType};
        let name = Name::from_ascii("_v2-origintunneld._tcp.argotunnel.com.").unwrap();
        let query = Query::query(name, RecordType::SRV);
        let none = |code| {
            NetError::Dns(DnsError::NoRecordsFound(NoRecords::new(
                query.clone(),
                code,
            )))
        };
        let cases = [
            (none(ResponseCode::NoError), "no SRV records"),
            (
                none(ResponseCode::NXDomain),
                "the name does not exist (NXDOMAIN)",
            ),
            (
                NetError::Dns(DnsError::ResponseCode(ResponseCode::ServFail)),
                "the resolver answered Server Failure",
            ),
            (NetError::Timeout, "no answer in time"),
        ];
        for (err, want) in cases {
            assert_eq!(super::srv_words(&err), want, "{err}");
        }
    }

    /// A failure as pod-cloudflare v0.82.0 gives it when the edge refused
    /// the registration of the only leg.
    fn refused(cause: &str, retry: bool) -> anyhow::Error {
        let retry = if retry { "yes" } else { "no" };
        anyhow::Error::new(pod_core::Failure::new(
            pod_core::Stage::EdgeRegister,
            pod_core::Class::Unavailable,
            format!("no edge leg worked: quic: the edge refused the registration: {cause} (retry: {retry}, after 1 s; podnet speaks cloudflared 2026.11.1)"),
        ))
    }

    /// Live 45169011007: a stand-in of the start answers with the
    /// refusals of `answers`, then accepts. A refusal that a new start can
    /// fix gets one more start, and only one; each other failure gets
    /// none.
    #[tokio::test]
    async fn a_refusal_that_a_new_start_can_fix_gets_one_more_start() {
        let not_found = "Unauthorized: Tunnel not found";
        let bad_token = "Unauthorized: Invalid tunnel secret";
        // (the kind, the refusals before the accept, the starts, the result)
        type Case<'a> = (TunnelKind, &'a [(&'a str, bool)], usize, Result<usize, i32>);
        let cases: [Case; 7] = [
            (Quick, &[], 1, Ok(1)),
            (Quick, &[(not_found, true)], 2, Ok(2)),
            (Quick, &[(not_found, false)], 2, Ok(2)),
            (Quick, &[(not_found, true), (not_found, true)], 2, Err(75)),
            (Named, &[(not_found, true)], 2, Ok(2)),
            (Named, &[(not_found, false)], 1, Err(69)),
            (Quick, &[(bad_token, false)], 1, Err(69)),
        ];
        for (kind, answers, starts, want) in cases {
            let mut calls = 0;
            let got = super::again(kind, std::time::Duration::ZERO, || {
                calls += 1;
                let answer = match answers.get(calls - 1) {
                    Some((cause, retry)) => Err(refused(cause, *retry)),
                    None => Ok(calls),
                };
                async move { answer }
            })
            .await
            .map_err(|e| crate::exit::code_of(&e));
            assert_eq!((calls, got), (starts, want), "{kind:?} {answers:?}");
        }
    }

    /// The host of a tunnel speaks with the words of podnet, counts into
    /// the metrics of podnet and has podnet's configuration of noTLSVerify.
    #[test]
    fn the_host_is_the_host_of_podnet() {
        let hosted = super::hosted().unwrap();
        let settings = &hosted.host.settings;
        assert_eq!(settings.words, crate::udp::words());
        assert_eq!(settings.client_version, super::CLIENT_VERSION);
        assert!(hosted.host.origin_no_verify.is_some());
        assert!(hosted.host.system_srv.is_some());
        let text = hosted.metrics.prometheus();
        assert!(text.contains("podnet_streams_total 0"), "{text}");
        assert!(hosted.services.is_empty());
    }
}
