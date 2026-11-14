//! The end-to-end channel on the reverse road (T-087): `relay node --resume
//! --seal` and `relay connect --resume --seal`.
//!
//! `pod-e2e` and `pod-relay` of PodUtils/podcrates do the work. This module
//! is the glue:
//!
//! * The node runs `pod_relay::reverse::E2e` under the far end of the
//!   resumable layer: each session answers the handshake of the channel
//!   (Noise XX with the Ed25519 key of each end), and the node dials the
//!   target only after the key of the operator is in the allow list of the
//!   node. A refused operator costs no connection to the target and no
//!   replay memory of the layer.
//! * The operator runs `pod_e2e::ends::operator` over the client of the
//!   layer. It checks the key of the node against its own allow list before
//!   it proves its key.
//! * The channel is above the layer: a move and a resume carry the same
//!   channel on. The relay sees the records of the layer, and in them the
//!   messages of the channel only.
//!
//! The CLI gives the key of the install (`podnet::identity`, the key of the
//! link) and its allow list.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use pod_core::{Event, Level};
use pod_e2e::ends::{self, Node};
use pod_identity::PublicKey;
use pod_relay::reverse::{E2e, Layered, LocalHandler};
use pod_session::resume::Next;
use pod_session::Settings;
use tokio::io::{AsyncRead, AsyncWrite};

use super::resume::{carry, layered, CarryError};
use crate::exit::{self, Fault};

/// How long the end of the channel is awaited after the end of the layer.
const CHANNEL_WAIT: Duration = Duration::from_secs(5);

/// Whether a key of the other end is allowed.
type Allows = dyn Fn(&PublicKey) -> bool + Send + Sync;

/// The end-to-end channel of one end: the key of this end, and the keys of
/// the other ends that it trusts.
#[derive(Clone)]
pub struct Seal {
    identity: Arc<pod_identity::Identity>,
    allows: Arc<Allows>,
}

impl Seal {
    /// The channel with the key `me`. `allows` says whether a key of the
    /// other end is allowed: a node lets in the operators that it allows,
    /// and an operator goes on only to a node that it allows.
    pub fn new(
        me: &pod_identity::Identity,
        allows: impl Fn(&PublicKey) -> bool + Send + Sync + 'static,
    ) -> Seal {
        Seal {
            identity: Arc::new(pod_identity::Identity::from_seed(&me.seed())),
            allows: Arc::new(allows),
        }
    }

    /// The public key of this end.
    pub fn public(&self) -> PublicKey {
        self.identity.public()
    }
}

/// The lines of the channel on a node go to the log.
fn log(event: Event) {
    match event.level {
        Level::Info => tracing::info!(kind = event.kind, "{event}"),
        Level::Warn => tracing::warn!(kind = event.kind, "{event}"),
    }
}

/// The node end of the channel over `inner`: an operator whose key is not
/// in the allow list gets a refusal, and the target is not dialed.
pub fn sealed<H: pod_relay::reverse::Handler>(inner: H, seal: &Seal) -> E2e<H> {
    let mut node = Node::open_to_all(seal.identity.clone(), Arc::new(log));
    let allows = seal.allows.clone();
    node.admit = Arc::new(move |key: &PublicKey, _ask: &pod_e2e::Ask| {
        if allows(key) {
            Ok(())
        } else {
            Err(format!(
                "the key {} is not in the allow list",
                key.display_fingerprint()
            ))
        }
    });
    E2e::new(inner, node)
}

/// The handler of `relay node --resume --seal`: the far end of the layer
/// over the channel over the dial of the target.
pub fn layered_sealed(inner: LocalHandler, seal: &Seal) -> Layered<E2e<LocalHandler>> {
    layered(sealed(inner, seal))
}

/// [`carry`] with the channel above the layer: `app` carries the plain
/// bytes, and the layer carries the messages of the channel. A refusal of
/// the channel, at either end, is [`CarryError::Channel`].
pub async fn carry_sealed<L, A, N, F>(
    first: L,
    app: A,
    settings: Settings,
    next: N,
    seal: &Seal,
) -> Result<(), CarryError>
where
    L: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    A: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    N: FnMut(bool) -> F,
    F: Future<Output = Next<L>>,
{
    let allows = seal.allows.clone();
    let check = move |node: &PublicKey| {
        if allows(node) {
            Ok(())
        } else {
            Err(format!(
                "the node key {} is not in the allow list of this end",
                node.display_fingerprint()
            ))
        }
    };
    let (records, channel) = ends::operator(app, seal.identity.clone(), check);
    let mut channel = Box::pin(channel);
    let mut layer = Box::pin(carry(first, records, settings, next));
    tokio::select! {
        ended = &mut channel => match ended {
            Err(e) => Err(CarryError::Channel(e)),
            // Both directions of the channel ended: the layer ends next.
            Ok(node) => {
                tracing::debug!(node = %node.display_fingerprint(), "the channel ended");
                layer.await
            }
        },
        ended = &mut layer => {
            // A refusal of the node can come just before the end of the
            // layer: the channel says why.
            match tokio::time::timeout(CHANNEL_WAIT, channel).await {
                Ok(Err(e)) => Err(CarryError::Channel(e)),
                _ => ended,
            }
        }
    }
}

/// The exit class of a failure of `pod-e2e`: 78 for a suite that does not
/// match, 77 for a refused key, 124 for a timeout, 69 for the others. The
/// sealed streams of the link use it too.
pub fn e2e_fault(error: pod_e2e::Error) -> anyhow::Error {
    use pod_core::Class;
    let failure: pod_core::Failure = error.into();
    let how = match failure.class {
        Class::Config => Fault::Config,
        Class::Refused => Fault::NoPermission,
        Class::Timeout => Fault::Timeout,
        _ => Fault::Unavailable,
    };
    exit::fault(how, anyhow::anyhow!("{failure}"))
}
