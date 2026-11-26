//! Each outbound TCP connection of podnet: the dial of `pod-proxy`
//! (PodUtils/podcrates) with podnet's settings, directly or through a proxy.
//!
//! [`dial`] is the one path. It takes the proxy of the choice of the process:
//!
//! * `--proxy URL` or `PODNET_PROXY` ([`set_proxy`]), a value of the user
//!   that can be a command;
//! * `--no-proxy` ([`set_direct`]);
//! * else the variables, in the order of curl: `HTTPS_PROXY`, `https_proxy`,
//!   `ALL_PROXY`, `all_proxy`; else the proxy of a tool's file (git, npm,
//!   pip, `/etc/environment`).
//!
//! `NO_PROXY` (or `no_proxy`) names the targets that do not use the proxy,
//! `--proxy`'s too, by the rules of [`rules`]. A proxy can be HTTP
//! (`CONNECT`, Basic or Digest) or SOCKS5 (`socks5://` resolves the name on
//! this host, `socks5h://` gives the name to the proxy), on TCP, on a Unix
//! socket or at a command. A direct dial resolves the name with the chain
//! of [`crate::resolve`] and races its addresses (RFC 8305). Each dial is
//! one decision `egress` of [`crate::decisions`].
//!
//! The words and the hops of a failure are podnet's: a proxy's refusal is a
//! [`Refusal`], and a failure through a proxy is at the hop `proxy`.

use std::net::SocketAddr;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::Result;
use pod_core::{BoxFuture, Class, Connected, Dial, Failure, Stage, Target};
use pod_proxy::{Dialer, Environment, Origin, Rules, Settings};
use tokio::net::TcpStream;
use tokio::time::Instant;

use crate::hop::{self, Hop};
use crate::net::BoxedIo;

pub use pod_proxy::{
    group_errors, Carrier, ChildIo, DialError, NoProxy, Proxy, ProxyKind, ATTEMPT_DELAY,
};

/// The variables of the proxy, in the order that podnet reads them.
pub const PROXY_VARIABLES: &[&str] = &pod_proxy::PROXY_VARS_UPPER_FIRST;
/// The variables of the list of targets that do not use the proxy.
pub const NO_PROXY_VARIABLES: &[&str] = &pod_proxy::NO_PROXY_VARS_UPPER_FIRST;
/// podnet's rules of the proxy settings: the upper case first, a URL that
/// cannot be used passed over with a warning, `http://` with no port on port 80
/// (the value of `pod-proxy`, Go and Python; curl reads 1080), an
/// entry `host:port` of `NO_PROXY` for that port only, and the loopback
/// through the proxy unless `NO_PROXY` names it.
pub fn rules() -> Rules {
    Rules {
        proxy_vars: &pod_proxy::PROXY_VARS_UPPER_FIRST,
        http_proxy_vars: &pod_proxy::HTTP_PROXY_VARS_UPPER_FIRST,
        no_proxy_vars: &pod_proxy::NO_PROXY_VARS_UPPER_FIRST,
        skip_unusable: true,
        no_proxy_ports: true,
        loopback_direct: false,
        ..Rules::default()
    }
}

/// The proxy of a URL by podnet's rules, as a variable gives it: no command.
pub fn parse(spec: &str) -> Result<Proxy> {
    check_unix_path(spec)?;
    hide_secrets(spec);
    Proxy::parse(spec).map_err(anyhow::Error::msg)
}

/// Give each secret part of the proxy spec `spec` (a password, the words of
/// an exec command) to the report, so that no line shows it (#754).
pub fn hide_secrets(spec: &str) {
    for secret in Proxy::secret_parts(spec) {
        crate::report::hide(&secret);
    }
}

