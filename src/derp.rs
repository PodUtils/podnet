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

/// derper's probe of each server through the way out of podnet: whether it
/// answered within `pod_derp::servers::PROBE_LIMIT`.
pub fn probe() -> impl Fn(&Server) -> Probing {
    let client =
        pod_proxy::Client::new(crate::trust::context().clone(), crate::egress::egress(None))
            .with_user_agent(names().user_agent);
    let trust = crate::trust::ws_trust();
    move |server: &Server| {
        Box::pin(servers::answers(
            server.clone(),
            client.clone(),
            trust.clone(),
        ))
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
/// order, and the sink of the lines.
#[derive(Clone)]
pub struct Road {
    pub dial: Dial,
    pub servers: Vec<Server>,
    pub events: Arc<dyn Events>,
}

impl Road {
    /// The road through the way out of podnet, with the lines in the log.
    pub fn out(servers: Vec<Server>) -> Road {
        Road {
            dial: tls_dial(),
            servers,
            events: log_events(),
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
/// `probe` answers and whose DERP handshake completes. When none does, exit
/// 69 with the reason of each server.
pub async fn online<P, F>(me: &Identity, road: &Road, probe: P) -> Result<Online>
where
    P: Fn(&Server) -> F,
    F: Future<Output = Result<(), String>>,
{
    let secret = Arc::new(Secret::of_node(me));
    let names = names();
    let (home, missed) = far::home(
        &secret,
        &road.dial,
        &road.events,
        &names,
        &road.servers,
        probe,
    )
    .await;
    let Some((server, conn)) = home else {
        let why = if missed.is_empty() {
            "the list of servers is empty".to_string()
        } else {
            missed.join("; ")
        };
        return Err(exit::fault(
            Fault::Unavailable,
            anyhow!("no DERP server took the node: {why}"),
        ));
    };
    let ticket = Ticket {
        node: me.public(),
        derp: secret.public(),
        server,
    };
    Ok(Online {
        ticket,
        secret,
        conn,
    })
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
    let unavailable = |e: anyhow::Error| exit::fault(Fault::Unavailable, e);
    let dialer = Dialer::new(
        ticket,
        &road.servers,
        road.dial.clone(),
        road.events.clone(),
        names(),
    )
    .map_err(|e| unavailable(anyhow!("{e}")))?;
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
    let mut layer = Box::pin(pod_derp::carry(&dialer, records, settings(), say));
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
    dialer.close().await;
    let server = &ticket.server;
    match carried {
        // 77 for a key that an end refuses, 78 for no shared suite.
        Err(e) => Err(relay_reverse::e2e_fault(e)
            .context(format!("the sealed session to the node at {server}"))),
        Ok(Err(failure)) => Err(unavailable(anyhow::Error::new(failure))),
        Ok(Ok(Outcome::Layer(ended))) => match &ended.end {
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
        Ok(Ok(Outcome::Plain(None))) => Ok(()),
        Ok(Ok(Outcome::Plain(Some(e)))) => Err(unavailable(anyhow!("the session over DERP: {e}"))),
    }
}
