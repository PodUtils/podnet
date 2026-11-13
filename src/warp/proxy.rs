//! A local SOCKS5 and HTTP `CONNECT` proxy whose connections go through WARP:
//! `pod-warp`'s proxy, with podnet's listeners, its flags (the limit of
//! clients, the credentials, the resolvers, the rules) as values of the
//! process, and its health line.
//!
//! Programs that read `ALL_PROXY` or `HTTPS_PROXY` use it directly. `podnet
//! warp run` puts any other program behind it with podnet-netns.

use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use pod_core::{BoxFuture, Connected, Dial, Failure, Target};
use pod_warp::proxy::{Proxy, Stats, Tunnel};
use podnet_netns::listen::ProxyListen;
use podnet_netns::socks::ProxyHost;
use tokio::task::JoinHandle;
use tracing::info;

use super::ip::WarpStack;
use super::Warped as _;

pub use super::ip::NO_CHECK_VARIABLE;
pub use crate::services::tunnel_resolver;
pub use pod_warp::proxy::{order, HEALTH_EVERY};
pub use pod_warp::resolver::Resolver;

/// The time limit of a connect through the tunnel or from the host.
const CONNECT_TIMEOUT: Duration = pod_warp::proxy::CONNECT_TIMEOUT;

/// A running proxy.
pub struct WarpProxy {
    /// Where the proxy accepts clients.
    pub listen: ProxyListen,
    task: JoinHandle<()>,
}

impl WarpProxy {
    /// Stop the proxy.
    pub fn abort(&self) {
        self.task.abort();
    }
}

/// A bound listener of the proxy, before the WARP session (issue 97: a
/// bind that fails is known before podnet waits for the network).
pub enum Bound {
    Tcp(tokio::net::TcpListener, ProxyListen),
    Unix(tokio::net::UnixListener, ProxyListen),
}

impl Bound {
    /// The address of the listener (the port that the kernel chose too).
    pub fn listen(&self) -> &ProxyListen {
        match self {
            Bound::Tcp(_, listen) | Bound::Unix(_, listen) => listen,
        }
    }
}

/// Bind `listen`: a TCP listener, or a unix socket of mode 0600.
pub async fn bind(listen: &ProxyListen) -> Result<Bound> {
    match listen {
        ProxyListen::Tcp(addr) => {
            let listener = crate::net::bind_tcp(*addr).await?;
            let actual = listener.local_addr().unwrap_or(*addr);
            Ok(Bound::Tcp(listener, ProxyListen::Tcp(actual)))
        }
        ProxyListen::Unix(path) => {
            // Issue 115: a socket that answers belongs to a running proxy:
            // it is not taken. Before, it was removed, and the running
            // proxy lost its clients.
            if std::os::unix::net::UnixStream::connect(path).is_ok() {
                bail!(
                    "unix:{} is served by another process (podnet ps; podnet warp down stops podnet warp up)",
                    path.display()
                );
            }
            crate::net::claim_socket_path(path, false)?;
            let listener = podnet_netns::listen::bind_private(path)
                .with_context(|| format!("binding unix:{}", path.display()))?;
            Ok(Bound::Unix(listener, listen.clone()))
        }
    }
}

/// Bind each of `listens` on its own: the listeners that work, and the
/// error of each one that does not.
pub async fn bind_all(listens: &[ProxyListen]) -> (Vec<Bound>, Vec<(ProxyListen, anyhow::Error)>) {
    let mut bound = Vec::new();
    let mut failed = Vec::new();
    for listen in listens {
        match bind(listen).await {
            Ok(b) => bound.push(b),
            Err(err) => failed.push((listen.clone(), err)),
        }
    }
    (bound, failed)
}

/// The counters of the proxy clients of this process (issue 117).
pub fn stats() -> &'static Arc<Stats> {
    static STATS: std::sync::LazyLock<Arc<Stats>> = std::sync::LazyLock::new(Default::default);
    &STATS
}

