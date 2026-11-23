//! TURN as a network: `turn://HOST[:PORT]` and `turns://HOST[:PORT]` (T-321).
//!
//! The network gives the datagrams of a host that has only a TURN server
//! as a relay, and a TCP stream through the TCP relay of RFC 6062. Each
//! [`Dial::datagrams`] call makes one UDP allocation on the server
//! (RFC 8656) with `pod-turn`, and the socket sends each datagram to a peer
//! in ChannelData. Each [`Dial::dial`] makes one TCP allocation (RFC 6062),
//! a Connect to the peer and a ConnectionBind, and the bytes of the stream
//! go through it (TURN-1 to TURN-7). podnet has no TURN code of its own.
//!
//! * `turn://HOST[:PORT]`: TURN over UDP for datagrams. A TCP stream uses
//!   plain TURN over TCP to the same server and port, because RFC 6062 needs
//!   a byte stream. Default port 3478.
//! * `turns://HOST[:PORT]`: TURN over TLS on TCP for both paths. Default
//!   port 5349. The TCP connection goes through the way out of the process
//!   (`--proxy`, the variables), or through the way below the network.
//!
//! The user, the password and the realm are settings, not parts of the spec
//! (a spec goes into `argv`, logs and JSON): `PODNET_TURN_USER`,
//! `PODNET_TURN_PASSWORD_FILE` or `PODNET_TURN_PASSWORD`, and
//! `PODNET_TURN_REALM`. The lifetime and the rate limit of the allocation
//! are settings too (`PODNET_TURN_LIFETIME`, `PODNET_TURN_RATE_LIMIT`), and
//! the status of the network gives both (TURN-6). They are read when the
//! network comes up. No text of this module holds a password, and no log
//! line or JSON document holds one (TURN-5).

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use pod_core::Resolve;
use pod_turn::{
    Client, Credentials, Limits, Opened, RelayedStream, Settings, SystemUdp, UdpSource,
};
use tokio::sync::{mpsc, oneshot};
use tokio::time::Instant;

use super::turn_socket::{named, TurnDatagrams, TurnStage};
use super::usage;
use crate::network::{
    BoxFuture, Class, Connected, Datagrams, Dial, Failure, Link, Stage, Target, Way,
};

/// The variable of the user.
pub const USER_VARIABLE: &str = "PODNET_TURN_USER";
/// The variable of the file that holds the password (one line).
pub const PASSWORD_FILE_VARIABLE: &str = "PODNET_TURN_PASSWORD_FILE";
/// The variable of the password. The file is the better choice.
pub const PASSWORD_VARIABLE: &str = "PODNET_TURN_PASSWORD";
/// The variable of the realm that the server must give. Default: the realm
/// of the server.
pub const REALM_VARIABLE: &str = "PODNET_TURN_REALM";
/// The variable of the lifetime of each allocation, in seconds (1 to
/// 3600). Default: the lifetime of `pod-turn` (600 s). A refresh comes
/// before its end (TURN-2).
pub const LIFETIME_VARIABLE: &str = "PODNET_TURN_LIFETIME";
/// The variable of the rate limit of the relay, in bytes for each second.
/// TURN carries no such attribute: it is the caller's policy, and it goes
/// into the status beside the lifetime (TURN-6).
pub const RATE_LIMIT_VARIABLE: &str = "PODNET_TURN_RATE_LIMIT";

const DEFAULT_UDP_PORT: u16 = 3478;
const DEFAULT_TLS_PORT: u16 = 5349;
/// The longest wait for the end of the probe allocation.
const CLOSE_WAIT: Duration = Duration::from_secs(3);

/// A TURN server that a spec names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Spec {
    /// True for `turns://`: TLS on TCP.
    pub(super) tls: bool,
    pub(super) server: Target,
}

impl Spec {
    /// The spec as it is shown: the scheme, the host and the port.
    pub(super) fn label(&self) -> String {
        let scheme = if self.tls { "turns" } else { "turn" };
        format!("{scheme}://{}", self.server)
    }

