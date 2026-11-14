//! The resumable layer on the reverse road (T-087): a session of
//! `relay connect --resume` outlives the limits of the relay (64 MiB and
//! 12 h for each relay session) and the loss of a leg.
//!
//! `pod-session` of PodUtils/podcrates does the work. This module is the
//! glue:
//!
//! * The node ([`run_node_resumable`]) runs the far end of the layer for
//!   each session: `pod_relay::reverse::Layered` over the handler of the
//!   target ([`layered`]). A resume goes on with the connection to the
//!   target that the session kept.
//! * The operator ([`connect_resumable`]) runs the client of the layer. Its
//!   first leg is one operator socket to `/v1/connect/NAME`. After the loss
//!   of a leg, and before a leg meets the limits of the relay (48 MiB, or
//!   11 h), it opens `/v1/connect/NAME` again, and the layer moves the
//!   session to the new leg.
//! * A node with no layer greets with no `GREETING`: the session then runs
//!   in the plain mode on its first leg, with a warning.

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use pod_relay::reverse::{self as road, operator, Layered, OperatorConfig, OperatorLimits};
use pod_session::client::{self, ClientError, Found, Outcome};
use pod_session::resume::{self, Next, Note};
use pod_session::{Ask, End, Ended, GiveUp, OsEntropy, Settings};
use pod_ws::ConnectError;
use tokio::io::{AsyncRead, AsyncWrite, DuplexStream};
use tokio::task::JoinHandle;

use super::seal::{carry_sealed, Seal};
use super::{
    connect_error, left_words, local_handler, outcome_error, run_node_with, FlushEach, Handler,
    Pair, Target, EOF_IDLE, TIMEOUT,
};
use crate::exit::{fault, Fault};
use crate::ws_relay::Relay;

/// The bytes between the layer and a leg, in each direction.
const PIPE: usize = 256 * 1024;
/// How long a later leg waits, after the end of its input, for the node to
/// end the relay session (podssh: the far end ends it within 30 s).
const NODE_ENDS: Duration = Duration::from_secs(35);
/// How long the end of a lost leg is awaited, for the log.
const LEG_WAIT: Duration = Duration::from_secs(5);

/// The limits of one leg after which the operator moves the session to a
/// new leg, and the end of the pair.
#[derive(Debug, Clone, Copy)]
pub struct Resume {
    /// The bytes of one leg, both ways: under the cap of the relay
    /// (64 MiB). Default 48 MiB.
    pub move_bytes: u64,
    /// The age of one leg: under the limit of the relay (12 h). Default
    /// 11 h.
    pub move_age: Duration,
    /// The end of the pair (milliseconds since the epoch), when it is
    /// known: the operator warns before it.
    pub expires: Option<u64>,
}

impl Default for Resume {
    fn default() -> Self {
        Resume {
            move_bytes: resume::MOVE_BYTES,
            move_age: resume::MOVE_AGE,
            expires: None,
        }
    }
}

impl Resume {
    /// The settings of the client of the layer. The client asks for the end
    /// of one direction (`end.v1`): after the end of its input, the answer
    /// of the target still comes.
    pub fn settings(&self) -> Settings {
        Settings {
            move_bytes: self.move_bytes,
            move_age: self.move_age,
            half_close: true,
            ..Settings::default()
        }
        .on_the_pair()
    }
}

/// The settings of the far end of the layer on a node.
pub fn node_settings() -> Settings {
    Settings::default().on_the_pair()
}

/// A handler whose sessions run the far end of the layer over the sessions
/// of `inner`, with the replay budget of a node (64 MiB).
pub fn layered<H: Handler>(inner: H) -> Layered<H> {
    Layered::new(inner, node_settings(), road::layered::NODE_BUDGET)
}

/// [`super::run_node`] with the far end of the layer for each session
/// (`relay node --resume`).
pub async fn run_node_resumable(
    follow: Option<&std::path::Path>,
    pair: Pair,
    target: &Target,
) -> Result<()> {
    let handler = Arc::new(layered(local_handler(target)));
    run_node_with(follow, pair, handler, &target.to_string()).await
}

/// [`run_node_resumable`] with the end-to-end channel above the layer
/// (`relay node --resume --seal`): the node dials the target only for an
/// operator whose key is in the allow list of `seal`.
pub async fn run_node_sealed(
    follow: Option<&std::path::Path>,
    pair: Pair,
    target: &Target,
    seal: &Seal,
) -> Result<()> {
    tracing::info!(
        key = %seal.public().display_fingerprint(),
        "the sessions of the node are sealed end to end"
    );
    let handler = Arc::new(super::seal::layered_sealed(local_handler(target), seal));
    run_node_with(follow, pair, handler, &target.to_string()).await
}

/// Why [`carry`] failed.
#[derive(Debug)]
pub enum CarryError {
    /// The handshake of the layer on the first leg failed.
    Start(ClientError),
    /// The session ended with this end, after its start.
    Ended(String),
    /// `next` stopped the session with these words.
    Stopped(String),
    /// The end-to-end channel above the layer failed (T-087, `--seal`).
    Channel(pod_e2e::Error),
}

