//! The reverse mode of the WebSocket relay (issue 88): inbound access to a
//! host that has outbound HTTPS only, over the reverse road of `pod-relay`
//! (PodUtils/podcrates) in its plain mode.
//!
//! * `podnet relay pair` asks the relay for a pair (`POST /v1/pair`): a name,
//!   a node token, a connect token and a stop token. podnet keeps the pair in
//!   a file of mode 0600 and prints the connect token for the operator.
//! * `podnet relay node` holds the WebSocket `/v1/node/NAME`. For each
//!   `open {id}`, it connects to the local target and answers `ready {id}`
//!   or `reject {id, reason}`. A lost socket is opened again with a backoff.
//!   A close for cause (the pair expired, the operator stopped it, a refused
//!   token) stops the node: a new pair is necessary. After the target shuts
//!   down its write side, the session waits [`HALF_CLOSE_IDLE`] for the last
//!   bytes of the operator (T-196).
//! * `podnet relay connect NAME` opens `/v1/connect/NAME` for the operator,
//!   waits for `ready`, and then copies raw bytes: stdin and stdout (for an
//!   SSH `ProxyCommand`), or each connection of a local listener. After the
//!   end of the input, the answer of the target still comes for
//!   [`EOF_IDLE`] (issue 109).
//!
//! The relay authenticates tokens, not persons: give the connect token only
//! over a channel that you trust.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context as _, Result};
use pod_relay::pair::PairContext;
use pod_relay::reverse::{self as road, Exit, OperatorConfig, OperatorLimits, Outcome};
use pod_ws::ConnectError;
use tokio::io::{AsyncRead, AsyncWrite};

mod flush_each;
pub use flush_each::{FlushEach, LayerGuard};
mod resume;
pub use resume::{
    carry, connect_resumable, layered, node_settings, run_node_resumable, CarryError, Resume,
};

use crate::exit::{fault, Fault};
use crate::ws_relay::Relay;

mod pairfile;
pub use pairfile::{file_of, load, remove, save};
pub use pod_relay::reverse::local::{channel, ChannelHandler, Target};
pub use pod_relay::reverse::{Handler, SessionId, EOF_IDLE, HALF_CLOSE_IDLE};

/// The environment variable with the connect token of the operator.
pub const CONNECT_TOKEN_VARIABLE: &str = crate::instances::CONNECT_TOKEN_VARIABLE;
/// The time for a WebSocket handshake and for a call of the relay.
pub(crate) const TIMEOUT: Duration = Duration::from_secs(20);
/// The time for a local connect (the relay reaps a session with no `ready`
/// after 15 s).
const LOCAL_CONNECT: Duration = Duration::from_secs(10);

/// A pair of the relay: the pair of `pod-relay`, with the URL of its relay
/// (its scheme too: a plain relay of a test is `http://`).
pub struct Pair {
    pub name: String,
    /// Milliseconds since the epoch.
    pub expires: u64,
    /// The base URL of the relay.
    pub relay: String,
    inner: pod_relay::pair::Pair,
}

impl std::fmt::Debug for Pair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pair")
            .field("name", &self.name)
            .field("expires", &self.expires)
            .field("relay", &self.relay)
            .field("tokens", &"<redacted>")
            .finish()
    }
}

impl Pair {
    fn of(inner: pod_relay::pair::Pair, relay: String) -> Self {
        Self {
            name: inner.name.clone(),
            expires: u64::try_from(inner.expires_ms).unwrap_or(0),
            relay,
            inner,
        }
    }

    /// For the node's socket, `/v1/node/NAME`.
    pub fn node_token(&self) -> &str {
        self.inner.node_token()
    }

    /// For the operator's socket, `/v1/connect/NAME`.
    pub fn connect_token(&self) -> &str {
        self.inner.connect_token()
    }

    /// For `/v1/stop/NAME`; empty in a pair file with none.
    pub fn stop_token(&self) -> &str {
        self.inner.stop_token()
    }

    /// The time that is left; zero when the pair expired.
    pub fn left(&self) -> Duration {
        left_of(self.expires)
    }
}

/// The time that is left before `expires` (milliseconds since the epoch).
fn left_of(expires: u64) -> Duration {
    let end = UNIX_EPOCH + Duration::from_millis(expires);
    end.duration_since(SystemTime::now()).unwrap_or_default()
}

