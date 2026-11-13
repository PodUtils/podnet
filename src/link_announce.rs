//! The route announcements on a link (T-285, step 3: `--announce`).
//!
//! With `--announce`, each end of a link opens one stream of the service
//! [`NAME`] to the other end and sends its signed announcement on it
//! ([`crate::mesh_announce`]) each 10 s, with a jitter of 10% either way.
//! The first one goes at once. The announcement lists the direct neighbours
//! of this node (the peers of its links that are up) and its services.
//!
//! The frame on the stream is a big-endian `u16` length and then the wire
//! bytes of the announcement. A length above [`MAX_FRAME`] ends the stream
//! (not the link). The receiver checks an announcement with
//! [`crate::mesh_announce::verify`], then with the rules of this module,
//! then with the seen-`seq` rule, then it gives it to the route table
//! ([`crate::mesh_table`]):
//!
//! * The path must be `[origin]` and the origin must be the peer of the
//!   link. A multi-hop announcement is dropped until the forwarding exists
//!   (T-287). This stops a peer from sending the announcement of a node that
//!   it does not speak for.
//! * A dropped announcement changes nothing and the link stays up.
//!
//! The cost of the link to a peer is the configured weight, else the RTT of
//! the link in ms (at least 1), else [`DEFAULT_COST`].
//!
//! [`Mesh`] does no I/O and reads no clock: each call that depends on time
//! takes `now` in ms. The tasks of [`run_sender`] and [`receive_stream`] use
//! [`now_ms`].

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use pod_identity::PublicKey;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::identity::Identity;
use crate::link::{Link, Stream};
use crate::link_service as service;
use crate::mesh_announce::{self as announce, Body, Drop as Rule, Neighbour, Receiver};
use crate::mesh_table::{Learned, Limits, Refused, RouteTable};

/// The service name of the stream of announcements.
pub const NAME: &str = "announce";

/// The cost of a link when no weight is set and no RTT is known.
pub const DEFAULT_COST: u32 = 10;

/// The most bytes of one frame: the largest announcement.
pub const MAX_FRAME: usize = announce::MAX_BODY + 64 + 1 + 32 * announce::MAX_PATH;

/// The time in ms since the unix epoch.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// The cost of a link: the weight, else the RTT in ms (at least 1), else
/// [`DEFAULT_COST`].
pub fn link_cost(weight: Option<u32>, rtt: Option<Duration>) -> u32 {
    match (weight, rtt) {
        (Some(w), _) => w,
        (None, Some(rtt)) => (rtt.as_millis().min(u128::from(u32::MAX)) as u32).max(1),
        (None, None) => DEFAULT_COST,
    }
}

/// The wait before the next announcement. `random` is any `u64`. The wait is
/// `interval_ms` with a jitter of 10% either way.
pub fn next_delay(interval_ms: u64, random: u64) -> Duration {
    let span = interval_ms / 5;
    let low = interval_ms - interval_ms / 10;
    Duration::from_millis(low + if span == 0 { 0 } else { random % (span + 1) })
}

fn random_u64() -> u64 {
    let mut b = [0u8; 8];
    // Without entropy the jitter is none: the interval still holds.
    let _ = getrandom::fill(&mut b);
    u64::from_le_bytes(b)
}

/// Why a received announcement did not reach the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dropped {
    /// A rule of [`crate::mesh_announce`].
    Announcement(Rule),
    /// The path is not `[origin]`, or the origin is not the peer.
    NotDirect,
    /// The route table refused it.
    Table(Refused),
}

impl std::fmt::Display for Dropped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Dropped::Announcement(rule) => write!(f, "{rule}"),
            Dropped::NotDirect => f.write_str("the announcement is not from the peer itself"),
            Dropped::Table(why) => write!(f, "the route table refused it: {why}"),
        }
    }
}

struct State {
    me_key: PublicKey,
    receiver: Receiver,
    table: RouteTable,
    seq: u64,
    neighbours: HashMap<PublicKey, u32>,
    received: u64,
    dropped: u64,
}

/// The state of the announcements of one node: its neighbours, its `seq`,
/// the seen `seq` of the origins and the route table.
pub struct Mesh {
    me: Arc<Identity>,
    services: Vec<String>,
    state: Mutex<State>,
}

