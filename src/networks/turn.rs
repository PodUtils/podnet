//! TURN as a network: `turn://HOST[:PORT]` and `turns://HOST[:PORT]` (T-321).
//!
//! The network gives the UDP of a host that has only a TURN server as a
//! relay. Each [`Dial::datagrams`] call makes one allocation on the server
//! (RFC 8656) with `pod-turn`, and the socket sends each datagram to a peer
//! in ChannelData. podnet has no TURN code of its own.
//!
//! * `turn://HOST[:PORT]`: TURN over UDP. Default port 3478.
//! * `turns://HOST[:PORT]`: TURN over TLS on TCP. Default port 5349. The
//!   TCP connection goes through the way out of the process (`--proxy`, the
//!   variables), or through the way below the network.
//!
//! The user, the password and the realm are settings, not parts of the spec
//! (a spec goes into `argv`, logs and JSON): `PODNET_TURN_USER`,
//! `PODNET_TURN_PASSWORD_FILE` or `PODNET_TURN_PASSWORD`, and
//! `PODNET_TURN_REALM`. They are read when the network comes up. No text of
//! this module holds a password.
//!
//! The network carries UDP only. A TCP flow needs the TCP relay of RFC 6062,
//! which `pod-turn` has not: [`Dial::dial`] refuses it by name.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use pod_core::Resolve;
use pod_turn::{Client, Credentials, Limits, Opened, Settings, SystemUdp, UdpSource};
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

/// The dialer of the network: one allocation for each datagram socket.
pub(super) struct TurnDialer {
    label: String,
    spec: Spec,
    account: Account,
    limits: Limits,
    /// The way for the TCP connection of `turns://`.
    dial: Arc<dyn Dial>,
    /// The socket of `turn://`.
    source: Arc<dyn UdpSource>,
}

impl TurnDialer {
    fn settings(&self) -> Settings {
        let credentials =
            Credentials::new(self.account.user.clone(), self.account.password.clone());
        let mut settings = if self.spec.tls {
            Settings::tls(
                self.spec.server.clone(),
                credentials,
                self.dial.clone(),
                crate::trust::context().clone(),
            )
        } else {
            Settings::udp(self.spec.server.clone(), credentials, self.source.clone())
        };
        settings.realm = self.account.realm.clone();
        settings.limits = self.limits.clone();
        settings.software = Some(format!("podnet/{}", env!("CARGO_PKG_VERSION")));
        settings
    }

    /// One allocation on the server. A fault names the stage `allocate`.
    async fn allocate(&self) -> Result<Client, Failure> {
        Client::allocate(self.settings())
            .await
            .map_err(|e| named(TurnStage::Allocate, &self.label, &e))
    }
}

impl Dial for TurnDialer {
    fn label(&self) -> String {
        self.label.clone()
    }

    /// TCP needs the TCP relay of RFC 6062: `pod-turn` has no such relay.
    fn dial<'a>(
        &'a self,
        target: &'a Target,
        _: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(async move {
            Err(Failure::new(
                Stage::Connect,
                Class::Unsupported,
                format!(
                    "{}: TURN carries UDP only, and {target} is a TCP target; the TCP relay of RFC 6062 is not in podnet yet",
                    self.label
                ),
            ))
        })
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

/// Bring the network up. One allocation is made and ended at once: it
/// proves the server, the account and the transport, and its fault names
/// the stage. The datagram sockets make their own allocations.
pub(super) async fn up(
    spec: &Spec,
    account: Account,
    below: Option<&Arc<dyn Dial>>,
    limits: Limits,
) -> Result<Link, Failure> {
    let label = spec.label();
    let dialer = Arc::new(TurnDialer {
        label: label.clone(),
        spec: spec.clone(),
        account,
        limits,
        dial: below.cloned().unwrap_or_else(crate::dialers::process),
        source: source(below),
    });
    let client = dialer.allocate().await?;
    // The status holds no user and no password.
    let status = serde_json::json!({
        "scheme": "turn",
        "spec": label,
        "transport": spec.transport(),
        "lifetime_s": client.lifetime().as_secs(),
        "mapped": client.mapped_address().map(|a| a.to_string()),
    });
    let _ = tokio::time::timeout(CLOSE_WAIT, client.close()).await;
    Ok(Link::new(Some(Way::destination(dialer))).with_status(status))
}