    fn transport(&self) -> &'static str {
        if self.tls {
            "tls"
        } else {
            "udp"
        }
    }
}

/// True when `spec` is a TURN spec.
pub(super) fn is_turn(spec: &str) -> bool {
    spec.starts_with("turn://") || spec.starts_with("turns://")
}

/// `spec` as an error line shows it: no user, no password, no path and no
/// query. A secret in a wrong place stays out of the error.
fn shown(spec: &str) -> String {
    let (scheme, rest) = spec.split_once("://").unwrap_or(("turn", spec));
    let (login, rest) = match rest.rsplit_once('@') {
        Some((_, host)) => ("***@", host),
        None => ("", rest),
    };
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    format!("{scheme}://{login}{}", &rest[..end])
}

/// Parse `turn://HOST[:PORT]` or `turns://HOST[:PORT]`. The parse reads no
/// variable and opens no file.
pub(super) fn parse(spec: &str) -> Result<Spec, Failure> {
    let (tls, rest) = match spec.strip_prefix("turns://") {
        Some(rest) => (true, rest),
        None => (false, spec.strip_prefix("turn://").unwrap_or(spec)),
    };
    let bad = |why: &str| usage(format!("{}: {why}", shown(spec)));
    if rest.contains('@') {
        return Err(bad(&format!(
            "a spec holds no user and no password; set {USER_VARIABLE} and {PASSWORD_FILE_VARIABLE}"
        )));
    }
    if rest.contains(['/', '?', '#']) {
        return Err(bad(&format!(
            "the form is turn://HOST[:PORT]; the user, the password and the realm are settings ({USER_VARIABLE}, {PASSWORD_FILE_VARIABLE}, {REALM_VARIABLE}), not parts of the spec"
        )));
    }
    let default = if tls {
        DEFAULT_TLS_PORT
    } else {
        DEFAULT_UDP_PORT
    };
    let (host, port) = if let Some(inner) = rest.strip_prefix('[') {
        let (inside, after) = inner
            .split_once(']')
            .ok_or_else(|| bad("an IPv6 address needs a closing ]"))?;
        let port = match after {
            "" => default,
            _ => port_of(
                after
                    .strip_prefix(':')
                    .ok_or_else(|| bad("give :PORT after ]"))?,
            )
            .ok_or_else(|| bad("the port must be 1 to 65535"))?,
        };
        (format!("[{inside}]"), port)
    } else {
        match rest.rsplit_once(':') {
            Some((host, _)) if host.contains(':') => {
                return Err(bad("an IPv6 address needs brackets: [ADDRESS]:PORT"));
            }
            Some((host, port)) => (
                host.to_string(),
                port_of(port).ok_or_else(|| bad("the port must be 1 to 65535"))?,
            ),
            None => (rest.to_string(), default),
        }
    };
    if host.is_empty() || host == "[]" {
        return Err(bad("give the host of the TURN server"));
    }
    Ok(Spec {
        tls,
        server: Target::new(&host, port),
    })
}

fn port_of(text: &str) -> Option<u16> {
    text.parse::<u16>().ok().filter(|p| *p != 0)
}

/// The user, the password and the realm of the allocations.
#[derive(Clone)]
pub(super) struct Account {
    pub(super) user: String,
    pub(super) password: String,
    pub(super) realm: Option<String>,
}

impl Account {
    #[cfg(test)]
    pub(super) fn new(user: &str, password: &str) -> Account {
        Account {
            user: user.to_string(),
            password: password.to_string(),
            realm: None,
        }
    }
}

/// The user alone: the password never leaves the value as text.
impl fmt::Debug for Account {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Account")
            .field("user", &self.user)
            .field("password", &"(hidden)")
            .field("realm", &self.realm)
            .finish()
    }
}