/// The most clients at the same time (`--max-connections`); 0 is no limit.
static MAX_CONNECTIONS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Set the most clients at the same time (issue 117); 0 is no limit.
pub fn set_max_connections(n: u64) {
    MAX_CONNECTIONS.store(n, std::sync::atomic::Ordering::Relaxed);
}

static CREDENTIALS: std::sync::OnceLock<pod_proxy::Credentials> = std::sync::OnceLock::new();

/// Set the credentials that each client of the proxies of this process
/// must give (issue 146). A later call does nothing.
pub fn set_credentials(credentials: pod_proxy::Credentials) {
    let _ = CREDENTIALS.set(credentials);
}

/// True when the clients must give credentials.
pub fn has_credentials() -> bool {
    CREDENTIALS.get().is_some()
}

/// The variable of the resolvers of the tunnel (a comma-separated list of
/// [`Resolver::parse`]).
pub const RESOLVERS_VARIABLE: &str = "PODNET_WARP_RESOLVERS";

static RESOLVERS: std::sync::OnceLock<Vec<Resolver>> = std::sync::OnceLock::new();

/// Parse a list of resolvers (`--resolver`, the variable).
pub fn parse_resolvers(specs: &[String]) -> Result<Vec<Resolver>> {
    pod_warp::resolver::parse_resolvers(specs).warped()
}

/// Set the resolvers of the tunnel of this process (issue 145). Call it
/// before the first session; a later call does nothing.
pub fn set_resolvers(list: Vec<Resolver>) {
    if !list.is_empty() {
        let _ = RESOLVERS.set(list);
    }
}

/// The resolvers of [`RESOLVERS_VARIABLE`], when it is set and valid.
fn env_resolvers() -> Option<Vec<Resolver>> {
    let value = std::env::var(RESOLVERS_VARIABLE).ok()?;
    parse_resolvers(&[value]).ok().filter(|l| !l.is_empty())
}

/// The resolvers that were set (`--resolver`), else the ones of
/// [`RESOLVERS_VARIABLE`]; empty for the resolver of the tunnel.
fn resolvers_given() -> Vec<Resolver> {
    RESOLVERS
        .get()
        .cloned()
        .or_else(env_resolvers)
        .unwrap_or_default()
}

/// The resolvers of the tunnel: the ones that were set, else
/// [`tunnel_resolver`].
pub fn resolver_list() -> Vec<Resolver> {
    let list = resolvers_given();
    if list.is_empty() {
        vec![Resolver::Udp(tunnel_resolver())]
    } else {
        list
    }
}

/// True when resolvers were set (not the default).
pub fn resolvers_set() -> bool {
    !resolvers_given().is_empty()
}

/// The resolvers of the tunnel, with podnet's TLS client for DNS over TLS
/// (issue 144).
pub fn resolvers() -> pod_warp::resolver::Resolvers {
    let mut resolvers = pod_warp::resolver::Resolvers::new(resolvers_given());
    match super::tls_client() {
        Ok(tls) => resolvers.tls = Some(tls),
        Err(err) => tracing::warn!(error = %format!("{err:#}"), "no TLS client for DNS over TLS"),
    }
    resolvers
}

/// The names of the proxies: the pins of `--resolve` first, else the
/// resolvers of the tunnel.
pub fn names() -> pod_warp::proxy::names::Names {
    pod_warp::proxy::names::Names {
        resolvers: resolvers(),
        pins: Some(Arc::new(crate::resolve::pinned)),
    }
}

/// The rule `direct`: a connection from the host, not through the tunnel,
/// and through no proxy of podnet (it can be this proxy).
struct Direct;

impl Dial for Direct {
    fn label(&self) -> String {
        "direct".into()
    }

    fn dial<'a>(
        &'a self,
        target: &'a Target,
        deadline: tokio::time::Instant,
    ) -> BoxFuture<'a, std::result::Result<Connected, Failure>> {
        Box::pin(async move {
            let limit = deadline.saturating_duration_since(tokio::time::Instant::now());
            match crate::egress::dial_direct(&target.host(), target.port(), limit).await {
                Ok(stream) => Ok(Connected::new(Box::new(stream))),
                Err(e) => Err(Failure::caused(
                    pod_core::Stage::Connect,
                    pod_core::Class::Unavailable,
                    format!("{e:#}"),
                    e,
                )),
            }
        })
    }
}

