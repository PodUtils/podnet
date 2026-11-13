//! WARP as a network: `warp` and `warp:identity=FILE` (T-097, T-099, T-180,
//! T-061).
//!
//! The network registers a WARP device (or loads the one of its identity
//! file), opens a MASQUE session, and gives a way out for TCP and UDP. The
//! session is `pod-warp`'s `Live`: a session that closes is replaced, and
//! the new flows take the new one. The probe of `Live` is off here: the probe
//! of the router decides (it has a control probe, so an outage of the host
//! renews no link).
//!
//! The identity file is `FILE`, else `warp.json` in `Ctx::state_dir` (a
//! router with `state_dir` keeps one directory for each network), else the
//! default file of `podnet warp`. A reconnect and a rotate read the same
//! file, so they register no new device. A rotate enrols a new MASQUE key
//! for the stored device (`Network::rotate`).
//!
//! The router renews a member of this network ([`Network::renews`]) when its
//! probe gives `AuthExpired` or takes it to `Down`, and the member has an
//! egress check ([`Network::egress_check`]): the trace of 1.1.1.1 through
//! the session must say that WARP is on.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pod_warp::live::{Connect, Live, LiveSettings, WAIT_FOR_NEW};
use tokio::time::Instant;

use crate::network::{
    failed, BoxFuture, Caps, Class, Connected, Ctx, Datagrams, Dial, EgressCheck, Failure, Link,
    Network, ProbeResult, Stage, Target, Way,
};
use crate::warp::exit::Exit;
use crate::warp::WarpStack;

/// What a spec `warp` or `warp:identity=FILE` names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spec {
    /// The identity file of the spec, if any.
    pub identity: Option<PathBuf>,
}

/// True for the forms of this network.
pub fn is_warp(spec: &str) -> bool {
    spec == "warp" || spec.starts_with("warp:")
}

/// Parse `warp` or `warp:identity=FILE`.
pub fn parse(spec: &str) -> Result<Spec, Failure> {
    let usage = |why: &str| Failure::new(Stage::Connect, Class::Usage, format!("{spec}: {why}"));
    if spec == "warp" {
        return Ok(Spec { identity: None });
    }
    let rest = spec.strip_prefix("warp:").unwrap_or_default();
    match rest.strip_prefix("identity=") {
        Some("") => Err(usage("give the path of the identity file")),
        Some(path) => Ok(Spec {
            identity: Some(PathBuf::from(path)),
        }),
        None => Err(usage("the forms are warp and warp:identity=FILE")),
    }
}

impl Spec {
    /// The spec as text.
    pub fn label(&self) -> String {
        match &self.identity {
            Some(path) => format!("warp:identity={}", path.display()),
            None => "warp".into(),
        }
    }
}

/// A WARP device that is ready to open sessions.
pub trait Device: Send + Sync {
    /// Open a session with its stack.
    fn connect(&self) -> BoxFuture<'_, anyhow::Result<WarpStack>>;
    /// Enrol a new MASQUE key for the device. The old key stops working.
    fn rotate_key(&mut self) -> BoxFuture<'_, anyhow::Result<()>>;
    /// Write the device and its key to `path`.
    fn save(&self, path: &Path) -> anyhow::Result<()>;
}

/// Where a network gets its device and its trace.
pub trait Source: Send + Sync {
    /// The device of the identity file `path`: registered when the file
    /// holds none. With no path, a device that is not kept.
    fn open<'a>(&'a self, path: Option<&'a Path>)
        -> BoxFuture<'a, anyhow::Result<Box<dyn Device>>>;
    /// The exit of the session `net` (the trace of 1.1.1.1).
    fn trace<'a>(&'a self, net: &'a dyn Dial) -> BoxFuture<'a, Result<Exit, Failure>>;
}

/// The source of the binary: `WarpClient` and the trace with the TLS client
/// of podnet.
struct Production;

impl Device for crate::warp::WarpClient {
    fn connect(&self) -> BoxFuture<'_, anyhow::Result<WarpStack>> {
        Box::pin(crate::warp::WarpClient::connect(self))
    }

    fn rotate_key(&mut self) -> BoxFuture<'_, anyhow::Result<()>> {
        Box::pin(crate::warp::WarpClient::rotate_key(self))
    }

    fn save(&self, path: &Path) -> anyhow::Result<()> {
        crate::warp::WarpClient::save(self, path)
    }
}

