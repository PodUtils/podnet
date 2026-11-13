//! The verb `accept` of the control socket (T-297): a client registers a
//! service for the streams that the peer opens.
//!
//! The request is `{"verb":"accept","peer":FP,"service":NAME,"target":
//! "unix:/ABS/PATH"}`. The instance must hold a link that is up to the
//! peer (69 or 77 as for `open`). The name must not be a service of the
//! instance (`--service`) nor one that another client holds (64). The
//! target is a unix socket with an absolute path: a client of the control
//! socket gets no TCP target and no program. After the answer the
//! connection stays; the service ends when the client closes it. The
//! grant of the peer in the allow list applies to the name as to any
//! service.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::io::AsyncReadExt;
use tokio::net::UnixStream;

use super::{peer_list, Answer, Hub, Reply};
use crate::exit;

/// The names that the instance serves itself, and the names that clients
/// registered.
#[derive(Default)]
pub(super) struct Table {
    fixed: BTreeSet<String>,
    claimed: BTreeMap<String, PathBuf>,
}

/// A registered service. The drop removes it.
pub(super) struct Claim {
    hub: Arc<Hub>,
    name: String,
}

impl Drop for Claim {
    fn drop(&mut self) {
        self.hub
            .accepted
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .claimed
            .remove(&self.name);
    }
}

/// Wait until the client ends the connection (or sends anything: a client
/// of this verb sends nothing more).
pub(super) async fn wait_gone(stream: &mut UnixStream) {
    let mut sink = [0u8; 64];
    while matches!(stream.read(&mut sink).await, Ok(n) if n > 0) {}
}

impl Hub {
    /// Name the services of the instance itself: no client may take them.
    pub fn reserve<'a>(&self, names: impl IntoIterator<Item = &'a String>) {
        let mut table = self.accepted.lock().unwrap_or_else(|e| e.into_inner());
        table.fixed.extend(names.into_iter().cloned());
    }

    /// The socket of the service `name` that a client registered.
    pub fn claimed(&self, name: &str) -> Option<PathBuf> {
        let table = self.accepted.lock().unwrap_or_else(|e| e.into_inner());
        table.claimed.get(name).cloned()
    }

    /// `accept`: register the service.
    pub(super) fn accept(self: &Arc<Hub>, request: &Value) -> Result<Reply, Answer> {
        let usage = |text: String| Answer::new(exit::USAGE, format!("accept: {text}"));
        let peer =
            peer_list(request["peer"].as_str().unwrap_or_default()).map_err(|e| Answer::of(&e))?;
        let name = request["service"].as_str().unwrap_or_default();
        crate::link_service::check_name(name).map_err(|e| Answer::of(&e))?;
        let target = request["target"].as_str().unwrap_or_default();
        let path = target
            .strip_prefix("unix:")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .ok_or_else(|| usage("give a target `unix:/ABSOLUTE/PATH`".into()))?;
        let held = self.find(&peer)?;
        let mut table = self.accepted.lock().unwrap_or_else(|e| e.into_inner());
        if table.fixed.contains(name) || table.claimed.contains_key(name) {
            return Err(usage(format!("the service {name:?} exists already")));
        }
        table.claimed.insert(name.to_string(), path);
        drop(table);
        let claim = Claim {
            hub: self.clone(),
            name: name.to_string(),
        };
        let doc = json!({
            "ok": true, "verb": "accept", "service": name,
            "peer": held.peer.display_fingerprint(),
        });
        Ok(Reply::Hold(doc, claim))
    }
}
