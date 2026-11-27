//! The DERP road (T-128): two podnet hosts meet at a DERP server, a server
//! of Tailscale's fleets (no account) or a derper of the user's own.
//!
//! `pod-derp` of PodUtils/podcrates does the work: DERP over a WebSocket at
//! `wss://HOST/derp` through the way out of podnet, the adapter that sends
//! again what a server drops, the links of a client and of a node, the
//! node's home and its `derp:` ticket. This module is the glue:
//!
//! * [`online`] finds the node's home: the first server of the list whose
//!   probe answers and whose DERP handshake completes. The node's key on the
//!   server comes from the key of the install, so the ticket stays the same
//!   on the same home.
//! * [`serve`] serves each link that a client opens at the home: the far end
//!   of the resumable layer (`pod-session`), and above it the node's end of
//!   the end-to-end channel (`pod-e2e`). The node dials the target only for
//!   a client whose key is in its allow list. When the home goes away, the
//!   node moves to the next server of the list and says its new ticket.
//! * [`connect`] carries a stream of the caller to the node of a ticket: the
//!   client of the layer over the links of `pod-derp`, a new link after each
//!   loss, and the operator's end of the channel above it. The node must
//!   prove the key that the ticket names.
//!
//! The server sees the keys, the sizes and the times; the bytes are sealed
//! end to end. A ticket is an address, not a credential: the allow list of
//! the node says who comes in.
//!
//! Exit codes: a usage error 64; no server, no node or no target 69; a key
//! that an end refuses 77; no shared suite 78 ([`crate::relay_reverse::e2e_fault`]).

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use pod_core::{Event, Events, Level};
use pod_derp::dial::Dial;
use pod_derp::far::{self, Roam};
use pod_derp::{servers, Conn, Dialer, Names, Node, Secret};
pub use pod_derp::{Server, Ticket};
use pod_e2e::ends;
use pod_identity::{Identity, PublicKey};
use pod_session::client::Outcome;
use pod_session::keep::Keeper;
use pod_session::resume::Note;
use pod_session::{budget, End, Settings};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::exit::{self, Fault};
use crate::relay_reverse::{self, Target};

/// The name of podnet in DERP's client info and in the words of the crate.
pub const APP: &str = "podnet";
/// The flag that keeps tailcat's four servers alone (`--main-fleet no`).
pub const MAIN_FLAG: &str = "--main-fleet";
/// The replay buffers that a node keeps for all its sessions, at most.
pub const BUDGET: usize = 64 << 20;
/// The bytes between the layer and the channel, in each direction.
const PIPE: usize = 256 * 1024;
/// How long the end of the channel is awaited after the end of the layer.
const CHANNEL_WAIT: Duration = Duration::from_secs(5);

/// Whether a key of the other end is allowed.
pub type Allows = dyn Fn(&PublicKey) -> bool + Send + Sync;

/// The names of podnet on the wire and in the words of `pod-derp`.
pub fn names() -> Names {
    Names {
        app: APP.into(),
        user_agent: format!("podnet/{}", env!("CARGO_PKG_VERSION")),
        main: MAIN_FLAG.into(),
    }
}

/// The way to each server: TLS through the way out of podnet (`--proxy`,
/// NO_PROXY, `--resolve`, `--ca-file`).
pub fn tls_dial() -> Dial {
    pod_derp::dial::tls(crate::ws::way(), crate::trust::ws_trust())
}

/// A probe of one server in progress: whether it answered.
pub type Probing = std::pin::Pin<Box<dyn Future<Output = Result<(), String>> + Send>>;

/// A probe of one server in progress: how long the answer took, or why
/// none came, with the cut of the failure.
pub type ProbingKept =
    std::pin::Pin<Box<dyn Future<Output = Result<Duration, servers::Missed>> + Send>>;

/// The client of derper's probe: the way out of podnet (the proxy, the
/// resolver, the roots), with the user agent of podnet.
fn probe_client() -> (pod_proxy::Client, pod_tls::Trust) {
    let client =
        pod_proxy::Client::new(crate::trust::context().clone(), crate::egress::egress(None))
            .with_user_agent(names().user_agent);
    (client, crate::trust::ws_trust())
}