impl Source for Production {
    fn open<'a>(
        &'a self,
        path: Option<&'a Path>,
    ) -> BoxFuture<'a, anyhow::Result<Box<dyn Device>>> {
        Box::pin(async move {
            let client = match path {
                Some(path) => crate::warp::WarpClient::register_or_load(path).await?,
                None => crate::warp::WarpClient::register().await?,
            };
            Ok(Box::new(client) as Box<dyn Device>)
        })
    }

    fn trace<'a>(&'a self, net: &'a dyn Dial) -> BoxFuture<'a, Result<Exit, Failure>> {
        Box::pin(async move {
            let tls = crate::warp::tls_client()
                .map_err(|e| Failure::new(Stage::Warp, Class::Config, format!("{e:#}")))?;
            pod_warp::exit::trace(net, &tls).await
        })
    }
}

/// The WARP network of a spec.
pub struct WarpNetwork {
    spec: Spec,
    source: Arc<dyn Source>,
    live: LiveSettings,
}

impl WarpNetwork {
    /// The network of the binary.
    pub fn new(spec: Spec) -> WarpNetwork {
        let mut live = crate::warp::live::settings();
        live.probe = None;
        WarpNetwork {
            spec,
            source: Arc::new(Production),
            live,
        }
    }

    /// The network over another source and other settings of the sessions
    /// (the tests).
    #[cfg(test)]
    pub fn with_source(spec: Spec, source: Arc<dyn Source>, live: LiveSettings) -> WarpNetwork {
        WarpNetwork { spec, source, live }
    }

    /// The identity file: the spec, else the directory of the state, else the
    /// default file. `None`: the device is not kept.
    fn identity(&self, ctx: &Ctx) -> anyhow::Result<Option<PathBuf>> {
        if let Some(path) = &self.spec.identity {
            return Ok(Some(path.clone()));
        }
        if let Some(dir) = &ctx.state_dir {
            return Ok(Some(dir.join("warp.json")));
        }
        crate::warp::WarpClient::default_identity_path()
    }

    fn refuse_below(&self, ctx: &Ctx) -> Result<(), Failure> {
        match &ctx.below {
            None => Ok(()),
            Some(below) => Err(Failure::new(
                Stage::Connect,
                Class::Unsupported,
                format!(
                    "{} cannot go over another way ({}) yet",
                    self.spec.label(),
                    below.label()
                ),
            )),
        }
    }

    /// Open the first session of `device`. A device that the edge refuses
    /// (the enrolled key is not accepted) enrols a new key once, for the
    /// same device, and tries again: it makes no new device.
    async fn first(
        &self,
        device: &mut Box<dyn Device>,
        path: Option<&Path>,
    ) -> anyhow::Result<WarpStack> {
        match device.connect().await {
            Err(err) if refused(&err) && path.is_some() => {
                eprintln!(
                    "podnet: {}: the edge refused the key of the stored device; enrolling a new key for it",
                    self.spec.label()
                );
                device.rotate_key().await?;
                if let Some(path) = path {
                    device.save(path)?;
                }
                device.connect().await
            }
            other => other,
        }
    }