/// Read the account with `get` (the variables of the environment). A fault
/// names the variables and never holds a value.
pub(super) fn account(get: &dyn Fn(&str) -> Option<String>) -> Result<Account, Failure> {
    let set = |name: &str| get(name).filter(|v| !v.is_empty());
    let user = set(USER_VARIABLE).ok_or_else(|| {
        usage(format!(
            "TURN needs a user: set {USER_VARIABLE} (and {PASSWORD_FILE_VARIABLE} or {PASSWORD_VARIABLE})"
        ))
    })?;
    let password = match (set(PASSWORD_FILE_VARIABLE), set(PASSWORD_VARIABLE)) {
        (Some(_), Some(_)) => {
            return Err(usage(format!(
                "set one of {PASSWORD_FILE_VARIABLE} and {PASSWORD_VARIABLE}, not both"
            )))
        }
        (Some(file), None) => {
            let text = std::fs::read_to_string(&file)
                .map_err(|e| usage(format!("{PASSWORD_FILE_VARIABLE}: cannot read {file}: {e}")))?;
            let line = text.strip_suffix('\n').unwrap_or(&text);
            line.strip_suffix('\r').unwrap_or(line).to_string()
        }
        (None, Some(password)) => password,
        (None, None) => {
            return Err(usage(format!(
                "TURN needs a password: set {PASSWORD_FILE_VARIABLE} (a file with one line) or {PASSWORD_VARIABLE}"
            )))
        }
    };
    if password.is_empty() {
        return Err(usage(format!(
            "{PASSWORD_FILE_VARIABLE}: the file holds no password"
        )));
    }
    Ok(Account {
        user,
        password,
        realm: set(REALM_VARIABLE),
    })
}

/// The socket of TURN over UDP through the datagrams of the way below the
/// network. A name of the server goes through the resolver of podnet.
struct BelowUdp {
    below: Arc<dyn Dial>,
    resolve: Arc<dyn Resolve>,
}

impl UdpSource for BelowUdp {
    fn open<'a>(
        &'a self,
        server: &'a Target,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<Opened, Failure>> {
        Box::pin(async move {
            let addr = match server {
                Target::Addr(addr) => *addr,
                Target::Name { host, port } => *self
                    .resolve
                    .resolve(host, *port, deadline)
                    .await?
                    .first()
                    .ok_or_else(|| {
                        Failure::new(
                            Stage::Dns,
                            Class::NotFound,
                            format!("{host} has no address for TURN"),
                        )
                    })?,
            };
            Ok((self.below.datagrams().await?, addr))
        })
    }
}

/// The dialer of the network: one allocation for each datagram socket,
/// and one TCP allocation for each TCP stream (RFC 6062).
pub(super) struct TurnDialer {
    label: String,
    spec: Spec,
    account: Account,
    limits: Limits,
    /// The relay's rate limit in bytes for each second, for the status.
    rate_limit: Option<u64>,
    /// The way for the TCP connection of `turn://` and `turns://`.
    dial: Arc<dyn Dial>,
    /// The socket of `turn://`.
    source: Arc<dyn UdpSource>,
    /// The resolver of a peer's name on the TCP path.
    resolve: Arc<dyn Resolve>,
    /// The thread that opens each TCP relay. `pod_turn::Client::connect`
    /// holds `&Client` across an await, so its future is not `Send`; the
    /// future of [`Dial`] must be `Send`. The relay runs on a thread of its
    /// own with a current-thread runtime, and only the `Send` stream comes
    /// back.
    worker: std::sync::OnceLock<RelayWorker>,
    /// The trust of a `turns://` connection. The process context in a
    /// command, and the CA of the stand in a test.
    tls: Arc<pod_tls::Context>,
}

/// One job of the worker thread: a TCP relay to open, or an allocation
/// that waits for the peer of an RFC 6062 connection (GitHub #485).
enum Job {
    Connect {
        settings: Settings,
        label: String,
        peer: std::net::SocketAddr,
        reply: oneshot::Sender<Result<RelayedStream, Failure>>,
    },
    Accept {
        settings: Settings,
        label: String,
        ready: oneshot::Sender<Result<std::net::SocketAddr, Failure>>,
        accepted: oneshot::Sender<Result<RelayedStream, Failure>>,
    },
}