/// T-349: refuse a proxy on a Unix socket whose path starts with `//HOST` or
/// holds `@` (exit 64). The URL form `socks5h+unix://USER:PASS@HOST` has no
/// authority. Since v0.45.0, `pod-proxy` refuses these paths too (their
/// entry 612); this check stays for the exit 64 of a usage fault and for
/// `unix:PATH` of the pod hops, which `pod-proxy` does not parse. The error
/// names the form of the spec, never its path. The forms are
/// `SCHEME+unix:PATH`, `SCHEME://unix:/PATH` and `unix:PATH`.
pub fn check_unix_path(spec: &str) -> Result<()> {
    let spec = spec.trim();
    let scheme_only = |s: &str| !s.is_empty() && !s.contains(['/', ':', '@']);
    if spec
        .split_once("+exec:")
        .is_some_and(|(s, _)| scheme_only(s))
    {
        return Ok(());
    }
    let unix = (spec.split_once("+unix:").filter(|(s, _)| scheme_only(s)))
        .map(|(s, path)| (format!("{s}+unix:"), path))
        .or_else(|| {
            let (s, path) = spec.split_once("://unix:")?;
            (scheme_only(s) && path.starts_with('/')).then(|| (format!("{s}://unix:"), path))
        })
        .or_else(|| Some(("unix:".to_string(), spec.strip_prefix("unix:")?)));
    match unix {
        Some((form, path)) if has_authority(path) => {
            Err(crate::exit::fault(
                crate::exit::Fault::Usage,
                anyhow::anyhow!(
                    "a proxy on a Unix socket ({form}) takes an absolute path with no user, password or host: \
                     give {form}/PATH"
                ),
            ))
        }
        _ => Ok(()),
    }
}

/// A Unix path with an authority (`//HOST`) or with `@`. `///PATH` has an
/// empty authority, as `socks5+unix:///run/proxy.sock` of the pod hops.
fn has_authority(path: &str) -> bool {
    path.contains('@') || (path.starts_with("//") && !path.starts_with("///"))
}

/// A variable of the environment for the proxy settings. A proxy URL that
/// [`check_unix_path`] refuses is passed over with a warning that does not
/// show it.
fn guarded_var(name: &str) -> Option<std::ffi::OsString> {
    let value = std::env::var_os(name)?;
    if NO_PROXY_VARIABLES.contains(&name) {
        return Some(value);
    }
    match check_unix_path(&value.to_string_lossy()) {
        Ok(()) => Some(value),
        Err(e) => {
            tracing::warn!("{name} is passed over: {e:#}");
            None
        }
    }
}

/// How podnet reaches the network, as `podnet status` names it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ProxyChoice {
    /// The proxy of the variables, if one is set (the default).
    #[default]
    FromEnvironment,
    /// No proxy (`--no-proxy`).
    Direct,
    /// This proxy (`--proxy URL`).
    Via(Proxy),
}

/// The choice of the process, its dialer and its resolver.
struct Way {
    choice: ProxyChoice,
    dialer: Arc<Dialer>,
    resolver: Arc<pod_dns::Resolver>,
}

static WAY: OnceLock<Way> = OnceLock::new();

/// The way of the process: the one set, else the variables and the proxy
/// of a tool's file, read at the first use.
fn way() -> &'static Way {
    WAY.get_or_init(|| {
        // `pod-proxy` (v0.45.0, their entry 612) refuses the Unix path of a
        // tool's file with `//HOST` or `@`: its warning names no path.
        let env = Environment::read_with(&guarded_var, rules())
            .or_detected(&guarded_var, &pod_proxy::detect::read_file);
        let (dialer, resolver) = dialer_and_resolver(pod_proxy::ProxyChoice::FromEnvironment(env));
        Way {
            choice: ProxyChoice::FromEnvironment,
            dialer: Arc::new(dialer),
            resolver,
        }
    })
}

/// Use the proxy that the user gave (`--proxy`, `PODNET_PROXY`): it can be
/// a command, and `NO_PROXY` still applies. Call it before the first
/// connection; a later call has no effect. The proxy, or why `spec` cannot
/// be used.
pub fn set_proxy(spec: &str) -> Result<Proxy> {
    check_unix_path(spec)?;
    let env = Environment::read_with(&guarded_var, rules())
        .with_proxy(spec, Origin::User("--proxy".into()));
    let proxy = match env.proxy.as_ref().map(|given| &given.value) {
        Some(Ok(proxy)) => proxy.clone(),
        Some(Err(why)) => anyhow::bail!("{why}"),
        None => anyhow::bail!("the value names no proxy"),
    };
    let (dialer, resolver) = dialer_and_resolver(pod_proxy::ProxyChoice::FromEnvironment(env));
    let _ = WAY.set(Way {
        choice: ProxyChoice::Via(proxy.clone()),
        dialer: Arc::new(dialer),
        resolver,
    });
    Ok(proxy)
}