/// derper's probe of each server through the way out of podnet: whether it
/// answered within `pod_derp::servers::PROBE_LIMIT`.
pub fn probe() -> impl Fn(&Server) -> Probing {
    let (client, trust) = probe_client();
    move |server: &Server| {
        Box::pin(servers::answers(
            server.clone(),
            client.clone(),
            trust.clone(),
        ))
    }
}

/// derper's probe of each server through the way out of podnet, with the
/// round trip of the answer or the reason and the cut of a failure (the row
/// `derp` of `podnet doctor`).
pub fn probe_kept() -> impl Fn(&Server) -> ProbingKept + Send + Sync + 'static {
    let (client, trust) = probe_client();
    move |server: &Server| {
        let (server, client, trust) = (server.clone(), client.clone(), trust.clone());
        Box::pin(async move { servers::probe_kept(&server, &client, &trust).await })
    }
}

/// The servers of the road: the list of `--server` (`HOST[:PORT][,...]`),
/// else the fleet of `pod-services` (tailcat's four, then the main fleet
/// unless `main` is no). A bad value exits 64.
pub fn servers_of(named: Option<&str>, main: Option<&str>) -> Result<Vec<Server>> {
    let (list, _) = servers::chosen(named, main, &names())
        .map_err(|why| exit::fault(Fault::Usage, anyhow!("{why}")))?;
    Ok(list)
}

/// The ticket in `text` (`derp:...`). A text that is no ticket exits 64.
pub fn ticket_of(text: &str) -> Result<Ticket> {
    Ticket::parse(text).map_err(|why| exit::fault(Fault::Usage, anyhow!("{why}")))
}

/// The settings of the layer at each end: the features of the layer over
/// DERP, with no window of the relay's contract. A client asks for the end
/// of one direction, so the answer of the target still comes after the end
/// of its input.
pub fn settings() -> Settings {
    Settings {
        features: pod_derp::FEATURES,
        half_close: true,
        ..Settings::default()
    }
    .off_the_pair()
}

/// The lines of the road go to the log: info at `-v`, warn always.
pub fn log_events() -> Arc<dyn Events> {
    Arc::new(|event: Event| match event.level {
        Level::Info => tracing::info!(kind = event.kind, "{event}"),
        Level::Warn => tracing::warn!(kind = event.kind, "{event}"),
    })
}

/// The road of one end: the way to each server, the servers in their
/// order, the sink of the lines, and whether a node probes each server
/// ([`probe`]) before its handshake.
#[derive(Clone)]
pub struct Road {
    pub dial: Dial,
    pub servers: Vec<Server>,
    pub events: Arc<dyn Events>,
    pub probed: bool,
}

impl Road {
    /// The road through the way out of podnet, with the lines in the log.
    pub fn out(servers: Vec<Server>) -> Road {
        Road {
            dial: tls_dial(),
            servers,
            events: log_events(),
            probed: true,
        }
    }
}

/// The key of one end of the channel and the keys of the other ends that it
/// trusts.
#[derive(Clone)]
pub struct Keys {
    pub me: Arc<Identity>,
    pub allows: Arc<Allows>,
}

impl Keys {
    /// The key `me`, which lets in the keys that `allows` admits.
    pub fn new(me: &Identity, allows: impl Fn(&PublicKey) -> bool + Send + Sync + 'static) -> Keys {
        Keys {
            me: Arc::new(Identity::from_seed(&me.seed())),
            allows: Arc::new(allows),
        }
    }
}

/// A node that is online at its home.
pub struct Online {
    /// The node's ticket: its key, its key on the server and its home.
    pub ticket: Ticket,
    secret: Arc<Secret>,
    conn: Conn,
}

/// Bring the node of the key `me` online: the first server of `road` whose
/// probe answers (when the road probes) and whose DERP handshake completes.
/// When none does, exit 69 with each cause grouped once. A proxy-hop failure
/// stops the search after the first server.
pub async fn online(me: &Identity, road: &Road) -> Result<Online> {
    let check: Box<dyn Fn(&Server) -> Probing + Send + Sync> = if road.probed {
        Box::new(probe())
    } else {
        Box::new(|_: &Server| -> Probing { Box::pin(async { Ok(()) }) })
    };
    online_with(me, road, check).await
}