impl std::fmt::Display for CarryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CarryError::Start(e) => write!(f, "{e}"),
            CarryError::Ended(why) | CarryError::Stopped(why) => f.write_str(why),
            CarryError::Channel(e) => write!(f, "the end-to-end channel: {e}"),
        }
    }
}

impl std::error::Error for CarryError {}

/// Carry `app` in a session of the layer. `first` is the first leg. After
/// the loss of a leg, `next(false)` gives a new leg; before a leg meets the
/// limits of `settings`, `next(true)` gives the leg of the move. A far end
/// with no layer gets the bytes as they are, on the first leg alone.
pub async fn carry<L, A, N, F>(
    first: L,
    app: A,
    settings: Settings,
    mut next: N,
) -> Result<(), CarryError>
where
    L: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    A: AsyncRead + AsyncWrite + Unpin + Send,
    N: FnMut(bool) -> F,
    F: Future<Output = Next<L>>,
{
    let client = client::start(first, Ask::New, settings, &mut OsEntropy)
        .await
        .map_err(CarryError::Start)?;
    match client.found() {
        found @ Found::Plain { .. } => tracing::warn!(
            "{found}: the session runs in the plain mode, and it ends at the limits of the relay (the node runs with no --resume)"
        ),
        found => tracing::debug!("{found}"),
    }
    let connect = move |ended: &Ended| next(matches!(ended.end, End::Moving));
    // On the heap: the future of the layer is large for the stack of a
    // debug build.
    let outcome = Box::pin(resume::run(client, app, connect, say, &mut OsEntropy)).await;
    match outcome {
        Outcome::Plain(None) => Ok(()),
        Outcome::Plain(Some(e)) => Err(CarryError::Ended(format!("the session failed: {e}"))),
        Outcome::Layer(ended) => match ended.end {
            End::LocalEnd | End::Closed(_) => Ok(()),
            End::GaveUp(gave_up) if gave_up.cause == GiveUp::Stopped => {
                Err(CarryError::Stopped(gave_up.reason))
            }
            End::GaveUp(gave_up) => Err(CarryError::Ended(format!(
                "no new leg carried the session on: {gave_up}"
            ))),
            end => Err(CarryError::Ended(format!(
                "the session ended: {}",
                end.loss()
            ))),
        },
    }
}

/// Say a step of the session: a loss and a resume always, a move and an
/// attempt with `-v`.
fn say(note: Note) {
    match note {
        Note::Moved { bytes, age, resent } => tracing::debug!(
            bytes,
            age_s = age.as_secs(),
            resent,
            "the session moved to a new leg before the limits of the relay"
        ),
        Note::Lost { why } => {
            tracing::info!("the session lost its leg ({why}); it waits for a new leg")
        }
        Note::Resumed { resent, after } => tracing::info!(
            resent,
            after_ms = after.as_millis() as u64,
            "the session resumed on a new leg"
        ),
        Note::MoveFailed { why } => {
            tracing::warn!("the move to a new leg failed ({why}); the old leg goes on")
        }
        Note::Retry { why, wait } => tracing::debug!(
            wait_ms = wait.as_millis() as u64,
            "a new leg did not open: {why}"
        ),
        other => tracing::debug!("{other:?}"),
    }
}

/// The legs of one session of an operator: each one an operator socket to
/// `/v1/connect/NAME`.
struct Legs {
    relay: Relay,
    trust: pod_ws::Trust,
    name: String,
    token: String,
    /// The leg of now, for the log of its end.
    current: Mutex<Option<JoinHandle<road::Outcome>>>,
    /// The relay refused the connect token on a later leg.
    refused: AtomicBool,
}

impl Legs {
    /// Open a leg. The first leg cannot know the far end: it keeps the wait
    /// of the plain mode after the end of its input. A later leg is a leg
    /// of the layer: the node ends it.
    async fn open(&self, later: bool) -> Result<DuplexStream, ConnectError> {
        let limits = if later {
            OperatorLimits {
                node_ends: Some(NODE_ENDS),
                ..OperatorLimits::default()
            }
        } else {
            OperatorLimits {
                eof_idle: Some(EOF_IDLE),
                ..OperatorLimits::default()
            }
        };
        let config = OperatorConfig {
            relay: self.relay.primary(),
            name: &self.name,
            connect_token: &self.token,
            trust: &self.trust,
            ctx: self.relay.ctx(),
            timeout: TIMEOUT,
            limits,
            wire: self.relay.wire(),
        };
        let (link, leg) = tokio::io::duplex(PIPE);
        let handle = operator::start(&config, leg).await?;
        *self.current.lock().unwrap_or_else(|e| e.into_inner()) = Some(handle);
        Ok(link)
    }

    /// The leg of now, when it ended within `wait`: how it ended.
    async fn ended(&self, wait: Duration) -> Option<road::Outcome> {
        let handle = self
            .current
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()?;
        match tokio::time::timeout(wait, handle).await {
            Ok(Ok(outcome)) => Some(outcome),
            _ => None,
        }
    }