/// Connect directly (`--no-proxy`). Call it before the first connection.
pub fn set_direct() {
    let (dialer, resolver) = dialer_and_resolver(pod_proxy::ProxyChoice::Direct);
    let _ = WAY.set(Way {
        choice: ProxyChoice::Direct,
        dialer: Arc::new(dialer),
        resolver,
    });
}

/// The choice of this process.
pub fn choice() -> ProxyChoice {
    way().choice.clone()
}

/// The resolver of the process: the chain of [`crate::resolve`], whose DoH
/// queries take the way of the process.
pub fn resolver() -> Arc<pod_dns::Resolver> {
    way().resolver.clone()
}

/// A dialer with podnet's settings: its program, no user agent and no
/// `TCP_NOTSENT_LOWAT`, its resolver (with no DoH for a proxy's own host:
/// a DoH query can need the same proxy), and its counters.
fn dialer(proxy: pod_proxy::ProxyChoice) -> Dialer {
    dialer_and_resolver(proxy).0
}

/// [`dialer`] and its resolver: the chain of [`crate::resolve`], whose DoH
/// queries go through the same proxy with the chain of no DoH.
fn dialer_and_resolver(proxy: pod_proxy::ProxyChoice) -> (Dialer, Arc<pod_dns::Resolver>) {
    let mut settings = Settings::new("podnet", proxy);
    settings.lowat = None;
    let plain = crate::resolve::plain();
    let doh_dial = Dialer::new(settings.clone(), plain.clone())
        .with_proxy_resolver(plain.clone())
        .with_observer(counters());
    if pod_core::Dial::names_inside(&doh_dial) {
        crate::resolve::note_names_inside();
    }
    let resolver = Arc::new(crate::resolve::with_doh(Arc::new(doh_dial)));
    let dialer = Dialer::new(settings, resolver.clone())
        .with_proxy_resolver(plain)
        .with_observer(counters());
    (dialer, resolver)
}

/// The proxy for `host:port`, after `NO_PROXY`, or `None` for a direct
/// connection.
pub fn proxy_for(host: &str, port: u16) -> Option<Proxy> {
    way().dialer.proxy_at(host, port).ok().flatten()
}

/// Connect to `host:port`: through `via` when it is given, else through the
/// proxy of [`proxy_for`], else directly.
pub async fn dial(
    host: &str,
    port: u16,
    via: Option<&Proxy>,
    timeout: Duration,
) -> Result<BoxedIo> {
    // EGRESS-3 and RUN-1: the policy decides before any dial, and a
    // refusal carries its reason instead of a timeout.
    if let Err(refused) = crate::policy::permit(host, port) {
        return Err(crate::exit::fault(
            crate::exit::Fault::Unavailable,
            anyhow::Error::new(refused),
        ));
    }
    match via {
        Some(proxy) => {
            open(
                &dialer(pod_proxy::ProxyChoice::Via(proxy.clone())),
                host,
                port,
                timeout,
            )
            .await
        }
        None => open(&way().dialer, host, port, timeout).await,
    }
}

/// Connect to `host:port` with no proxy: each address of the name, with
/// [`ATTEMPT_DELAY`] between the starts. The policy of the process decides
/// before the first connect, as it does for [`dial`].
pub async fn dial_direct(host: &str, port: u16, timeout: Duration) -> Result<TcpStream> {
    if let Err(refused) = crate::policy::permit(host, port) {
        return Err(crate::exit::fault(
            crate::exit::Fault::Unavailable,
            anyhow::Error::new(refused),
        ));
    }
    dialer(pod_proxy::ProxyChoice::Direct)
        .tcp(host, port, timeout)
        .await
        .map_err(|e| failed(e, host, port, None))
}

/// Connect to the first of `addrs` that answers, with no proxy. The
/// attempts start [`ATTEMPT_DELAY`] apart, or at once when the attempt
/// before fails. Gives the index of the address and the stream.
///
/// The policy of the process decides for each address before the first
/// connect, as [`dial_direct`] does. A refused address is never attempted,
/// and its reason names the address.
pub async fn dial_addrs(addrs: &[SocketAddr], timeout: Duration) -> Result<(usize, TcpStream)> {
    for addr in addrs {
        crate::policy::permit(&addr.ip().to_string(), addr.port()).map_err(|refused| {
            crate::exit::fault(crate::exit::Fault::Unavailable, anyhow::Error::new(refused))
        })?;
    }
    pod_proxy::dial_addrs(addrs, timeout)
        .await
        .map_err(|e| error_of(&e))
}