/// The queue of a worker thread. It ends when the dialer is dropped.
struct RelayWorker {
    jobs: mpsc::UnboundedSender<Job>,
}

/// A TCP allocation of the stand-in, and the peer that connected to it.
#[allow(dead_code)]
pub(super) struct Accepted {
    /// The address that a peer connects to.
    pub(super) relayed: std::net::SocketAddr,
    /// The stream of the peer, which completes when the peer connects.
    pub(super) stream: oneshot::Receiver<Result<RelayedStream, Failure>>,
    /// The dialer whose worker thread owns the sockets of `stream`. The
    /// stream's I/O is registered on that thread's runtime, so it must stay
    /// up while the stream lives; else a read fails with "the context is
    /// being shutdown". Dropping this drops the worker and closes the stream.
    _keep: Option<Arc<TurnDialer>>,
}

impl RelayWorker {
    fn start() -> RelayWorker {
        let (jobs, mut queue) = mpsc::unbounded_channel::<Job>();
        std::thread::Builder::new()
            .name("podnet-turn".into())
            .spawn(move || {
                let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                else {
                    return;
                };
                // A LocalSet takes the future of `Client::connect`, which
                // borrows the client across an await and so is not `Send`.
                // The keeper of each allocation runs on this runtime while
                // the thread waits for the next job.
                let local = tokio::task::LocalSet::new();
                runtime.block_on(local.run_until(async move {
                    while let Some(job) = queue.recv().await {
                        match job {
                            Job::Connect {
                                settings,
                                label,
                                peer,
                                reply,
                            } => {
                                let result = Client::allocate(settings)
                                    .await
                                    .map_err(|e| named(TurnStage::Allocate, &label, &e))
                                    .map(|client| (client, label));
                                let result = match result {
                                    Err(failure) => Err(failure),
                                    Ok((client, label)) => client
                                        .connect(peer)
                                        .await
                                        .map_err(|e| named(TurnStage::Relay, &label, &e)),
                                };
                                let _ = reply.send(result);
                            }
                            Job::Accept {
                                settings,
                                label,
                                ready,
                                accepted,
                            } => {
                                let client = match Client::allocate(settings).await {
                                    Ok(client) => client,
                                    Err(e) => {
                                        let failure = named(TurnStage::Allocate, &label, &e);
                                        let _ = ready.send(Err(failure_of(&failure)));
                                        let _ = accepted.send(Err(failure));
                                        continue;
                                    }
                                };
                                let relayed = client.relayed_address();
                                if ready.send(Ok(relayed)).is_err() {
                                    continue;
                                }
                                let result = client
                                    .accept()
                                    .await
                                    .map_err(|e| named(TurnStage::Relay, &label, &e));
                                let _ = accepted.send(result);
                            }
                        }
                    }
                }));
            })
            .ok();
        RelayWorker { jobs }
    }

    fn open(
        &self,
        settings: Settings,
        label: String,
        peer: std::net::SocketAddr,
    ) -> oneshot::Receiver<Result<RelayedStream, Failure>> {
        let (reply, wait) = oneshot::channel();
        let job = Job::Connect {
            settings,
            label,
            peer,
            reply,
        };
        if self.jobs.send(job).is_err() {
            let (reply, wait) = oneshot::channel();
            let _ = reply.send(Err(Failure::new(
                Stage::Relay,
                Class::Unavailable,
                "the TURN relay worker ended",
            )));
            return wait;
        }
        wait
    }