/// Remove a leading server label from a probe failure string.
fn strip_server_prefix(server: &Server, text: &str) -> String {
    let label = server.to_string();
    if let Some(rest) = text.strip_prefix(&label) {
        if let Some(rest) = rest.strip_prefix(": ") {
            return rest.to_string();
        }
        if let Some(rest) = rest.strip_prefix(' ') {
            return rest.to_string();
        }
    }
    text.to_string()
}

/// True when `cause` shows that the proxy hop itself failed, not the target.
fn is_proxy_hop_text(cause: &str) -> bool {
    cause.contains("could not reach the proxy")
        || cause.contains("could not resolve the proxy")
        || cause.contains("the proxy setting is not usable:")
        || (cause.contains("the proxy ") && cause.contains(" did not answer as its protocol says:"))
        || (cause.contains("the proxy command") && cause.contains(" exited "))
}

/// True when a dial or handshake failure broke at the proxy hop itself.
fn is_proxy_hop_failure(failure: &pod_core::Failure) -> bool {
    if let Some(dial) = std::error::Error::source(failure)
        .and_then(|err| err.downcast_ref::<pod_proxy::DialError>())
    {
        return matches!(
            dial,
            pod_proxy::DialError::BadProxy(_)
                | pod_proxy::DialError::ProxyUnreachable { .. }
                | pod_proxy::DialError::ProxyProtocol { .. }
                | pod_proxy::DialError::ProxyExited { .. }
                | pod_proxy::DialError::Timeout {
                    stage: pod_core::Stage::Proxy,
                    ..
                }
        );
    }
    is_proxy_hop_text(&failure.to_string())
}

/// Format grouped server failures and any trailing search notes.
fn summarize_missed(missed: &[(Server, String)], notes: &[String]) -> String {
    let mut parts = Vec::new();
    if !missed.is_empty() {
        parts.push(pod_proxy::group_texts(missed, "servers"));
    }
    parts.extend(notes.iter().cloned());
    if parts.is_empty() {
        "the list of servers is empty".to_string()
    } else {
        parts.join("; ")
    }
}

async fn online_with<P, F>(me: &Identity, road: &Road, check: P) -> Result<Online>
where
    P: Fn(&Server) -> F,
    F: Future<Output = Result<(), String>>,
{
    let secret = Arc::new(Secret::of_node(me));
    let names = names();
    let mut missed: Vec<(Server, String)> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    let mut main_since: Option<tokio::time::Instant> = None;

    for (at, server) in road.servers.iter().enumerate() {
        if server.fleet_name() == Some(servers::MAIN_FLEET) {
            let since = *main_since.get_or_insert_with(tokio::time::Instant::now);
            if since.elapsed() > far::MAIN_SEARCH {
                let left = road.servers.len() - at;
                notes.push(format!(
                    "{left} servers of the main fleet not tried within {} s",
                    far::MAIN_SEARCH.as_secs()
                ));
                break;
            }
        }
        if road.probed {
            if let Err(why) = check(server).await {
                let cause = strip_server_prefix(server, &why);
                if is_proxy_hop_text(&cause) {
                    return Err(exit::fault(
                        Fault::Unavailable,
                        anyhow!("no DERP server took the node: {cause}"),
                    ));
                }
                missed.push((server.clone(), cause));
                continue;
            }
        }
        let node = Node {
            secret: secret.clone(),
            server: server.clone(),
            dial: road.dial.clone(),
            events: road.events.clone(),
            names: names.clone(),
        };
        match far::connect(&node).await {
            Ok(conn) => {
                let ticket = Ticket {
                    node: me.public(),
                    derp: secret.public(),
                    server: server.clone(),
                };
                return Ok(Online {
                    ticket,
                    secret,
                    conn,
                });
            }
            Err(why) => {
                if is_proxy_hop_failure(&why) {
                    return Err(exit::fault(
                        Fault::Unavailable,
                        anyhow!("no DERP server took the node: {why}"),
                    ));
                }
                let cause = if road.probed {
                    format!("answered its probe, then its handshake failed: {why}")
                } else {
                    why.to_string()
                };
                missed.push((server.clone(), cause));
            }
        }
    }

    let why = summarize_missed(&missed, &notes);
    Err(exit::fault(
        Fault::Unavailable,
        anyhow!("no DERP server took the node: {why}"),
    ))
}

