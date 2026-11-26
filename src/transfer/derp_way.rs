//! A DERP server as a way of the transit (T-128). It is a podnet extension
//! of the transit of magic-wormhole, for two podnet ends that reach no
//! direct hint and no transit relay, as the podnet relay is (T-104), with
//! no pair and no token.
//!
//! * The sender runs a node at a DERP server for the time of the transfer
//!   ([`start`]), under a key of this run, with the resumable layer of
//!   `pod-session` on each link (`crate::derp`). Its hint is [`Hint::Derp`]:
//!   the `derp:` ticket of the node.
//! * The receiver carries a stream to the node of the ticket (`dial`): the
//!   session goes on after the loss of a link. A node that does not answer
//!   is dialed again after [`RETRY`].
//! * On the stream, the two ends run the handshake of the transit
//!   ([`super::connect`]), as on a direct connection. The server sees only
//!   records that the transit key seals.
//! * A client of magic-wormhole ignores the hint, as it ignores each kind
//!   of hint that it does not know. podnet sends a file only to podnet.

use std::sync::Arc;
use std::time::Duration;

use pod_core::BoxedIo;
use pod_identity::Identity;
use tokio::io::DuplexStream;
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;

use super::messages::Hint;
use super::podnet_relay::Carried;
use super::transit::Role;
use crate::derp::{self, Road, Ticket};
use crate::exit::{self, Fault};

/// The wait before a new dial of a node that did not answer.
pub const RETRY: Duration = Duration::from_secs(1);
/// The sessions of a node that wait for the transit.
const DEPTH: usize = 4;
/// The bytes between the layer and the transit, in each direction.
const PIPE: usize = 256 * 1024;

/// The sessions that the node of a sender takes.
pub type Sessions = mpsc::Receiver<DuplexStream>;

/// The node of a sender. It runs while this value lives.
pub struct Node {
    task: JoinHandle<()>,
    /// The hint of the node.
    pub hint: Hint,
    /// What the node is: `DERP HOST`, with the server of its home.
    pub what: String,
}

impl Drop for Node {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Bring a node of a new key online on `road`, and serve its sessions: each
/// one goes to the receiver of the result. No server takes the node: exit
/// 69.
pub async fn start(road: &Road) -> anyhow::Result<(Node, Sessions)> {
    let mut seed = [0u8; 32];
    getrandom::fill(&mut seed).map_err(|e| {
        exit::fault(
            Fault::Unavailable,
            anyhow::anyhow!("no random seed for the key of the DERP node: {e}"),
        )
    })?;
    let me = Identity::from_seed(&seed);
    let online = derp::online(&me, road).await?;
    let hint = Hint::Derp {
        ticket: online.ticket.to_string(),
    };
    let what = format!("DERP {}", online.ticket.server);
    let (tx, sessions) = mpsc::channel(DEPTH);
    let open = move || {
        let tx = tx.clone();
        async move {
            let (node_end, transfer_end) = tokio::io::duplex(PIPE);
            tx.send(transfer_end)
                .await
                .map_err(|_| "the transfer is gone".to_string())?;
            Ok(node_end)
        }
    };
    let moved = |ticket: &Ticket| {
        tracing::info!(server = %ticket.server, "the DERP node of the transfer moved");
    };
    let task = tokio::spawn(derp::serve_with(online, road.clone(), open, moved));
    Ok((Node { task, hint, what }, sessions))
}

/// Dial the node of `ticket` on `road`, again after each [`RETRY`], until a
/// stream passes the handshake of the transit. The stream and what it is
/// (`DERP HOST`). A ticket that does not parse gives `None`.
pub(super) async fn dial(
    ticket: String,
    road: Road,
    role: Role,
    key: [u8; 32],
    won: Arc<Mutex<bool>>,
) -> Option<(BoxedIo, String)> {
    let ticket = Ticket::parse(&ticket).ok()?;
    let what = format!("DERP {}", ticket.server);
    loop {
        let (mine, app) = tokio::io::duplex(PIPE);
        let (ticket_of, road_of) = (ticket.clone(), road.clone());
        let task = tokio::spawn(async move { derp::carry(&ticket_of, &road_of, app).await });
        let carried = Carried::new(mine, task);
        if let Some(io) = super::connect::handshake(Box::new(carried), role, key, won.clone()).await
        {
            return Some((io, what));
        }
        tokio::time::sleep(RETRY).await;
    }
}