/// The calls of the relay of `relay` about a pair.
fn pair_context(relay: &Relay) -> PairContext<'_> {
    static TRUST: pod_ws::Trust = pod_ws::Trust::Default;
    PairContext {
        ctx: relay.ctx(),
        relay: relay.primary(),
        trust: &TRUST,
        timeout: TIMEOUT,
    }
}

/// Ask the relay for a new pair. A name collision (409) is asked again
/// once: names are random.
pub async fn pair(relay: &Relay) -> Result<Pair> {
    if relay.wire() != road::Wire::Tls {
        bail!("a pair is made over TLS: {} is a plain relay", relay.url());
    }
    let inner = pod_relay::pair::create(&pair_context(relay))
        .await
        .map_err(|e| anyhow::anyhow!("the relay refused a pair: {e}"))?;
    Ok(Pair::of(inner, relay.url()))
}

/// The directory of the pairs: `relay` in the state directory.
pub fn pairs_dir() -> Result<PathBuf> {
    let dir = crate::state::state_dir()?
        .context("no state directory (set XDG_STATE_HOME or HOME)")?
        .join("relay");
    Ok(dir)
}

/// Stop the node of `pair` and its sessions at the relay
/// (`POST /v1/stop/NAME` with the stop token).
pub async fn stop(pair: &Pair) -> Result<()> {
    if pair.stop_token().is_empty() {
        bail!("the pair {} has no stop token", pair.name);
    }
    let relay = Relay::single(&pair.relay, None)?;
    pod_relay::pair::stop(&pair_context(&relay), &pair.inner)
        .await
        .map(|_| ())
        .map_err(|e| anyhow::anyhow!("the relay did not stop {}: {e}", pair.name))
}

/// The dial of a TCP target of a node: direct, with podnet's resolver of no
/// DoH, as a local service is on this host or its network.
fn direct_dialer() -> pod_proxy::Dialer {
    let mut settings = pod_proxy::Settings::new("podnet", pod_proxy::ProxyChoice::Direct);
    settings.lowat = None;
    pod_proxy::Dialer::new(settings, crate::resolve::plain())
}

/// The times before the expiry of a pair at which the node warns, the
/// longest first (podssh).
pub const WARN_BEFORE: [Duration; 3] = [
    Duration::from_secs(24 * 3600),
    Duration::from_secs(6 * 3600),
    Duration::from_secs(3600),
];