    async fn link(&self, mut device: Box<dyn Device>, path: Option<&Path>) -> anyhow::Result<Link> {
        let first = self.first(&mut device, path).await?;
        let device: Arc<dyn Device> = Arc::from(device);
        let last = Arc::new(Mutex::new(None::<(Class, String)>));
        let connect: Connect<WarpStack> = {
            let last = last.clone();
            Box::new(move || {
                let (device, last) = (device.clone(), last.clone());
                Box::pin(async move {
                    match device.connect().await {
                        Ok(net) => {
                            *lock(&last) = None;
                            Ok(net)
                        }
                        Err(err) => {
                            let failure = failure_of(&err);
                            *lock(&last) = Some((failure.class, failure.chain()));
                            Err(failure)
                        }
                    }
                })
            })
        };
        let live = Live::start(first, connect, &self.live);
        let dial = Arc::new(LiveDial {
            live: live.clone(),
            last,
        });
        let status = serde_json::json!({
            "scheme": "warp",
            "spec": self.spec.label(),
            "identity": path.map(|p| p.display().to_string()),
        });
        let ends = live.clone();
        let close: crate::network::Close = Box::new(move || {
            Box::pin(async move {
                let current = ends.get_within(Duration::ZERO).await;
                drop(ends);
                if let Ok(net) = current {
                    net.close("the link of the network ends");
                }
            })
        });
        Ok(Link::new(Some(Way::destination(dial)))
            .with_status(status)
            .on_close(close))
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The failure in the chain of `err`, else `Unavailable` at the WARP stage.
/// The TLS alert `access_denied` is `Refused` (see [`ACCESS_DENIED`]).
fn failure_of(err: &anyhow::Error) -> Failure {
    let denied = err
        .chain()
        .any(|cause| cause.to_string().contains(ACCESS_DENIED));
    for cause in err.chain() {
        if let Some(f) = cause.downcast_ref::<Failure>() {
            let class = if denied { Class::Refused } else { f.class };
            return Failure::new(f.stage, class, f.chain());
        }
    }
    let class = if denied {
        Class::Refused
    } else {
        Class::Unavailable
    };
    Failure::new(Stage::Warp, class, format!("{err:#}"))
}

/// The text of the TLS alert `access_denied` in the error of an I/O of the
/// session (the edge sends it for a key that it does not know; the error
/// keeps no typed alert, so the text is the only mark; ask of podcrates: class
/// it `Refused`).
const ACCESS_DENIED: &str = "received fatal alert: AccessDenied";

/// True when the edge refused the credential of the device: the answer 403
/// to the CONNECT-IP request, or the TLS alert `access_denied`.
fn refused(err: &anyhow::Error) -> bool {
    failure_of(err).class == Class::Refused
}

impl Network for WarpNetwork {
    fn scheme(&self) -> &'static str {
        "warp"
    }

    fn caps(&self) -> Caps {
        Caps::EGRESS_TCP.with(Caps::EGRESS_UDP)
    }

    fn up<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, anyhow::Result<Link>> {
        Box::pin(async move {
            self.refuse_below(ctx)?;
            let path = self.identity(ctx)?;
            let fresh = path.as_deref().is_some_and(|p| !p.exists());
            let device = self.source.open(path.as_deref()).await?;
            if let (true, Some(path)) = (fresh, &path) {
                eprintln!(
                    "podnet: {}: registered a WARP device, kept in {}",
                    self.spec.label(),
                    path.display()
                );
            }
            self.link(device, path.as_deref()).await
        })
    }

    /// A new MASQUE key for the stored device, and a session with it. With
    /// no identity file, the device is not kept: a rotate is a new link.
    fn rotate<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, anyhow::Result<Link>> {
        Box::pin(async move {
            self.refuse_below(ctx)?;
            let Some(path) = self.identity(ctx)? else {
                return self.up(ctx).await;
            };
            let mut device = self.source.open(Some(&path)).await?;
            device.rotate_key().await?;
            device.save(&path)?;
            eprintln!(
                "podnet: {}: enrolled a new key for the device of {}",
                self.spec.label(),
                path.display()
            );
            self.link(device, Some(&path)).await
        })
    }

    fn renews(&self) -> bool {
        true
    }

    /// The trace of 1.1.1.1 through the member: WARP must be `on` or `plus`,
    /// else `WrongEgress`.
    fn egress_check(&self) -> Option<EgressCheck> {
        let source = self.source.clone();
        Some(Arc::new(move |way: Way, timeout: Duration| {
            let source = source.clone();
            Box::pin(async move {
                let start = Instant::now();
                match tokio::time::timeout(timeout, source.trace(&*way.dial)).await {
                    Ok(Ok(exit)) => match exit.warp.as_deref() {
                        Some("on" | "plus") => ProbeResult::Ok(start.elapsed()),
                        _ => ProbeResult::WrongEgress,
                    },
                    Ok(Err(failure)) => failed(&failure),
                    Err(_) => ProbeResult::ConnectFailed { clean: false },
                }
            })
        }))
    }
}

/// The way of a WARP link: the session of now, which `Live` replaces when it
/// closes.
struct LiveDial {
    live: Arc<Live<WarpStack>>,
    /// The class and the text of the last failure to open a session.
    last: Arc<Mutex<Option<(Class, String)>>>,
}

impl LiveDial {
    /// The session of now. While none is open, the failure of the last try
    /// to open one, when the edge refused the credential (the probe reads
    /// it as `AuthExpired`).
    async fn session(&self, deadline: Instant) -> Result<Arc<WarpStack>, Failure> {
        let limit = deadline
            .saturating_duration_since(Instant::now())
            .min(WAIT_FOR_NEW);
        match self.live.get_within(limit).await {
            Ok(net) => Ok(net),
            Err(err) => match lock(&self.last).clone() {
                Some((class, text)) if matches!(class, Class::Refused | Class::Revoked) => {
                    Err(Failure::new(Stage::Warp, class, text))
                }
                _ => Err(err),
            },
        }
    }
}

impl Dial for LiveDial {
    fn label(&self) -> String {
        "WARP".into()
    }

    fn dial<'a>(
        &'a self,
        target: &'a Target,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(async move { self.session(deadline).await?.dial(target, deadline).await })
    }

    fn dial_addrs<'a>(
        &'a self,
        host: &'a str,
        addrs: &'a [std::net::SocketAddr],
        deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(async move {
            self.session(deadline)
                .await?
                .dial_addrs(host, addrs, deadline)
                .await
        })
    }

    fn names_inside(&self) -> bool {
        true
    }

    fn datagrams<'a>(&'a self) -> BoxFuture<'a, Result<Arc<dyn Datagrams>, Failure>> {
        Box::pin(async move {
            self.session(Instant::now() + WAIT_FOR_NEW)
                .await?
                .datagrams()
                .await
        })
    }
}