/// [`dial`] with `dialer`, and podnet's words of a failure.
async fn open(dialer: &Dialer, host: &str, port: u16, timeout: Duration) -> Result<BoxedIo> {
    let proxy = dialer.proxy_at(host, port).ok().flatten();
    match dialer.stream(host, port, timeout).await {
        Ok((stream, _)) => Ok(Box::new(stream)),
        Err(e) => Err(failed(e, host, port, proxy.as_ref())),
    }
}

/// A failed dial in podnet's words: through a proxy, `HOST:PORT through
/// the proxy URL`, at the hop `proxy`, with its [`Refusal`]; directly,
/// `connecting to HOST:PORT`, at the hop `dns` when the name did not
/// resolve.
fn failed(e: DialError, host: &str, port: u16, proxy: Option<&Proxy>) -> anyhow::Error {
    let at = pod_proxy::authority(host, port);
    match proxy {
        Some(proxy) => {
            let e = proxy_named_once(e);
            let err = match &e {
                DialError::ProxyRefused { status, reason, .. } => {
                    anyhow::Error::new(Refusal::of(*status, reason))
                }
                // #754: the last line of the stderr of an exec command can
                // hold the words of the command.
                DialError::ProxyExited { word, status, said } => {
                    error_of(&DialError::ProxyExited {
                        word: word.clone(),
                        status: status.clone(),
                        said: crate::report::mask(said),
                    })
                }
                _ => error_of(&e),
            };
            // #762: a proxy that answers 502 or 504 (a SOCKS5 reply of the
            // far side is a 502) reached out and the target failed: the hop
            // is the target, not the proxy.
            let hop = match &e {
                DialError::ProxyRefused {
                    status: 502 | 504, ..
                } => Hop::Connect,
                _ => Hop::Proxy,
            };
            hop::at(
                hop,
                err.context(format!("{at} through the proxy {}", proxy.label())),
            )
        }
        None => {
            let stage = e.stage_and_class().0;
            // A timeout of the dial already names the step (`connecting to
            // HOST:PORT timed out ...`): name it once (T-452).
            let words = format!("connecting to {at}");
            let err = error_of(&e);
            // #753 (podcrates v0.86.0): the line of a name that does not
            // resolve names the host itself; no lead that names it again.
            let err = match err.to_string().starts_with(&words) || stage == Stage::Dns {
                true => err,
                false => err.context(words),
            };
            match stage {
                Stage::Dns => hop::at(Hop::Dns, err),
                Stage::Proxy => hop::at(Hop::Proxy, err),
                _ => err,
            }
        }
    }
}

/// A failed dial `e` of podnet as the [`Failure`] of a [`Dial`]: the stage
/// of its hop (else `stage`), and the text of its chain, each cause once.
/// A dial that failed at the proxy has its [`DialError`] as the cause:
/// `pod-ws` keeps a cause of that type as it is, and makes any other cause
/// a `Connect` with no stage (GitHub #697).
pub(crate) fn dial_failure(e: anyhow::Error, stage: Stage, class: Class) -> Failure {
    let stage = match hop::of(&e) {
        Some(Hop::Proxy) => Stage::Proxy,
        Some(Hop::Dns) => Stage::Dns,
        _ => stage,
    };
    let text = crate::exit::text(&e);
    // A time limit keeps its DialError too, so that its words name the
    // limit of the dial: pod-ws writes "timed out after 0s" for a timeout
    // with no DialError (#761).
    let dial = e.chain().find_map(|c| c.downcast_ref::<DialError>());
    let typed = match (stage, dial) {
        (Stage::Proxy, Some(dial)) => Some(dial.clone()),
        (_, Some(dial @ DialError::Timeout { .. })) => Some(dial.clone()),
        _ => None,
    };
    match typed {
        Some(dial) => Failure::caused(stage, class, text, dial),
        None => Failure::caused(stage, class, text, e),
    }
}