/// `5 h 59 min`, `59 min`, or `less than a minute`.
pub fn left_words(left: Duration) -> String {
    let minutes = left.as_secs() / 60;
    match (minutes / 60, minutes % 60) {
        (0, 0) => "less than a minute".to_string(),
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

/// The part of [`WARN_BEFORE`] that `left` is in: the index of the shortest
/// time that is not shorter than `left`; none more than 24 h before.
fn band(left: Duration) -> Option<usize> {
    WARN_BEFORE.iter().rposition(|before| *before >= left)
}

/// The warning for `pair` with `left` time. `follow`: the node takes the
/// next stored pair at the expiry.
pub fn expiry_line(pair: &Pair, left: Duration, follow: bool) -> String {
    expiry_words(&pair.name, pair.expires, left, follow)
}

fn expiry_words(name: &str, expires: u64, left: Duration, follow: bool) -> String {
    let when = format!(
        "the pair {name} expires in {}, at {}",
        left_words(left),
        pod_core::clock::utc_minutes(expires / 1000)
    );
    if follow {
        format!("{when}; make the next pair now (podnet relay pair): this node takes it at the expiry, and its operators need its name and connect token")
    } else {
        format!("{when}; the next pair needs podnet relay pair and a new start of the node (a node with no pair name takes the next pair at the expiry)")
    }
}

/// Warn for the pair `name` at each part of [`WARN_BEFORE`] once; the first
/// at once when the node starts within it. Ends at the expiry.
async fn warn_expiry(name: String, expires: u64, follow: bool) {
    warn_expiry_with(expires, |left| expiry_words(&name, expires, left, follow)).await
}

/// Warn `words` of the time that is left at each part of [`WARN_BEFORE`]
/// once, before `expires`; the first at once within a part.
pub(crate) async fn warn_expiry_with(expires: u64, words: impl Fn(Duration) -> String) {
    let mut said: Option<usize> = None;
    loop {
        let left = left_of(expires);
        if left.is_zero() {
            return;
        }
        if let Some(part) = band(left) {
            if said.is_none_or(|before| part > before) {
                tracing::warn!("{}", words(left));
                said = Some(part);
            }
        }
        // Look again at the next time of WARN_BEFORE, at most each 10 min
        // (the clock of the host can jump).
        let edge = WARN_BEFORE
            .iter()
            .find(|before| **before < left)
            .map(|before| left - *before);
        let wait = edge
            .unwrap_or(Duration::from_secs(600))
            .min(Duration::from_secs(600))
            .max(Duration::from_secs(1));
        tokio::time::sleep(wait.min(left)).await;
    }
}

/// Run the node of `pair` until the pair ends, with the expiry warnings. A
/// lost socket is opened again with a backoff. With `follow` (the pair
/// directory, when no pair name was given), the node takes the next stored
/// pair at the expiry, with no new start.
pub async fn run_node(follow: Option<&Path>, pair: Pair, target: &Target) -> Result<()> {
    let handler = Arc::new(local_handler(target));
    run_node_with(follow, pair, handler, &target.to_string()).await
}

/// The handler of a node that connects each session to `target`.
pub(crate) fn local_handler(target: &Target) -> road::LocalHandler {
    road::LocalHandler {
        target: target.clone(),
        timeout: LOCAL_CONNECT,
        dialer: Arc::new(direct_dialer()),
    }
}

/// [`run_node`] with a `handler` of the process in place of a target: each
/// session that the relay opens goes to the handler, with no socket file
/// (T-083). [`channel`] gives a handler whose sessions a service of the
/// process takes. `shown` names the handler in the log and in the ready
/// document.
pub async fn run_node_with<H: Handler>(
    follow: Option<&Path>,
    pair: Pair,
    handler: Arc<H>,
    shown: &str,
) -> Result<()> {
    let mut pair = pair;
    loop {
        let warnings = tokio::spawn(warn_expiry(
            pair.name.clone(),
            pair.expires,
            follow.is_some(),
        ));
        let (result, back) = node(pair, handler.clone(), shown, true).await;
        pair = back;
        warnings.abort();
        let expired = pair.left() < Duration::from_secs(5);
        let next = match follow {
            Some(dir) if result.is_err() && expired => pairfile::next(dir, &pair.inner),
            _ => None,
        };
        let Some(next) = next else {
            return result;
        };
        crate::report::hide(next.node_token());
        tracing::warn!(
            old = %pair.name,
            new = %next.name,
            expires = %pod_core::clock::utc_minutes(next.expires / 1000),
            "the pair expired; the node takes the next stored pair"
        );
        pair = next;
    }
}

/// The node of `pair` for a service of the process that is not a node
/// command (the transit of a transfer, T-104): no expiry warnings, no next
/// pair, and no ready document, so the signals of the process keep their
/// default action. It ends as [`run_node_with`] ends.
pub async fn serve_pair<H: Handler>(pair: Pair, handler: Arc<H>, shown: &str) -> Result<()> {
    node(pair, handler, shown, false).await.0
}

/// The node of `pair` until it ends: at the expiry of the pair, at a close
/// for cause, or at a fault that a new socket would repeat. Gives `pair`
/// back. With `ready`, the node sends the ready document of a command when
/// it is online.
async fn node<H: Handler>(
    pair: Pair,
    handler: Arc<H>,
    shown: &str,
    ready: bool,
) -> (Result<()>, Pair) {
    let relay = match Relay::single(&pair.relay, None) {
        Ok(relay) => relay,
        Err(err) => return (Err(fault(Fault::Config, err)), pair),
    };
    let left = pair.left();
    let Pair {
        name,
        expires,
        relay: url,
        inner,
    } = pair;
    let shown = shown.to_string();
    let (said_name, said_url) = (name.clone(), url.clone());
    let say = move |line: String| {
        if line.starts_with("online") {
            tracing::info!(name = %said_name, target = %shown, "the node is connected to the relay");
            if ready {
                crate::detach::ready(ready_document(&said_name, &said_url, expires, &shown));
            }
        } else {
            tracing::warn!(name = %said_name, "{line}");
        }
    };
    let trust = pod_ws::Trust::Default;
    let mut config = road::NodeConfig {
        pair: inner,
        label: None,
        trust: &trust,
        ctx: relay.ctx(),
        timeout: TIMEOUT,
        settings: road::Settings {
            half_close: Some(HALF_CLOSE_IDLE),
            ..road::Settings::default()
        },
        repair: None,
        wire: relay.wire(),
        say: Some(&say),
    };
    let exit = road::run(&mut config, handler, tokio::time::sleep(left)).await;
    let pair = Pair::of(config.pair, url);
    let name = &name;
    let (class, why) = match exit {
        // The only stop is the end of the pair.
        Exit::Stopped | Exit::PairExpired => (
            Fault::Config,
            format!("the pair {name} expired; make a new pair (podnet relay pair)"),
        ),
        Exit::PairStopped => (
            Fault::Config,
            format!("the relay ended the node {name}: the operator stopped the pair; make a new pair (podnet relay pair)"),
        ),
        Exit::Forbidden => (
            Fault::Config,
            format!("the relay refused the node token of {name}: the pair is stopped or the token is wrong; make a new pair (podnet relay pair)"),
        ),
        Exit::NameInUse => (
            Fault::Unavailable,
            format!("another node holds the name {name} at the relay (409)"),
        ),
        Exit::Unusable(why) => (
            Fault::Config,
            format!("the node {name} cannot connect to the relay: {why}"),
        ),
    };
    (Err(fault(class, anyhow::anyhow!(why))), pair)
}

/// The ready document of a node (issue 119): the key `name` is the name of
/// the instance (`detach::ready` sets it), so the pair is `pair`, with its
/// end.
pub fn node_ready(pair: &Pair, target: &Target) -> serde_json::Value {
    ready_document(&pair.name, &pair.relay, pair.expires, &target.to_string())
}

fn ready_document(name: &str, relay: &str, expires: u64, target: &str) -> serde_json::Value {
    serde_json::json!({
        "pair": name,
        "relay": relay,
        "target": target,
        "expires": expires,
        "expires_in": left_of(expires).as_secs(),
    })
}

/// The detached instances that serve the node of the pair `name` (issue
/// 119: `relay stop` stops them too).
pub fn instances_of_pair(records: &[crate::instances::Record], name: &str) -> Vec<String> {
    records
        .iter()
        .filter(|r| r.command.starts_with("relay node"))
        .filter(|r| {
            r.ready
                .as_ref()
                .and_then(|d| d.get("pair"))
                .and_then(|v| v.as_str())
                .map(|p| p == name)
                .unwrap_or_else(|| r.args.iter().any(|a| a == name))
        })
        .map(|r| r.name.clone())
        .collect()
}

/// The operator side: open a session to the node `name`, wait for `ready`,
/// then copy `input` to the node and the node to `output`. The end of the
/// input does not close the session (issue 109): it ends when the relay
/// closes it, or after [`EOF_IDLE`] with no data of the node. A close other
/// than 1000 is an error (podssh #84): the output can be cut.
pub async fn connect<R, W>(
    relay: &Relay,
    name: &str,
    token: &str,
    input: R,
    output: W,
) -> Result<()>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let trust = pod_ws::Trust::Default;
    let config = OperatorConfig {
        relay: relay.primary(),
        name,
        connect_token: token,
        trust: &trust,
        ctx: relay.ctx(),
        timeout: TIMEOUT,
        limits: OperatorLimits {
            eof_idle: Some(EOF_IDLE),
            ..OperatorLimits::default()
        },
        wire: relay.wire(),
    };
    let seen = Arc::new(std::sync::atomic::AtomicBool::new(false));
    // Each frame of the node reaches the reader at once (T-315). A node of
    // the resumable layer greets first: the guard stops the session.
    let output = FlushEach::new(LayerGuard::new(output, seen.clone()));
    let outcome = road::operator::run(&config, tokio::io::join(input, output)).await;
    if seen.load(std::sync::atomic::Ordering::SeqCst) {
        return Err(fault(
            Fault::Config,
            anyhow::anyhow!("the node {name} runs the resumable layer (relay node --resume): use relay connect --resume"),
        ));
    }
    match outcome {
        Ok(outcome) => match outcome_error(name, outcome) {
            Some(err) => Err(err),
            None => Ok(()),
        },
        Err(err) => Err(connect_error(name, err)),
    }
}

/// The error of an operator session that ended as `outcome`; none for a
/// success.
pub(crate) fn outcome_error(name: &str, outcome: Outcome) -> Option<anyhow::Error> {
    let unavailable = |why: String| Some(fault(Fault::Unavailable, anyhow::anyhow!(why)));
    match outcome {
        outcome if outcome.is_success() => None,
        Outcome::NeverReady { code, reason } => match (reason.is_empty(), code) {
            (false, _) => unavailable(format!("the node {name} refused the session: {reason}")),
            (true, Some(code)) => unavailable(format!(
                "the relay closed the session to {name}: {code}{}",
                close_hint(code)
            )),
            (true, None) => unavailable(format!("the relay closed the session to {name}")),
        },
        Outcome::Ended { code, reason } => unavailable(format!(
            "the session to {name} ended: {code} {reason}{}",
            close_hint(code)
        )),
        Outcome::LocalEnd => None,
    }
}

/// The error of an operator socket that did not open.
pub(crate) fn connect_error(name: &str, err: ConnectError) -> anyhow::Error {
    match err {
        ConnectError::Config(why) => fault(Fault::Usage, anyhow::anyhow!(why)),
        ConnectError::Refused {
            status: status @ (401 | 403),
            ..
        } => fault(
            Fault::Config,
            anyhow::anyhow!("the relay refused the connect token for {name} ({status})"),
        ),
        ConnectError::Refused { status, body } => fault(
            Fault::Unavailable,
            anyhow::anyhow!(
                "the relay refused the session to {name}: HTTP {status}: {}",
                body.chars().take(200).collect::<String>()
            ),
        ),
        err => fault(
            Fault::Unavailable,
            anyhow::anyhow!("the session to {name}: {err}"),
        ),
    }
}

/// What a close code of the relay means, for the error line.
pub use pod_ws::close_hint;

#[cfg(test)]
mod tests {
    use super::*;

    fn record(
        name: &str,
        command: &str,
        args: &[&str],
        pair: Option<&str>,
    ) -> crate::instances::Record {
        crate::instances::Record {
            name: name.into(),
            pid: 1,
            supervisor: None,
            started: 0,
            command: command.into(),
            args: args.iter().map(|a| a.to_string()).collect(),
            hash: String::new(),
            cwd: "/".into(),
            state: "ready".into(),
            ready: pair.map(|p| serde_json::json!({ "pair": p })),
            log: String::new(),
            restarts: 0,
            retry_after_s: None,
        }
    }

    // Issue 119: relay stop finds the node instances of its pair.
    #[test]
    fn the_instances_of_a_pair() {
        let records = [
            record("relay-node-a1", "relay node", &[], Some("ssh-box")),
            record(
                "relay-node-b2",
                "relay node",
                &["relay", "node", "ssh-box"],
                None,
            ),
            record("relay-node-c3", "relay node", &[], Some("other")),
            record("ssh-box", "tunnel", &["ssh-box"], None),
        ];
        assert_eq!(
            instances_of_pair(&records, "ssh-box"),
            ["relay-node-a1", "relay-node-b2"]
        );
    }

    // Issue 119: the ready document names the pair and its end.
    #[test]
    fn the_node_ready_document_names_the_pair() {
        let dir = tempfile::tempdir().unwrap();
        let expires = (SystemTime::now() + Duration::from_secs(600))
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let pair = pairfile::tests::written(dir.path(), "ssh-box", expires);
        assert_eq!(pair.relay, "https://relay.example");
        let doc = node_ready(&pair, &Target::parse("127.0.0.1:22").unwrap());
        assert_eq!(doc["pair"], "ssh-box");
        assert!(doc.get("name").is_none());
        assert_eq!(doc["expires"], pair.expires);
        assert_eq!(doc["target"], "127.0.0.1:22");
        assert!(doc["expires_in"].as_u64().unwrap() > 590);
    }

    #[test]
    fn the_expiry_words_and_parts() {
        assert_eq!(left_words(Duration::from_secs(30)), "less than a minute");
        assert_eq!(left_words(Duration::from_secs(59 * 60 + 59)), "59 min");
        assert_eq!(left_words(Duration::from_secs(6 * 3600)), "6 h");
        assert_eq!(
            left_words(Duration::from_secs(5 * 3600 + 59 * 60)),
            "5 h 59 min"
        );
        assert_eq!(band(Duration::from_secs(25 * 3600)), None);
        assert_eq!(band(Duration::from_secs(24 * 3600)), Some(0));
        assert_eq!(band(Duration::from_secs(7 * 3600)), Some(0));
        assert_eq!(band(Duration::from_secs(6 * 3600)), Some(1));
        assert_eq!(band(Duration::from_secs(59 * 60)), Some(2));
        let dir = tempfile::tempdir().unwrap();
        let pair = pairfile::tests::written(dir.path(), "abc", 0);
        let line = expiry_line(&pair, Duration::from_secs(3600), true);
        assert!(
            line.contains("the pair abc expires in 1 h, at 1970-01-01 00:00 UTC"),
            "{line}"
        );
        assert!(line.contains("takes it at the expiry"), "{line}");
        let line = expiry_line(&pair, Duration::from_secs(3600), false);
        assert!(line.contains("a new start of the node"), "{line}");
    }
}