    /// One allocation that waits for the peer of an RFC 6062 connection
    /// (GitHub #485). The relayed address comes first; the stream of the
    /// peer comes when the peer connects.
    fn listen(
        &self,
        settings: Settings,
        label: String,
    ) -> oneshot::Receiver<Result<Accepted, Failure>> {
        let (reply, wait) = oneshot::channel();
        let (ready, relayed) = oneshot::channel();
        let (accepted, stream) = oneshot::channel();
        let job = Job::Accept {
            settings,
            label,
            ready,
            accepted,
        };
        if self.jobs.send(job).is_err() {
            let (reply, wait) = oneshot::channel();
            let _ = reply.send(Err(Failure::new(
                Stage::Relay,
                Class::Unavailable,
                "the TURN relay worker ended",
            )));
            return wait;
        }
        tokio::spawn(async move {
            let result = match relayed.await {
                Ok(Ok(relayed)) => Ok(Accepted {
                    relayed,
                    stream,
                    _keep: None,
                }),
                Ok(Err(failure)) => Err(failure),
                Err(_) => Err(Failure::new(
                    Stage::Relay,
                    Class::Unavailable,
                    "the TURN relay worker ended",
                )),
            };
            let _ = reply.send(result);
        });
        wait
    }
}

/// A copy of `failure`, for the two waits of an accept job.
fn failure_of(failure: &Failure) -> Failure {
    Failure::new(failure.stage, failure.class, failure.text().to_string())
}

impl TurnDialer {
    fn credentials(&self) -> Credentials {
        Credentials::new(self.account.user.clone(), self.account.password.clone())
    }

    fn settings(&self) -> Settings {
        let mut settings = if self.spec.tls {
            Settings::tls(
                self.spec.server.clone(),
                self.credentials(),
                self.dial.clone(),
                self.tls.clone(),
            )
        } else {
            Settings::udp(
                self.spec.server.clone(),
                self.credentials(),
                self.source.clone(),
            )
        };
        settings.realm = self.account.realm.clone();
        settings.limits = self.limits.clone();
        settings.rate_limit = self.rate_limit;
        settings.software = Some(format!("podnet/{}", env!("CARGO_PKG_VERSION")));
        settings
    }

    /// The settings of a TCP relay (RFC 6062, 4.1): a stream transport,
    /// REQUESTED-TRANSPORT TCP, and the same server, account and lifetime.
    /// `turn://` uses plain TURN over TCP for this path, because RFC 6062
    /// needs a byte stream and its UDP transport cannot carry one.
    fn stream_settings(&self) -> Settings {
        let mut settings = if self.spec.tls {
            Settings::tls(
                self.spec.server.clone(),
                self.credentials(),
                self.dial.clone(),
                self.tls.clone(),
            )
        } else {
            Settings::tcp(
                self.spec.server.clone(),
                self.credentials(),
                self.dial.clone(),
            )
        };
        settings.realm = self.account.realm.clone();
        settings.limits = self.limits.clone();
        settings.rate_limit = self.rate_limit;
        settings.software = Some(format!("podnet/{}", env!("CARGO_PKG_VERSION")));
        settings.allocation.relay = pod_turn::Relay::Tcp;
        settings
    }

    /// One allocation on the server. A fault names the stage `allocate`.
    async fn allocate(&self) -> Result<Client, Failure> {
        Client::allocate(self.settings())
            .await
            .map_err(|e| named(TurnStage::Allocate, &self.label, &e))
    }