/// The class of a failed dial `e` (GitHub #719): the class of the
/// [`DialError`] in its chain (a time limit, a name with no address), else
/// the class of its error of I/O, else `default`.
pub(crate) fn class_of(e: &anyhow::Error, default: Class) -> Class {
    if let Some(dial) = e.chain().find_map(|c| c.downcast_ref::<DialError>()) {
        return dial.stage_and_class().1;
    }
    match e.chain().find_map(|c| c.downcast_ref::<std::io::Error>()) {
        Some(io) => Class::of_io(io.kind()),
        None => default,
    }
}

/// `e` with the proxy named once: pod-proxy writes `could not reach the
/// proxy P: could not connect to P: ...` (GitHub #697).
fn proxy_named_once(e: DialError) -> DialError {
    match e {
        DialError::ProxyUnreachable { proxy, detail } => {
            let lead = format!("could not connect to {proxy}");
            let detail = pod_core::failure::strip_lead(&detail, &lead).to_string();
            DialError::ProxyUnreachable { proxy, detail }
        }
        other => other,
    }
}

/// The error of a dial: when every address failed, the error of I/O of the
/// last one, with its kind and the words of each address (`podnet doctor`
/// reads a refusal of this host by the kind); else the error of the dial.
fn error_of(e: &DialError) -> anyhow::Error {
    match e {
        DialError::Connect {
            kind: Some(kind),
            detail,
            ..
        } => anyhow::Error::new(std::io::Error::new(*kind, detail.clone())),
        _ => anyhow::Error::new(e.clone()),
    }
}

/// A proxy answered and refused the connection: an HTTP status that is not
/// 2xx (`status`), or a SOCKS5 refusal (`status` 0). A lost connection is
/// not a refusal.
#[derive(Debug, Clone)]
pub struct Refusal {
    pub status: u16,
    pub text: String,
}

impl Refusal {
    /// The refusal of a proxy's `status` and `reason` (`pod-proxy` gives a
    /// SOCKS5 refusal the status of its HTTP case, and a reason that starts
    /// with `SOCKS5`).
    fn of(status: u16, reason: &str) -> Refusal {
        if reason.starts_with("SOCKS5") {
            return Refusal {
                status: 0,
                text: reason.to_string(),
            };
        }
        let text = match reason.trim() {
            "" => format!("the proxy refused CONNECT with HTTP {status}"),
            reason => format!("the proxy refused CONNECT with HTTP {status}: {reason}"),
        };
        Refusal { status, text }
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text)
    }
}

impl std::error::Error for Refusal {}

/// The refusal of a proxy in the chain of `err`, if there is one.
pub fn refusal(err: &anyhow::Error) -> Option<&Refusal> {
    // An error with a hop holds the refusal in its own chain.
    err.chain().find_map(|e| {
        e.downcast_ref::<Refusal>().or_else(|| {
            e.downcast_ref::<hop::AtHop>()
                .and_then(|h| refusal(h.inner()))
        })
    })
}

/// The dial of podnet for a crate that takes a [`Dial`] (the HTTP client of
/// [`crate::fetch`]): [`dial`], through `via` or the choice of the process,
/// with podnet's words; the error is the cause of the failure.
pub struct Egress {
    via: Option<Proxy>,
}

/// The datagrams of a way, with the policy of the process in front of each
/// send (EGRESS-3 and EGRESS-4). A peer-supplied UDP target is refused with
/// its reason before any datagram leaves.
#[derive(Debug)]
pub struct CheckedDatagrams {
    inner: Arc<dyn pod_core::Datagrams>,
}

impl CheckedDatagrams {
    /// Wrap `inner`.
    pub fn new(inner: Arc<dyn pod_core::Datagrams>) -> CheckedDatagrams {
        CheckedDatagrams { inner }
    }
}

impl pod_core::Datagrams for CheckedDatagrams {
    fn poll_writable(
        &self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        self.inner.poll_writable(cx)
    }

    fn try_send(&self, transmit: &pod_core::Transmit<'_>) -> std::io::Result<()> {
        crate::policy::permit_io(
            &transmit.destination.ip().to_string(),
            transmit.destination.port(),
        )?;
        self.inner.try_send(transmit)
    }