/// The dial of the rule `direct`.
pub fn direct() -> Arc<dyn Dial> {
    Arc::new(Direct)
}

/// A proxy over `proxy` (a session, or the L4 door) with the values of
/// this process.
fn with_values(mut proxy: Proxy) -> Arc<Proxy> {
    proxy.rules = super::rules::rules().clone();
    proxy.names = names();
    proxy.credentials = CREDENTIALS.get().cloned();
    proxy.max_connections = MAX_CONNECTIONS.load(std::sync::atomic::Ordering::Relaxed);
    proxy.stats = stats().clone();
    proxy.report = super::report();
    Arc::new(proxy)
}

/// The proxy of the session of `live`: each client takes the session of
/// the time it comes; after a reconnect (issue 75), new clients take the
/// new session.
fn over(live: Arc<super::Live<WarpStack>>) -> Arc<Proxy> {
    with_values(Proxy::new(live as Arc<dyn Tunnel>, direct()))
}

/// Accept clients on `bound` with `proxy`.
fn serve_on(proxy: Arc<Proxy>, bound: Bound) -> WarpProxy {
    let listen = bound.listen().clone();
    let task = match bound {
        Bound::Tcp(listener, _) => proxy.run_tcp(listener),
        Bound::Unix(listener, _) => proxy.run_unix(listener),
    };
    info!(%listen, "WARP proxy is listening");
    WarpProxy { listen, task }
}

/// Accept proxy clients on `bound` and connect them through the WARP
/// session of `live`.
pub fn run(live: Arc<super::Live<WarpStack>>, bound: Bound) -> WarpProxy {
    serve_on(over(live), bound)
}

/// Accept proxy clients on `bound` and connect each one with its own
/// HTTP/3 CONNECT through the L4 door (issue 140). UDP is not served.
pub fn run_l4(l4: Arc<pod_warp::L4>, bound: Bound) -> WarpProxy {
    serve_on(with_values(Proxy::over_l4(l4, direct())), bound)
}

/// [`bind`] and [`run`] for one listener.
pub async fn serve(live: Arc<super::Live<WarpStack>>, listen: ProxyListen) -> Result<WarpProxy> {
    Ok(run(live, bind(&listen).await?))
}

/// The DNS forwarder of `warp proxy --dns`: each query goes to the
/// resolvers of the tunnel of the session of `live`.
pub fn serve_dns(live: Arc<super::Live<WarpStack>>, socket: tokio::net::UdpSocket) {
    over(live).serve_dns(socket);
}

/// Log one line about the health of the proxy each [`HEALTH_EVERY`]
/// (issue 116: `podnet logs warp` showed nothing after the start).
pub fn health_log(live: Arc<super::Live<WarpStack>>) {
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        let weak = Arc::downgrade(&live);
        let proxy = over(live);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(HEALTH_EVERY);
            tick.tick().await;
            loop {
                tick.tick().await;
                let Some(live) = weak.upgrade() else { return };
                let health = proxy.health();
                let link = super::ip::link_stats();
                info!(
                    clients = health.clients,
                    since_start = health.since_start,
                    refused = health.refused,
                    dns_dropped = health.dns_dropped,
                    reconnects = live.reconnects(),
                    tunnel = health.tunnel,
                    direct = health.direct,
                    rejected = health.rejected,
                    tx_dropped = link.tx_dropped,
                    tx_held = link.tx_held,
                    decisions = %crate::decisions::summary(),
                    "WARP proxy health"
                );
            }
        });
    });
}

/// The target of `host` and `port`.
pub fn target(host: &ProxyHost, port: u16) -> Target {
    match host {
        ProxyHost::Ip(ip) => Target::Addr(std::net::SocketAddr::new(*ip, port)),
        ProxyHost::Domain(name) => Target::Name {
            host: name.clone(),
            port,
        },
    }
}