    /// One relayed TCP connection to `target` through a TCP allocation
    /// (RFC 6062). The allocation lives while the stream lives, and the
    /// stream refreshes it on its timer.
    async fn connect_tcp(&self, target: &Target, deadline: Instant) -> Result<Connected, Failure> {
        let addr = match target {
            Target::Addr(addr) => *addr,
            Target::Name { host, port } => *self
                .resolve
                .resolve(host, *port, deadline)
                .await?
                .first()
                .ok_or_else(|| {
                    Failure::new(
                        Stage::Dns,
                        Class::NotFound,
                        format!("{host} has no address for the TURN relay"),
                    )
                })?,
        };
        let worker = self.worker.get_or_init(RelayWorker::start).open(
            self.stream_settings(),
            self.label.clone(),
            addr,
        );
        let stream = match tokio::time::timeout_at(deadline, worker).await {
            Err(_) => {
                return Err(Failure::new(
                    Stage::Relay,
                    Class::Timeout,
                    format!(
                        "{}: relay: the TCP allocation of {addr} did not come up in time",
                        self.label
                    ),
                ))
            }
            Ok(Err(_)) => {
                return Err(Failure::new(
                    Stage::Relay,
                    Class::Unavailable,
                    format!("{}: relay: the TURN relay worker ended", self.label),
                ))
            }
            Ok(Ok(result)) => result?,
        };
        Ok(Connected {
            local: None,
            peer: Some(addr),
            io: Box::new(stream),
        })
    }

    /// A TCP allocation that accepts one peer (RFC 6062, 4.2). A peer that
    /// connects to `relayed` is the peer of the returned stream. The
    /// allocation lives while the stream lives; `Client::accept` consumes
    /// the client, so one allocation accepts one peer.
    #[allow(dead_code)]
    pub(super) async fn accept(self: &Arc<Self>, deadline: Instant) -> Result<Accepted, Failure> {
        let worker = self
            .worker
            .get_or_init(RelayWorker::start)
            .listen(self.stream_settings(), self.label.clone());
        match tokio::time::timeout_at(deadline, worker).await {
            Err(_) => Err(Failure::new(
                Stage::Relay,
                Class::Timeout,
                format!(
                    "{}: relay: the TCP allocation for a peer did not come up in time",
                    self.label
                ),
            )),
            Ok(Err(_)) => Err(Failure::new(
                Stage::Relay,
                Class::Unavailable,
                format!("{}: relay: the TURN relay worker ended", self.label),
            )),
            Ok(Ok(Ok(mut accepted))) => {
                accepted._keep = Some(Arc::clone(self));
                Ok(accepted)
            }
            Ok(Ok(Err(failure))) => Err(failure),
        }
    }
}

impl Dial for TurnDialer {
    fn label(&self) -> String {
        self.label.clone()
    }

    /// A TCP stream through a TCP allocation (RFC 6062). `turn://` uses
    /// plain TURN over TCP for this path; the UDP path of the network stays.
    fn dial<'a>(
        &'a self,
        target: &'a Target,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(async move { self.connect_tcp(target, deadline).await })
    }

    /// One allocation for the socket. The allocation ends when the socket
    /// is dropped.
    fn datagrams<'a>(&'a self) -> BoxFuture<'a, Result<Arc<dyn Datagrams>, Failure>> {
        Box::pin(async move {
            let client = self.allocate().await?;
            Ok(Arc::new(TurnDatagrams::start(client, self.label.clone())) as Arc<dyn Datagrams>)
        })
    }
}

/// The source of the UDP socket for `turn://`: the way below, else the UDP
/// socket of this host.
fn source(below: Option<&Arc<dyn Dial>>) -> Arc<dyn UdpSource> {
    let resolver: Arc<dyn Resolve> = crate::resolve::plain();
    match below {
        Some(below) => Arc::new(BelowUdp {
            below: below.clone(),
            resolve: resolver,
        }),
        None => Arc::new(SystemUdp::with_resolver(resolver)),
    }
}