/// The node's end of the channel: a client whose key `keys` does not allow
/// gets a refusal, and the target is not dialed.
fn e2e_node(keys: &Keys) -> ends::Node {
    let events: Arc<dyn Events> = log_events();
    let mut node = ends::Node::open_to_all(keys.me.clone(), events);
    let allows = keys.allows.clone();
    node.admit = Arc::new(move |key: &PublicKey, _: &pod_e2e::Ask| {
        if allows(key) {
            Ok(())
        } else {
            Err(format!(
                "the key {} is not in the allow list",
                key.display_fingerprint()
            ))
        }
    });
    node
}

/// Serve each session of the node until the caller stops it: one
/// connection to `target` for each client that `keys` lets in. `moved`
/// hears the new ticket after a move of the home.
pub async fn serve<M>(online: Online, road: Road, target: Target, keys: Keys, moved: M)
where
    M: Fn(&Ticket) + Send + Sync + 'static,
{
    let node_end = Arc::new(e2e_node(&keys));
    let handler = relay_reverse::local_handler(&target);
    let open = move || {
        let (node_end, handler) = (node_end.clone(), handler.clone());
        async move {
            let (ours, theirs) = tokio::io::duplex(PIPE);
            tokio::spawn(async move {
                let dial = || async move {
                    let target = &handler.target;
                    pod_relay::reverse::local::connect(target, &handler.dialer, handler.timeout)
                        .await
                        .map_err(|why| format!("{target}: {why}"))
                };
                ends::serve_node(theirs, &node_end, dial).await;
            });
            Ok::<_, String>(ours)
        }
    };
    serve_with(online, road, open, moved).await
}

/// Serve each session of the node until the caller stops it: the bytes of
/// each new session go to the stream that `open` gives. `moved` hears the
/// new ticket after a move of the home.
pub async fn serve_with<A, O, F, M>(online: Online, road: Road, open: O, moved: M)
where
    A: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    O: Fn() -> F + Clone + Send + Sync + 'static,
    F: Future<Output = Result<A, String>> + Send + 'static,
    M: Fn(&Ticket) + Send + Sync + 'static,
{
    let Online {
        ticket,
        secret,
        conn,
    } = online;
    let node = Node {
        secret,
        server: ticket.server.clone(),
        dial: road.dial.clone(),
        events: road.events.clone(),
        names: names(),
    };
    let roam = Roam {
        servers: road.servers.clone(),
        moved: Arc::new(move |server: &Server| {
            moved(&Ticket {
                server: server.clone(),
                ..ticket.clone()
            })
        }),
    };
    let settings = settings();
    let tell = budget::Tell::new(road.events.clone(), budget::CHANGE);
    let keeper = Arc::new(Keeper::new(settings.resume_deadline).with_tell(tell));
    far::serve_roaming(node, roam, Some(conn), settings, BUDGET, open, keeper).await
}

/// Say a step of the session: a loss and a resume always, a retry with `-v`.
fn say(note: Note) {
    match note {
        Note::Lost { why } => {
            tracing::info!("the session lost its DERP link ({why}); it waits for a new link")
        }
        Note::Resumed { resent, after } => tracing::info!(
            resent,
            after_ms = after.as_millis() as u64,
            "the session resumed on a new DERP link"
        ),
        Note::Retry { why, wait } => tracing::debug!(
            wait_ms = wait.as_millis() as u64,
            "a new DERP link did not open: {why}"
        ),
        other => tracing::debug!("{other:?}"),
    }
}

