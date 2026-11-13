//! The verbs `reconnect` and `rotate` of the control socket (T-307, GitHub
//! #476, row E10-2).
//!
//! Both verbs make a new link to the peer of an old link. An instance that
//! dialed (`podnet link connect`) holds a [`Redial`]: the carrier, the
//! address, the identity, the allow list and the settings of its first dial.
//! An instance that listens holds none. A listener does not dial, so both
//! verbs answer 69 there.
//!
//! * `reconnect` (`"link":ID`): reset the streams of the link, end the link,
//!   and dial again with the same identity. Each reconnect makes a new
//!   session: the streams that ran end with a reset (`CANCEL`) and are
//!   named in `reset_streams`.
//! * `rotate` (`"link":ID`): dial a new link first, pin it for the streams
//!   that open from now on, and end the old link when its streams end.
//!   No stream fails.
//!
//! The new link must prove the key of the old link. Each verb answers the
//! status document after the change with `link` (the id of the new link) and
//! `replaced` (the id of the old link).

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use pod_identity::PublicKey;
use serde_json::Value;

use super::{Answer, Held, Hub, Reply, Slot, LIMIT};
use crate::exit;

/// The time that a new dial can take.
const DIAL_LIMIT: Duration = Duration::from_secs(30);
/// The time between two looks of `rotate` at the streams of the old link.
const DRAIN_STEP: Duration = Duration::from_millis(100);

/// A future that a [`Redial`] returns.
pub type Dialing<'a> = Pin<Box<dyn Future<Output = anyhow::Result<u64>> + Send + 'a>>;

/// The way to make a link again. [`Redial::redial`] dials the peer, checks
/// that it proves `expect`, holds the new link in `hub`, starts the task
/// that serves it, and returns its id.
pub trait Redial: Send + Sync {
    fn redial<'a>(&'a self, hub: &'a Arc<Hub>, expect: &'a PublicKey) -> Dialing<'a>;
}

/// What a hub keeps for the two verbs.
#[derive(Default)]
pub(super) struct State {
    dialer: OnceLock<Arc<dyn Redial>>,
    busy: AtomicUsize,
}

/// A change of links runs: the instance does not end for want of a link.
struct Busy<'a>(&'a AtomicUsize);

impl Drop for Busy<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Slot {
    /// The id of the link that this slot holds.
    pub fn id(&self) -> u64 {
        self.held.id
    }
}

impl Hub {
    /// Give the verbs `reconnect` and `rotate` the way to dial again.
    pub fn attach_redial(&self, dialer: Arc<dyn Redial>) {
        let _ = self.redial.dialer.set(dialer);
    }

    /// The links that the instance holds.
    pub fn links(&self) -> Vec<Arc<crate::link::Link>> {
        self.rows().iter().map(|h| h.link.clone()).collect()
    }

    /// True while no link is held and no change of links runs: the instance
    /// has nothing to serve.
    pub fn down(&self) -> bool {
        self.rows().is_empty() && self.redial.busy.load(Ordering::SeqCst) == 0
    }

    /// The dialer, and the link of `request`. A listener has no dialer: 69.
    fn redialing(
        &self,
        verb: &str,
        request: &Value,
    ) -> Result<(Arc<dyn Redial>, Arc<Held>), Answer> {
        let held = self.link_of(super::verbs::number(request, "link")?)?;
        match self.redial.dialer.get() {
            Some(dialer) if held.role == "dialer" => Ok((dialer.clone(), held)),
            _ => Err(Answer::new(
                exit::UNAVAILABLE,
                format!("{verb}: this instance listens, and a listener does not dial"),
            )),
        }
    }

    /// The status document after `verb` replaced `old` with `new`.
    fn replaced(&self, verb: &str, old: u64, new: u64, reset: Vec<u32>) -> Reply {
        let Reply::Json(mut doc) = self.after(verb) else {
            unreachable!("after answers a document")
        };
        doc["link"] = new.into();
        doc["replaced"] = old.into();
        doc["reset_streams"] = reset.into();
        Reply::Json(doc)
    }

    async fn dial_again(self: &Arc<Hub>, dialer: &dyn Redial, old: &Held) -> Result<u64, Answer> {
        match tokio::time::timeout(DIAL_LIMIT, dialer.redial(self, &old.peer)).await {
            Ok(Ok(id)) => Ok(id),
            Ok(Err(err)) => Err(Answer::of(&err)),
            Err(_) => Err(Answer::new(
                exit::TIMEOUT,
                format!("the dial did not end in {} s", DIAL_LIMIT.as_secs()),
            )),
        }
    }

    /// `reconnect`: end the link and dial again.
    pub(super) async fn reconnect(self: &Arc<Hub>, request: &Value) -> Result<Reply, Answer> {
        let (dialer, old) = self.redialing("reconnect", request)?;
        self.redial.busy.fetch_add(1, Ordering::SeqCst);
        let _busy = Busy(&self.redial.busy);
        // A new session cannot carry the streams of the old one.
        let reset: Vec<u32> = {
            let streams = old.streams.lock().unwrap_or_else(|e| e.into_inner());
            streams.values().for_each(|cancel| cancel.notify_one());
            streams.keys().copied().collect()
        };
        if tokio::time::timeout(LIMIT, old.link.close()).await.is_err() {
            return Err(Answer::new(
                exit::TIMEOUT,
                format!("link {} did not end in {} s", old.id, LIMIT.as_secs()),
            ));
        }
        let new = self.dial_again(&*dialer, &old).await?;
        eprintln!("podnet: link    reconnect {} -> {new}", old.id);
        Ok(self.replaced("reconnect", old.id, new, reset))
    }

    /// `rotate`: dial a new link, move the new streams to it, and end the
    /// old link when its streams end.
    pub(super) async fn rotate(self: &Arc<Hub>, request: &Value) -> Result<Reply, Answer> {
        let (dialer, old) = self.redialing("rotate", request)?;
        if old.link.ended().is_some() {
            return Err(Answer::new(
                exit::UNAVAILABLE,
                format!("link {} has ended: use reconnect", old.id),
            ));
        }
        self.redial.busy.fetch_add(1, Ordering::SeqCst);
        let _busy = Busy(&self.redial.busy);
        let new = self.dial_again(&*dialer, &old).await?;
        {
            // The pin of another link stays.
            let mut pin = self.selected.lock().unwrap_or_else(|e| e.into_inner());
            if pin.is_none() || *pin == Some(old.id) {
                *pin = Some(new);
            }
        }
        let drained = old.clone();
        tokio::spawn(async move {
            while drained.link.ended().is_none() {
                let idle = drained.link.streams() == 0
                    && drained
                        .streams
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .is_empty();
                if idle {
                    drained.link.close().await;
                    break;
                }
                tokio::time::sleep(DRAIN_STEP).await;
            }
        });
        eprintln!("podnet: link    rotate {} -> {new}", old.id);
        Ok(self.replaced("rotate", old.id, new, Vec::new()))
    }
}