/// Read the lifetime and the rate limit of the settings, and check them
/// before any connection. A value that is not a number is a usage error
/// that names the variable, never a value.
pub(super) fn limits(
    get: &dyn Fn(&str) -> Option<String>,
) -> Result<(Limits, Option<u64>), Failure> {
    let mut limits = Limits::default();
    if let Some(text) = get(LIFETIME_VARIABLE).filter(|v| !v.is_empty()) {
        let secs: u64 = text.parse().map_err(|_| {
            usage(format!(
                "{LIFETIME_VARIABLE} is the lifetime in seconds, a number: {text:?} is not one"
            ))
        })?;
        if !(1..=3600).contains(&secs) {
            return Err(usage(format!("{LIFETIME_VARIABLE} is 1 to 3600 seconds")));
        }
        limits.lifetime = Duration::from_secs(secs);
        // A refresh comes before the end, at most at half of the lifetime.
        limits.refresh_before = Duration::from_secs(secs / 2).max(Duration::from_secs(1));
    }
    let rate_limit = match get(RATE_LIMIT_VARIABLE).filter(|v| !v.is_empty()) {
        Some(text) => Some(text.parse::<u64>().map_err(|_| {
            usage(format!(
                "{RATE_LIMIT_VARIABLE} is a number of bytes for each second"
            ))
        })?),
        None => None,
    };
    Ok((limits, rate_limit))
}

/// Bring the network up. One allocation is made and ended at once: it
/// proves the server, the account and the transport, and its fault names
/// the stage. The datagram sockets make their own allocations, and each
/// TCP stream makes a TCP allocation of its own.
pub(super) async fn up(
    spec: &Spec,
    account: Account,
    below: Option<&Arc<dyn Dial>>,
    limits: Limits,
    rate_limit: Option<u64>,
) -> Result<Link, Failure> {
    up_with_tls(
        spec,
        account,
        below,
        limits,
        rate_limit,
        crate::trust::context().clone(),
    )
    .await
}

/// The dialer of `spec` with the trust of the TLS connections given.
fn dialer(
    spec: &Spec,
    account: Account,
    below: Option<&Arc<dyn Dial>>,
    limits: Limits,
    rate_limit: Option<u64>,
    tls: Arc<pod_tls::Context>,
) -> Arc<TurnDialer> {
    let label = spec.label();
    let dial: Arc<dyn Dial> = below.cloned().unwrap_or_else(crate::dialers::process);
    Arc::new(TurnDialer {
        label,
        spec: spec.clone(),
        account,
        limits,
        rate_limit,
        dial,
        source: source(below),
        resolve: crate::resolve::plain(),
        worker: std::sync::OnceLock::new(),
        tls,
    })
}

/// [`up`] with the trust of the TLS connections given, so that a test can
/// trust the CA of its own stand (GitHub #485).
pub(super) async fn up_with_tls(
    spec: &Spec,
    account: Account,
    below: Option<&Arc<dyn Dial>>,
    limits: Limits,
    rate_limit: Option<u64>,
    tls: Arc<pod_tls::Context>,
) -> Result<Link, Failure> {
    let label = spec.label();
    let dialer = dialer(spec, account, below, limits, rate_limit, tls);
    let client = dialer.allocate().await?;
    // The status holds no user and no password (TURN-5).
    let status = serde_json::json!({
        "scheme": "turn",
        "spec": label,
        "transport": spec.transport(),
        "relay": "udp",
        "tcp_relay": "rfc6062",
        "lifetime_s": client.lifetime().as_secs(),
        "rate_limit": rate_limit,
        "mapped": client.mapped_address().map(|a| a.to_string()),
    });
    let _ = tokio::time::timeout(CLOSE_WAIT, client.close()).await;
    Ok(Link::new(Some(Way::destination(dialer))).with_status(status))
}

/// A TCP allocation that waits for one peer of RFC 6062 (GitHub #485):
/// the caller learns the relayed address, and the peer that connects to it
/// becomes the stream. `Client::accept` consumes the client, so one
/// allocation accepts one peer.
#[allow(dead_code)]
pub(super) async fn listen(
    spec: &Spec,
    account: Account,
    below: Option<&Arc<dyn Dial>>,
    limits: Limits,
    rate_limit: Option<u64>,
    tls: Arc<pod_tls::Context>,
) -> Result<Accepted, Failure> {
    let dialer = dialer(spec, account, below, limits, rate_limit, tls);
    dialer
        .accept(tokio::time::Instant::now() + std::time::Duration::from_secs(60))
        .await
}