/// Carry `app` to the node of `ticket` until the session ends. The node
/// must prove the key of the ticket, and let in the key of `me`.
pub async fn connect<A>(ticket: &Ticket, road: &Road, app: A, me: &Identity) -> Result<()>
where
    A: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let want = ticket.node;
    let check = move |node: &PublicKey| {
        if *node == want {
            Ok(())
        } else {
            Err(format!(
                "the node proved the key {}, not the key {} of the ticket",
                node.display_fingerprint(),
                want.display_fingerprint()
            ))
        }
    };
    let me = Arc::new(Identity::from_seed(&me.seed()));
    let (records, channel) = ends::operator(app, me, check);
    let mut channel = Box::pin(channel);
    let mut layer = Box::pin(carry(ticket, road, records));
    let carried = tokio::select! {
        ended = &mut channel => match ended {
            Err(e) => Err(e),
            // Both directions of the channel ended: the layer ends next.
            Ok(_) => Ok(layer.await),
        },
        ended = &mut layer => {
            // A refusal of the node can come just before the end of the
            // layer: the channel says why.
            match tokio::time::timeout(CHANNEL_WAIT, channel).await {
                Ok(Err(e)) => Err(e),
                _ => Ok(ended),
            }
        }
    };
    match carried {
        // 77 for a key that an end refuses, 78 for no shared suite.
        Err(e) => Err(relay_reverse::e2e_fault(e).context(format!(
            "the sealed session to the node at {}",
            ticket.server
        ))),
        Ok(ended) => ended,
    }
}

/// Carry `app` in a session of the layer to the node of `ticket`, with no
/// channel above the layer (the caller seals its bytes), until the session
/// ends. No server or no node exits 69.
pub async fn carry<A>(ticket: &Ticket, road: &Road, app: A) -> Result<()>
where
    A: AsyncRead + AsyncWrite + Unpin + Send,
{
    let unavailable = |e: anyhow::Error| exit::fault(Fault::Unavailable, e);
    let dialer = Dialer::new(
        ticket,
        &road.servers,
        road.dial.clone(),
        road.events.clone(),
        names(),
    )
    .map_err(|e| unavailable(anyhow!("{e}")))?;
    let carried = pod_derp::carry(&dialer, app, settings(), say).await;
    dialer.close().await;
    match carried {
        Err(failure) => Err(unavailable(anyhow::Error::new(failure))),
        Ok(Outcome::Layer(ended)) => match &ended.end {
            End::LocalEnd | End::Closed(_) => Ok(()),
            end => match crate::session::expired(end, settings().resume_deadline) {
                Some(e) => Err(unavailable(
                    anyhow::Error::new(e).context("the session over DERP ended"),
                )),
                None => Err(unavailable(anyhow!(
                    "the session over DERP ended: {}",
                    end.loss()
                ))),
            },
        },
        Ok(Outcome::Plain(None)) => Ok(()),
        Ok(Outcome::Plain(Some(e))) => Err(unavailable(anyhow!("the session over DERP: {e}"))),
    }
}