    /// A new leg after a loss, or for a move.
    async fn next(&self, moving: bool) -> Next<DuplexStream> {
        if !moving {
            if let Some(outcome) = self.ended(LEG_WAIT).await {
                tracing::debug!("the lost leg ended: {outcome:?}");
            }
        }
        match self.open(true).await {
            Ok(link) => Next::Link(link),
            Err(ConnectError::Refused {
                status: status @ (401 | 403),
                ..
            }) => {
                self.refused.store(true, Ordering::SeqCst);
                Next::Stop(format!(
                    "the relay refused the connect token for {} ({status}): the pair expired or was stopped",
                    self.name
                ))
            }
            Err(ConnectError::Config(why)) => Next::Stop(why),
            Err(err) => Next::Retry(err.to_string()),
        }
    }
}

/// The words of the warning of an operator before the end of the pair.
fn operator_expiry_words(name: &str, expires: u64, left: Duration) -> String {
    format!(
        "the pair {name} expires in {}, at {}; this session ends then: open a new session to the next pair",
        left_words(left),
        pod_core::clock::utc_minutes(expires / 1000)
    )
}

/// The operator side with the layer (`relay connect --resume`): as
/// [`super::connect`], and the session moves to a new leg before the limits
/// of the relay, and resumes after the loss of a leg.
pub async fn connect_resumable<R, W>(
    relay: &Relay,
    name: &str,
    token: &str,
    input: R,
    output: W,
    resume: Resume,
) -> Result<()>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    connect_layer(relay, name, token, (input, output), resume, None).await
}

/// [`connect_resumable`] with the end-to-end channel above the layer
/// (`relay connect --resume --seal`): the key of the node must be in the
/// allow list of `seal` (else exit 77), and the node must let in the key
/// of this end (else exit 77).
pub async fn connect_sealed<R, W>(
    relay: &Relay,
    name: &str,
    token: &str,
    input: R,
    output: W,
    resume: Resume,
    seal: &Seal,
) -> Result<()>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    connect_layer(relay, name, token, (input, output), resume, Some(seal)).await
}

async fn connect_layer<R, W>(
    relay: &Relay,
    name: &str,
    token: &str,
    (input, output): (R, W),
    resume: Resume,
    seal: Option<&Seal>,
) -> Result<()>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let legs = Arc::new(Legs {
        relay: relay.clone(),
        trust: pod_ws::Trust::Default,
        name: name.to_string(),
        token: token.to_string(),
        current: Mutex::new(None),
        refused: AtomicBool::new(false),
    });
    let first = legs
        .open(false)
        .await
        .map_err(|err| connect_error(name, err))?;
    let warnings = resume.expires.map(|expires| {
        let name = name.to_string();
        tokio::spawn(super::warn_expiry_with(expires, move |left| {
            operator_expiry_words(&name, expires, left)
        }))
    });
    // Each frame of the node reaches the reader at once (T-315).
    let app = tokio::io::join(input, FlushEach::new(output));
    let next = {
        let legs = legs.clone();
        move |moving: bool| {
            let legs = legs.clone();
            async move { legs.next(moving).await }
        }
    };
    let result = match seal {
        None => carry(first, app, resume.settings(), next).await,
        Some(seal) => carry_sealed(first, app, resume.settings(), next, seal).await,
    };
    if let Some(warnings) = warnings {
        warnings.abort();
    }
    match result {
        Ok(()) => Ok(()),
        // The first leg ended before the handshake: its outcome says why
        // (a refusal of the node, a close of the relay).
        Err(CarryError::Start(e)) => match legs
            .ended(LEG_WAIT)
            .await
            .and_then(|o| outcome_error(name, o))
        {
            Some(err) => Err(err),
            None => Err(fault(
                Fault::Unavailable,
                anyhow::anyhow!("the session to {name}: {e}"),
            )),
        },
        Err(CarryError::Stopped(why)) if legs.refused.load(Ordering::SeqCst) => {
            Err(fault(Fault::Config, anyhow::anyhow!(why)))
        }
        // 77 for a key that is refused at either end, 78 for no shared
        // suite.
        Err(CarryError::Channel(e)) => {
            Err(super::seal::e2e_fault(e).context(format!("the sealed session to {name}")))
        }
        Err(e) => Err(fault(
            Fault::Unavailable,
            anyhow::anyhow!("the session to {name}: {e}"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_settings_move_under_the_limits_of_the_relay() {
        let settings = Resume::default().settings();
        assert_eq!(settings.move_bytes, 48 << 20);
        assert_eq!(settings.move_age, Duration::from_secs(11 * 3600));
        assert!(settings.half_close);
        assert!(settings.window.is_some(), "the window of the pair");
        let line = operator_expiry_words("abc", 0, Duration::from_secs(3600));
        assert!(
            line.contains("the pair abc expires in 1 h, at 1970-01-01 00:00 UTC"),
            "{line}"
        );
    }
}