impl Mesh {
    /// A mesh for `me` that offers `services`. The first `seq` is `now`
    /// (ms), so that a restart does not use a `seq` again.
    pub fn new(me: Arc<Identity>, mut services: Vec<String>, now: u64) -> Arc<Mesh> {
        services.sort();
        services.dedup();
        services.truncate(announce::MAX_SERVICES);
        let key = me.public();
        Arc::new(Mesh {
            me,
            services,
            state: Mutex::new(State {
                me_key: key,
                receiver: Receiver::new(key),
                table: RouteTable::new(key, Limits::default()),
                seq: now,
                neighbours: HashMap::new(),
                received: 0,
                dropped: 0,
            }),
        })
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A link to `peer` is up, at `cost`.
    pub fn neighbour_up(&self, peer: PublicKey, cost: u32) {
        self.lock().neighbours.insert(peer, cost);
    }

    /// Change the cost of the link to `peer`.
    pub fn set_cost(&self, peer: &PublicKey, cost: u32) {
        if let Some(c) = self.lock().neighbours.get_mut(peer) {
            *c = cost;
        }
    }

    /// The link to `peer` is down: end its direct route at once.
    pub fn neighbour_down(&self, peer: &PublicKey, now: u64) {
        let mut st = self.lock();
        st.neighbours.remove(peer);
        st.table.session_closed(peer, now);
    }

    /// The signed announcement of this node at time `now`: the neighbours
    /// that are up (the first 64 by key), the services, the next `seq`.
    pub fn make(&self, now: u64) -> Result<Vec<u8>, announce::EncodeError> {
        let mut st = self.lock();
        st.seq = st.seq.saturating_add(1);
        let mut neighbours: Vec<Neighbour> = st
            .neighbours
            .iter()
            .map(|(key, cost)| Neighbour {
                key: *key,
                cost: *cost,
            })
            .collect();
        neighbours.sort_by_key(|n| n.key.0);
        neighbours.truncate(announce::MAX_NEIGHBOURS);
        let origin = self.me.public();
        let body = Body {
            origin,
            seq: st.seq,
            issued_ms: now,
            expires_ms: now + announce::LIFETIME_MS,
            neighbours,
            services: self.services.clone(),
        };
        announce::encode(self.me.key(), &body, &[origin])
    }

    /// Take an announcement that the peer `from` sent over a link of cost
    /// `cost`, at time `now`. A dropped announcement changes no route and
    /// keeps no `seq`.
    pub fn receive(
        &self,
        from: &PublicKey,
        cost: u32,
        wire: &[u8],
        now: u64,
    ) -> Result<Learned, Dropped> {
        let mut st = self.lock();
        let done = Self::take(&mut st, from, cost, wire, now);
        match &done {
            Ok(_) => st.received += 1,
            Err(_) => st.dropped += 1,
        }
        done
    }

    fn take(
        st: &mut State,
        from: &PublicKey,
        cost: u32,
        wire: &[u8],
        now: u64,
    ) -> Result<Learned, Dropped> {
        let v = announce::verify(wire, &st.me_key, now).map_err(Dropped::Announcement)?;
        if v.path.as_slice() != [*from] || v.body.origin != *from {
            return Err(Dropped::NotDirect);
        }
        let v = st.receiver.note(v, now).map_err(Dropped::Announcement)?;
        st.table
            .learn(*from, cost, &v.route(0), now)
            .map_err(Dropped::Table)
    }

    /// The best route of each origin as a JSON document (the verb `routes`).
    pub fn routes(&self, now: u64) -> Value {
        let st = self.lock();
        let mut rows: Vec<Value> = st
            .table
            .best_routes(now)
            .into_iter()
            .map(|r| {
                json!({
                    "origin": r.origin.display_fingerprint(),
                    "next_hop": r.next_hop.display_fingerprint(),
                    "cost": r.cost,
                    "hops": r.hops,
                    "seq": r.seq,
                    "expires_ms": r.expires_ms,
                    "direct": r.is_direct(),
                })
            })
            .collect();
        rows.sort_by_key(|r| r["origin"].as_str().unwrap_or_default().to_string());
        json!({
            "ok": true,
            "verb": "routes",
            "node": self.me.fingerprint(),
            "neighbours": st.neighbours.len(),
            "received": st.received,
            "dropped": st.dropped,
            "evictions": st.table.evictions(),
            "routes": rows,
        })
    }
}

/// Send the announcements of `mesh` to the peer of `link` until the link
/// ends or the peer has no service [`NAME`]. `weight` is the configured
/// cost of the link; `interval_ms` is [`announce::INTERVAL_MS`] unless a test
/// gives less.
pub async fn run_sender(
    mesh: Arc<Mesh>,
    link: Arc<Link>,
    peer: PublicKey,
    weight: Option<u32>,
    interval_ms: u64,
) {
    let mut stream = match link.open() {
        Ok(stream) => stream,
        Err(err) => {
            tracing::warn!(error = %err, "announce: no stream");
            return;
        }
    };
    if let Err(err) = service::request(&mut stream, NAME).await {
        tracing::warn!(error = %format!("{err:#}"), "announce: the peer takes no announcements");
        return;
    }
    loop {
        mesh.set_cost(&peer, link_cost(weight, link.rtt()));
        let wire = match mesh.make(now_ms()) {
            Ok(wire) => wire,
            Err(err) => {
                tracing::warn!(error = %err, "announce: no announcement");
                return;
            }
        };
        let mut frame = Vec::with_capacity(wire.len() + 2);
        frame.extend_from_slice(&(wire.len() as u16).to_be_bytes());
        frame.extend_from_slice(&wire);
        if stream.write_all(&frame).await.is_err() {
            return;
        }
        tokio::time::sleep(next_delay(interval_ms, random_u64())).await;
    }
}

/// Answer and read the frames of a stream that the peer opened for the
/// service [`NAME`] (the request is read) and give
/// each announcement to `mesh`. It returns when the stream ends or a length
/// is too large. A dropped announcement does not end it.
pub async fn receive_stream(
    mesh: Arc<Mesh>,
    link: Arc<Link>,
    peer: PublicKey,
    weight: Option<u32>,
    stream: &mut Stream,
) {
    if service::reply(stream, service::OPEN).await.is_err() {
        return;
    }
    let mut len = [0u8; 2];
    while stream.read_exact(&mut len).await.is_ok() {
        let n = usize::from(u16::from_be_bytes(len));
        if n > MAX_FRAME {
            tracing::warn!(peer = %peer.display_fingerprint(), "announce: a frame is too large");
            return;
        }
        let mut wire = vec![0u8; n];
        if stream.read_exact(&mut wire).await.is_err() {
            return;
        }
        let cost = link_cost(weight, link.rtt());
        if let Err(why) = mesh.receive(&peer, cost, &wire, now_ms()) {
            tracing::warn!(peer = %peer.display_fingerprint(), "announce: dropped: {why}");
        }
    }
}

/// A link in the mesh: the neighbour is up, and the sender runs. The end of
/// the link, or the drop, stops the sender and ends the direct route of the
/// peer.
pub struct Guard {
    mesh: Arc<Mesh>,
    peer: PublicKey,
    sender: tokio::task::JoinHandle<()>,
    watch: tokio::task::JoinHandle<()>,
}

impl Guard {
    /// Add the neighbour `peer` of `link` and start its sender.
    pub fn start(
        mesh: &Arc<Mesh>,
        link: &Arc<Link>,
        peer: PublicKey,
        weight: Option<u32>,
        interval_ms: u64,
    ) -> Guard {
        mesh.neighbour_up(peer, link_cost(weight, link.rtt()));
        let sender = tokio::spawn(run_sender(
            mesh.clone(),
            link.clone(),
            peer,
            weight,
            interval_ms,
        ));
        // The end of the link ends the direct route of the peer at once.
        let watch = {
            let (mesh, link) = (mesh.clone(), link.clone());
            tokio::spawn(async move {
                link.closed().await;
                mesh.neighbour_down(&peer, now_ms());
            })
        };
        Guard {
            mesh: mesh.clone(),
            peer,
            sender,
            watch,
        }
    }
}

impl std::ops::Drop for Guard {
    fn drop(&mut self) {
        self.sender.abort();
        self.watch.abort();
        self.mesh.neighbour_down(&self.peer, now_ms());
    }
}

/// Serve the streams that the peer of a link opens, for an end that has no
/// other service (the dialer). A stream for [`NAME`] goes to
/// [`receive_stream`]; any other service gets the answer "no such service".
pub async fn serve_streams(mesh: Arc<Mesh>, link: Arc<Link>, peer: PublicKey, weight: Option<u32>) {
    while let Ok(mut stream) = link.accept().await {
        let (mesh, link) = (mesh.clone(), link.clone());
        tokio::spawn(async move {
            match service::accept(&mut stream).await {
                Ok(name) if name == NAME => {
                    receive_stream(mesh, link, peer, weight, &mut stream).await;
                }
                Ok(_) => {
                    let _ = service::reply(&mut stream, service::NO_SERVICE).await;
                }
                Err(_) => {}
            }
        });
    }
}