#[cfg(test)]
#[tokio::test]
async fn the_error_line_names_one_cause_once() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use pod_core::{Class, Failure, Stage};
    use pod_proxy::DialError;

    let make_servers = |n: usize| -> Vec<Server> {
        (0..n)
            .map(|i| Server::parse(&format!("s{i}.example")).unwrap())
            .collect()
    };
    let me = Identity::from_seed(&[7u8; 32]);

    struct Case {
        label: &'static str,
        count: usize,
        make_err: fn(usize) -> Failure,
        expected_attempts: usize,
        want_once: &'static [&'static str],
        want_substrings: &'static [&'static str],
    }

    let cases = [
        Case {
            label: "100 servers through an unreachable proxy stop after the first failure",
            count: 100,
            make_err: |_| {
                Failure::from(DialError::ProxyUnreachable {
                    proxy: "127.0.0.1:1".into(),
                    detail: "Connection refused".into(),
                })
            },
            expected_attempts: 1,
            want_once: &["could not reach the proxy 127.0.0.1:1: Connection refused"],
            want_substrings: &["no DERP server took the node:"],
        },
        Case {
            label: "100 servers with one non-proxy cause group under one cause",
            count: 100,
            make_err: |_| Failure::new(Stage::Connect, Class::Unavailable, "connection refused"),
            expected_attempts: 100,
            want_once: &["connection refused"],
            want_substrings: &["100 servers (s0.example, s1.example, ...)"],
        },
        Case {
            label: "mixed causes group each cause once",
            count: 5,
            make_err: |i| {
                if i < 3 {
                    Failure::new(Stage::Connect, Class::Unavailable, "connection refused")
                } else {
                    Failure::new(Stage::Tls, Class::Timeout, "TLS handshake timed out")
                }
            },
            expected_attempts: 5,
            want_once: &["connection refused", "TLS handshake timed out"],
            want_substrings: &[
                "3 servers (s0.example, s1.example, ...)",
                "2 servers (s3.example, s4.example)",
            ],
        },
        Case {
            label: "a single server failure names the server once",
            count: 1,
            make_err: |_| Failure::new(Stage::Connect, Class::Unavailable, "connection refused"),
            expected_attempts: 1,
            want_once: &["connection refused"],
            want_substrings: &["s0.example"],
        },
    ];

    for case in cases {
        let attempts = Arc::new(AtomicUsize::new(0));
        let counter = attempts.clone();
        let make_err = case.make_err;
        let dial: Dial = Arc::new(move |_server: &Server| {
            let idx = counter.fetch_add(1, Ordering::SeqCst);
            let err = make_err(idx);
            Box::pin(async move { Err(err) })
        });
        let road = Road {
            dial,
            servers: make_servers(case.count),
            events: Arc::new(|_: Event| {}),
            probed: false,
        };
        let err = match online(&me, &road).await {
            Ok(_) => panic!("{}: expected error", case.label),
            Err(err) => err,
        };
        assert_eq!(exit::code_of(&err), 69, "{}: exit code", case.label);
        let line = err.to_string();
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            case.expected_attempts,
            "{}: attempts (line: {line})",
            case.label
        );
        assert!(
            line.len() < 400,
            "{}: line too long ({} chars): {line}",
            case.label,
            line.len()
        );
        for needle in case.want_once {
            assert_eq!(
                line.matches(needle).count(),
                1,
                "{}: expected {needle:?} once in {line:?}",
                case.label
            );
        }
        for sub in case.want_substrings {
            assert!(
                line.contains(sub),
                "{}: expected {sub:?} in {line:?}",
                case.label
            );
        }
    }

    // Probed path: stop after one proxy-hop failure and group identical probe timeouts.
    type ProbeCase = (
        &'static str,
        fn(&Server) -> String,
        usize,
        &'static str,
        &'static str,
    );
    let probe_cases: &[ProbeCase] = &[
        (
            "probed: 100 servers stop after first unreachable proxy",
            |s| format!("{s}: could not reach the proxy 127.0.0.1:1: Connection refused"),
            1,
            "could not reach the proxy 127.0.0.1:1: Connection refused",
            "no DERP server took the node: could not reach the proxy 127.0.0.1:1: Connection refused",
        ),
        (
            "probed: 100 servers with identical timeout group once",
            |s| format!("{s} did not answer /generate_204 within 5 s"),
            100,
            "did not answer /generate_204 within 5 s",
            "100 servers (s0.example, s1.example, ...): did not answer /generate_204 within 5 s",
        ),
    ];
    for &(label, make_probe_err, expected_probes, once_needle, want_sub) in probe_cases {
        let probes = Arc::new(AtomicUsize::new(0));
        let counter = probes.clone();
        let dial: Dial = Arc::new(|_server: &Server| {
            Box::pin(async { Err(Failure::new(Stage::Connect, Class::Unavailable, "unused")) })
        });
        let road = Road {
            dial,
            servers: make_servers(100),
            events: Arc::new(|_: Event| {}),
            probed: true,
        };
        let err = match online_with(&me, &road, move |server: &Server| {
            counter.fetch_add(1, Ordering::SeqCst);
            let msg = make_probe_err(server);
            async move { Err(msg) }
        })
        .await
        {
            Ok(_) => panic!("{label}: expected error"),
            Err(err) => err,
        };
        assert_eq!(exit::code_of(&err), 69, "{label}: exit code");
        let line = err.to_string();
        assert_eq!(
            probes.load(Ordering::SeqCst),
            expected_probes,
            "{label}: probes (line: {line})"
        );
        assert_eq!(
            line.matches(once_needle).count(),
            1,
            "{label}: expected {once_needle:?} once in {line:?}"
        );
        assert!(
            line.contains(want_sub),
            "{label}: expected {want_sub:?} in {line:?}"
        );
    }
}