    fn poll_recv(
        &self,
        cx: &mut std::task::Context<'_>,
        bufs: &mut [std::io::IoSliceMut<'_>],
        meta: &mut [pod_core::RecvMeta],
    ) -> std::task::Poll<std::io::Result<usize>> {
        self.inner.poll_recv(cx, bufs, meta)
    }

    fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.inner.local_addr()
    }

    fn max_transmit_segments(&self) -> usize {
        self.inner.max_transmit_segments()
    }

    fn max_receive_segments(&self) -> usize {
        self.inner.max_receive_segments()
    }

    fn may_fragment(&self) -> bool {
        self.inner.may_fragment()
    }
}

/// The dial of the process, or through `via` alone.
pub fn egress(via: Option<Proxy>) -> Arc<dyn Dial> {
    Arc::new(Egress { via })
}

/// The way out of one request through `proxy`, or the client's with none
/// ([`pod_proxy::client::Request::via`]).
pub fn via(proxy: Option<&Proxy>) -> Option<Arc<dyn Dial>> {
    proxy.map(|p| egress(Some(p.clone())))
}

impl Dial for Egress {
    fn label(&self) -> String {
        match &self.via {
            Some(proxy) => proxy.label(),
            None => way().dialer.label(),
        }
    }

    fn proxy_label(&self, target: &Target) -> Option<String> {
        match &self.via {
            Some(proxy) => Some(proxy.to_string()),
            None => proxy_for(&target.host(), target.port()).map(|p| p.to_string()),
        }
    }

    fn dial<'a>(
        &'a self,
        target: &'a Target,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(async move {
            let limit = deadline.saturating_duration_since(Instant::now());
            match dial(&target.host(), target.port(), self.via.as_ref(), limit).await {
                Ok(io) => Ok(Connected::new(Box::new(io))),
                Err(e) => {
                    let class = class_of(&e, Class::Unavailable);
                    Err(dial_failure(e, Stage::Connect, class))
                }
            }
        })
    }

    fn names_inside(&self) -> bool {
        match &self.via {
            Some(proxy) => proxy.kind() != (ProxyKind::Socks5 { remote_dns: false }),
            None => way().dialer.names_inside(),
        }
    }

    /// GitHub #746: the way of the process carries UDP when it has no proxy
    /// (an IPv4 socket of the system, as the dialer `Direct`); a proxy
    /// carries no UDP, and the refusal names it.
    fn datagrams<'a>(&'a self) -> BoxFuture<'a, Result<Arc<dyn pod_core::Datagrams>, Failure>> {
        let proxy = match &self.via {
            Some(proxy) => Some(proxy.label()),
            None => match choice() {
                ProxyChoice::Direct => None,
                _ => proxy_for("udp.invalid", 53).map(|p| p.label()),
            },
        };
        match proxy {
            None => Box::pin(async move {
                let any = std::net::SocketAddr::from((std::net::Ipv4Addr::UNSPECIFIED, 0));
                let socket = crate::network::SystemDatagrams::bind(any)
                    .await
                    .map_err(|e| {
                        Failure::io(Stage::Connect, format!("a UDP socket on {any}"), e)
                    })?;
                Ok(Arc::new(CheckedDatagrams::new(Arc::new(socket)))
                    as Arc<dyn pod_core::Datagrams>)
            }),
            Some(proxy) => Box::pin(async move {
                Err(Failure::new(
                    Stage::Connect,
                    Class::Unavailable,
                    format!("the proxy {proxy} carries no UDP"),
                ))
            }),
        }
    }
}

/// The counters of the decisions of [`crate::decisions`].
struct Counters;

/// [`Counters`], for a dialer or a resolver.
pub(crate) fn counters() -> Arc<dyn pod_core::Observer> {
    Arc::new(Counters)
}

impl pod_core::Observer for Counters {
    fn decision(&self, decision: &'static str, outcome: &str) {
        crate::decisions::count(decision, outcome);
    }
}

/// The status line of a proxy answer to `CONNECT` (the fuzz target).
pub(crate) fn parse_status(head: &[u8]) -> Result<u16> {
    let line = String::from_utf8_lossy(head);
    let (status, _) = pod_proxy::http::parse_status_line(&line).map_err(anyhow::Error::msg)?;
    Ok(status)
}

#[cfg(test)]
#[path = "egress_tests.rs"]
mod tests;
