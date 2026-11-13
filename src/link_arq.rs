//! A link over any lossy datagram carrier: the selective-repeat ARQ (T-277).
//!
//! `pod_link::arq` of PodUtils/podcrates does the work: frames with a
//! 32-bit sequence, a 16-bit fragment index and a fragment count; fragments
//! of at most 1 KiB; an `ACK` with a bitmap of 256 frames; at most 256
//! frames in flight; a resend of each lost fragment by itself; an RTO of
//! 200 ms to 8 s; 8 retries, then `arq-timeout`. The carriers `dns` and
//! `icmp` run it inside the crate, under their TLS. This module is the glue
//! for a carrier of datagrams that has no ARQ of its own (a
//! [`pod_link::datagram::Datagrams`] of the caller):
//!
//! * [`over`] starts the ARQ over the datagrams, a [`Link`] of podnet over
//!   the ARQ, and a task that answers each rekey that the ARQ asks for
//!   before its sequence wraps ([`Rekey`]).
//! * The ARQ is below the link and knows no stream of it. A stream that the
//!   peer resets, or that ends, closes alone; the link and its other streams
//!   go on. The ARQ ends only when a frame of the link (data or control)
//!   gets no acknowledgement after its retries: the carrier is gone, so the
//!   link ends with it (exit 124, [`arq_fault`]).
//! * The sequence never wraps to 0 under the same keys with no rekey: new
//!   frames wait at the wrap until the [`Rekey`] answers. A [`Rekey`] that
//!   fails leaves the frames at the wrap, and the keepalive of the link ends
//!   the link.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

pub use pod_link::arq::{ArqError, ArqHandle, ArqSettings, ArqStream, Stats};
pub use pod_link::datagram::Datagrams;
use pod_link::{Link, Role, Settings};
use tokio::task::JoinHandle;

use crate::exit::{self, Fault};

/// The future of a rekey hook.
pub type RekeyFuture = Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send>>;

/// What the glue does when the ARQ asks for a rekey before the wrap of its
/// sequence.
pub enum Rekey {
    /// A layer above the ARQ seals each byte with its own nonces and its own
    /// rekey (the TLS of a carrier, or the record layer of a sealed stream,
    /// [`crate::link_record`]). The ARQ sequence is no nonce of that layer,
    /// so its wrap reuses no key and no nonce: the glue lets the sequence
    /// wrap, and counts it.
    Above,
    /// Call the hook, and let the sequence wrap when it gives `Ok`. An error
    /// keeps each new frame at the wrap.
    With(Box<dyn FnMut() -> RekeyFuture + Send>),
}

struct State {
    rekeys: AtomicU64,
    failure: Mutex<Option<String>>,
}

/// A link over the ARQ, with the handle of the ARQ and the task of the
/// rekeys. A drop stops the task of the rekeys.
pub struct ArqLink {
    /// The link of podnet over the ARQ.
    pub link: Link,
    /// The counters, the error and the rekey state of the ARQ.
    pub arq: ArqHandle,
    state: Arc<State>,
    task: JoinHandle<()>,
}

impl ArqLink {
    /// The rekeys that the glue answered.
    pub fn rekeys(&self) -> u64 {
        self.state.rekeys.load(Ordering::SeqCst)
    }

    /// The error of the rekey hook that stopped the wrap, if one failed.
    pub fn rekey_failure(&self) -> Option<String> {
        self.state
            .failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Why the ARQ ended, as an exit error of podnet ([`arq_fault`]);
    /// `None` while the ARQ runs.
    pub fn arq_error(&self) -> Option<anyhow::Error> {
        self.arq.error().map(arq_fault)
    }
}

impl Drop for ArqLink {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// An error of the ARQ with the exit class of podnet: a setting out of its
/// limits exits 64, `arq-timeout` exits 124, a broken protocol exits 65, a
/// failed carrier exits 69.
pub fn arq_fault(error: ArqError) -> anyhow::Error {
    let fault = match &error {
        ArqError::Settings(_) => Fault::Usage,
        ArqError::Timeout { .. } => Fault::Timeout,
        ArqError::Protocol(_) => Fault::DataError,
        ArqError::Carrier { .. } | ArqError::WriteClosed => Fault::Unavailable,
    };
    exit::fault(fault, anyhow::Error::new(error))
}

/// Start the ARQ over `datagrams` with `arq`, a link of podnet as `role`
/// with `settings` over it, and the task that answers the rekeys with
/// `rekey`. It must run in a tokio runtime with the time driver on. Both
/// ends give the same `arq.start`. A setting out of its limits exits 64.
pub fn over<D: Datagrams>(
    datagrams: D,
    role: Role,
    arq: ArqSettings,
    settings: Settings,
    rekey: Rekey,
) -> anyhow::Result<ArqLink> {
    let stream = ArqStream::new(datagrams, arq).map_err(arq_fault)?;
    let handle = stream.handle();
    let link = crate::link::from_io(stream, role, settings)?;
    let state = Arc::new(State {
        rekeys: AtomicU64::new(0),
        failure: Mutex::new(None),
    });
    let task = tokio::spawn(answer(handle.clone(), rekey, state.clone()));
    Ok(ArqLink {
        link,
        arq: handle,
        state,
        task,
    })
}

/// Answer each rekey that `arq` asks for, until a hook fails.
async fn answer(arq: ArqHandle, mut rekey: Rekey, state: Arc<State>) {
    loop {
        arq.rekey_wanted().await;
        if let Rekey::With(hook) = &mut rekey {
            if let Err(e) = hook().await {
                *state.failure.lock().unwrap_or_else(PoisonError::into_inner) =
                    Some(format!("{e:#}"));
                return;
            }
        }
        state.rekeys.fetch_add(1, Ordering::SeqCst);
        arq.rekeyed();
    }
}