/// Connect to `host:port` through the tunnel; a name is resolved inside
/// the tunnel (the pins first).
pub async fn dial(stack: Arc<WarpStack>, host: ProxyHost, port: u16) -> Result<pod_core::BoxedIo> {
    let connected = names()
        .connect(&*stack, &target(&host, port), CONNECT_TIMEOUT)
        .await
        .warped()?;
    Ok(connected.io)
}

/// The first address of `name` through the tunnel (IPv4 first; the pins
/// first).
pub async fn resolve(stack: &WarpStack, name: &str) -> Result<IpAddr> {
    names().resolve(stack, name).await.warped()
}

/// The addresses of `name` through the tunnel (the pins first).
pub async fn resolve_all(stack: &WarpStack, name: &str) -> Result<Vec<IpAddr>> {
    names().resolve_all(stack, name).await.warped()
}

/// Check that a packet crosses a session (issue 120): a DNS query to the
/// resolvers of the tunnel gets an answer. Gives the time of the answer.
pub async fn check_data_path(stack: &WarpStack) -> Result<Duration> {
    let check = pod_warp::DataPathCheck::of(&resolvers());
    stack
        .check_data_path(&check)
        .await
        .warped()
        .context("the MASQUE session is open, and no resolver answered through it")
}

#[cfg(test)]
mod tests {
    use super::*;

    // Issue 115: a unix socket that answers is not taken.
    #[tokio::test]
    async fn a_unix_socket_that_answers_is_not_taken() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("warp.sock");
        let first = tokio::net::UnixListener::bind(&path).unwrap();
        let err = match bind(&ProxyListen::Unix(path.clone())).await {
            Err(err) => err,
            Ok(_) => panic!("a live socket was taken"),
        };
        assert!(format!("{err}").contains("another process"), "{err}");
        let client = tokio::net::UnixStream::connect(&path);
        let (accepted, connected) = tokio::join!(first.accept(), client);
        assert!(accepted.is_ok() && connected.is_ok());
        drop(first);
        assert!(
            bind(&ProxyListen::Unix(path)).await.is_ok(),
            "a dead socket is taken"
        );
    }

    /// Issue 97: each listener is bound on its own. A TCP listener that the
    /// host refuses (here: a port in use, and an address that is not on
    /// this host) does not stop the unix socket, which takes clients.
    #[tokio::test]
    async fn a_refused_tcp_listener_does_not_stop_the_unix_socket() {
        crate::test_host::need_loopback!();
        let taken = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("warp.sock");
        let listens = vec![
            ProxyListen::Tcp(taken.local_addr().unwrap()),
            ProxyListen::Tcp("192.0.2.1:40000".parse().unwrap()),
            ProxyListen::Unix(path.clone()),
        ];
        let (bound, failed) = bind_all(&listens).await;
        assert_eq!(bound.len(), 1);
        assert_eq!(failed.len(), 2);
        assert!(format!("{:#}", failed[0].1).contains("binding tcp://"));
        let Some(Bound::Unix(listener, _)) = bound.into_iter().next() else {
            panic!("the unix socket is the listener that works");
        };
        let client = tokio::net::UnixStream::connect(&path);
        let (accepted, connected) = tokio::join!(listener.accept(), client);
        assert!(accepted.is_ok() && connected.is_ok());
        let (bound, failed) = bind_all(&listens[..2]).await;
        assert!(bound.is_empty() && failed.len() == 2);
    }

    // Issue 145: the resolvers of the flag go first; with none, the
    // resolver of the tunnel.
    #[test]
    fn the_resolvers_of_the_tunnel() {
        let list = parse_resolvers(&["1.0.0.1,tls://9.9.9.9#dns.quad9.net".into()]).unwrap();
        assert_eq!(list.len(), 2);
        assert!(parse_resolvers(&["udp://".into()]).is_err());
    }
}
