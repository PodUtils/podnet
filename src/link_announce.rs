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
//! * The last node of the path must be the peer of the link. This stops a
//!   peer from sending the announcement of a node that it does not speak
//!   for. [`crate::mesh_announce::verify`] has checked the chain of hop
//!   signatures, and that each hop names the next node of the path and the
//!   last hop names this node (T-346).
//! * An announcement of another version of the wire is dropped; the log
//!   gets one line for each stream that names the version.
//! * A new announcement (it reached the route table) is forwarded: this node
//!   adds its hop ([`announce::append_hop`], with the cost of the link to the
//!   peer) and sends it to each other neighbour that is not in the path. The
//!   path of the sent announcement has at most [`crate::link_forward::DEFAULT_HOPS`]
//!   nodes.
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
use tokio::sync::mpsc;

use crate::identity::Identity;
use crate::link::{Link, Stream};
use crate::link_service as service;
use crate::mesh_announce::{self as announce, Body, Drop as Rule, Neighbour, Receiver};
use crate::mesh_table::{Learned, Limits, Refused, RouteTable};

pub mod limit;
mod status;
pub use status::{metrics_text, ViewLimits};

/// The service name of the stream of announcements.
pub const NAME: &str = "announce";

/// The cost of a link when no weight is set and no RTT is known.
pub const DEFAULT_COST: u32 = 10;

/// The most bytes of one frame: the largest announcement.
pub const MAX_FRAME: usize = announce::MAX_WIRE;

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
    /// The last node of the path is not the peer.
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

/// The frames that wait to go to a peer: the announcements of other nodes
/// that this node forwards.
pub const OUTBOX: usize = 64;

struct State {
    outbox: HashMap<PublicKey, mpsc::Sender<Vec<u8>>>,
    links: HashMap<PublicKey, Arc<Link>>,
    hop_limit: usize,
    forwarded: u64,
    /// The frames that the bucket of a stream dropped (T-347).
    limited: u64,
    /// The signature checks of one stream: each second, and at most.
    check_rate: (u32, u32),
    receiver: Receiver,
    table: RouteTable,
    seq: u64,
    neighbours: HashMap<PublicKey, u32>,
    received: u64,
    dropped: u64,
    seen: HashMap<PublicKey, Seen>,
    names: HashMap<PublicKey, String>,
}

/// What the last accepted announcement of an origin said (T-289).
struct Seen {
    at_ms: u64,
    services: Vec<String>,
    neighbours: Vec<(PublicKey, u32)>,
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
                outbox: HashMap::new(),
                links: HashMap::new(),
                hop_limit: usize::from(crate::link_forward::DEFAULT_HOPS),
                forwarded: 0,
                limited: 0,
                check_rate: (limit::RATE, limit::BURST),
                receiver: Receiver::new(key),
                table: RouteTable::new(key, Limits::default()),
                seq: now,
                neighbours: HashMap::new(),
                received: 0,
                dropped: 0,
                seen: HashMap::new(),
                names: HashMap::new(),
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

    /// Keep the link to `peer`: streams for other nodes go over it, and the
    /// forwarded announcements go to the channel `outbox`.
    pub fn attach_link(&self, peer: PublicKey, link: Arc<Link>, outbox: mpsc::Sender<Vec<u8>>) {
        let mut st = self.lock();
        st.links.insert(peer, link);
        st.outbox.insert(peer, outbox);
    }

    /// The most nodes of a path that this node forwards and of a route that
    /// it uses for a stream (1 to 16, default 8).
    pub fn set_hop_limit(&self, limit: u8) {
        self.lock().hop_limit = usize::from(limit.clamp(1, crate::link_forward::MAX_HOPS));
    }

    /// The signature checks that one announce stream can take: `rate` each
    /// second and `burst` at most (T-347; default 2048 and 8192). A stream
    /// takes the value when it starts.
    pub fn set_check_rate(&self, rate: u32, burst: u32) {
        self.lock().check_rate = (rate.max(1), burst.max(announce::MAX_PATH as u32 + 1));
    }

    fn check_rate(&self) -> (u32, u32) {
        self.lock().check_rate
    }

    /// Count a frame that the bucket of a stream dropped.
    fn note_limited(&self) {
        let mut st = self.lock();
        st.limited += 1;
        st.dropped += 1;
    }

    /// The key of this node.
    pub fn public(&self) -> PublicKey {
        self.me.public()
    }

    /// The origin whose fingerprint is `text` (as `routes` shows it), if the
    /// table holds a route to it.
    pub fn origin_named(&self, text: &str) -> Option<PublicKey> {
        let st = self.lock();
        let found = st
            .table
            .origins()
            .find(|k| k.display_fingerprint() == text)
            .copied();
        found
    }

    /// The links that lead to `dest`, best route first: for each usable
    /// route whose next hop is not in `skip`, has a link here and keeps the
    /// hop limit, the next hop and its link. The route table ranks them.
    pub fn ways(
        &self,
        dest: &PublicKey,
        skip: &[PublicKey],
        now: u64,
    ) -> Vec<(PublicKey, Arc<Link>)> {
        let st = self.lock();
        st.table
            .ranked(dest, now)
            .into_iter()
            .filter(|r| r.down_since_ms.is_none() && r.hops <= st.hop_limit)
            .filter(|r| !skip.contains(&r.next_hop))
            .filter_map(|r| Some((r.next_hop, st.links.get(&r.next_hop)?.clone())))
            .collect()
    }

    /// The link to `peer` is down: end its direct route at once.
    pub fn neighbour_down(&self, peer: &PublicKey, now: u64) {
        let mut st = self.lock();
        st.neighbours.remove(peer);
        st.links.remove(peer);
        st.outbox.remove(peer);
        st.table.session_closed(peer, now);
    }

    /// The signed announcement of this node at time `now` for the neighbour
    /// `to`: the neighbours that are up (the first 64 by key), the services,
    /// the next `seq`. The hop of this node names `to` (T-346).
    pub fn make(&self, now: u64, to: &PublicKey) -> Result<Vec<u8>, announce::EncodeError> {
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
        announce::encode(self.me.key(), &body, to)
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
        // The signature checks run before the lock: a flood on one stream
        // does not stop the other streams and the routes (T-347).
        let verified = announce::verify(wire, &self.me.public(), now);
        let mut st = self.lock();
        let done = verified
            .map_err(Dropped::Announcement)
            .and_then(|v| self.take(&mut st, from, cost, wire, v, now));
        match &done {
            Ok(_) => st.received += 1,
            Err(_) => st.dropped += 1,
        }
        done
    }

    fn take(
        &self,
        st: &mut State,
        from: &PublicKey,
        cost: u32,
        wire: &[u8],
        v: announce::Verified,
        now: u64,
    ) -> Result<Learned, Dropped> {
        if v.path.last() != Some(from) {
            return Err(Dropped::NotDirect);
        }
        if st
            .receiver
            .seq_of(&v.body.origin)
            .is_some_and(|seen| v.body.seq <= seen)
        {
            return Self::take_other_path(st, from, cost, &v, now);
        }
        let v = st.receiver.note(v, now).map_err(Dropped::Announcement)?;
        let learned = st
            .table
            .learn(*from, cost, &v.route(), now)
            .map_err(Dropped::Table)?;
        Self::note_seen(st, &v, now);
        Self::forward(&self.me, st, wire, &v.path, cost);
        Ok(learned)
    }

    /// An announcement whose `seq` is not the newest of its origin (T-335).
    /// Each link of the origin gets its own `seq`, so the path of a round
    /// through another next hop can come with the same or an older `seq`.
    /// The route table takes it when its next hop has no newer route of
    /// that origin: then the origin has a second live next hop. It is not
    /// forwarded (the newest `seq` was) and it does not change the status
    /// view. A replay from the same next hop stays [`Rule::SeenSeq`].
    fn take_other_path(
        st: &mut State,
        from: &PublicKey,
        cost: u32,
        v: &announce::Verified,
        now: u64,
    ) -> Result<Learned, Dropped> {
        match st.table.learn(*from, cost, &v.route(), now) {
            Ok(learned) => Ok(learned),
            Err(Refused::StaleSeq) => Err(Dropped::Announcement(Rule::SeenSeq)),
            Err(why) => Err(Dropped::Table(why)),
        }
    }

    /// Keep what an accepted announcement said, for the status view. The
    /// map holds the origins of the route table only: a full map drops the
    /// entries of origins that the table does not hold.
    fn note_seen(st: &mut State, v: &announce::Verified, now: u64) {
        if st.seen.len() >= 2 * crate::mesh_table::MAX_ORIGINS {
            let State { seen, table, .. } = st;
            let held: std::collections::HashSet<PublicKey> = table.origins().copied().collect();
            seen.retain(|k, _| held.contains(k));
        }
        st.seen.insert(
            v.body.origin,
            Seen {
                at_ms: now,
                services: v.body.services.clone(),
                neighbours: v.body.neighbours.iter().map(|n| (n.key, n.cost)).collect(),
            },
        );
    }

    /// Give a node a local name for the status view (1 to 64 characters, no
    /// control character; a bad name is not set).
    pub fn set_name(&self, node: PublicKey, name: &str) -> bool {
        let ok = !name.is_empty() && name.len() <= 64 && !name.chars().any(char::is_control);
        if ok {
            self.lock().names.insert(node, name.to_string());
        }
        ok
    }

    /// Send the announcement on with the hop of this node, to each neighbour
    /// that is not in the path. Each neighbour gets a hop that names it
    /// (T-346). A full outbox drops the frame: the origin sends again.
    fn forward(me: &Identity, st: &mut State, wire: &[u8], path: &[PublicKey], cost: u32) {
        if path.len() + 1 > st.hop_limit {
            return;
        }
        for (peer, tx) in &st.outbox {
            if path.contains(peer) {
                continue;
            }
            let Ok(out) = announce::append_hop(wire, me.key(), cost, peer) else {
                return;
            };
            if tx.try_send(out).is_ok() {
                st.forwarded += 1;
            }
        }
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
            "forwarded": st.forwarded,
            "limited": st.limited,
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
    mut forwards: mpsc::Receiver<Vec<u8>>,
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
    // The own announcement goes at once, then at each interval. A forwarded
    // announcement goes when it comes.
    let mut wait = Duration::ZERO;
    loop {
        let wire = tokio::select! {
            _ = tokio::time::sleep(wait) => {
                wait = next_delay(interval_ms, random_u64());
                mesh.set_cost(&peer, link_cost(weight, link.rtt()));
                match mesh.make(now_ms(), &peer) {
                    Ok(wire) => wire,
                    Err(err) => {
                        tracing::warn!(error = %err, "announce: no announcement");
                        return;
                    }
                }
            }
            Some(wire) = forwards.recv() => wire,
        };
        if write_frame(&mut stream, &wire).await.is_err() {
            return;
        }
    }
}

async fn write_frame(stream: &mut Stream, wire: &[u8]) -> std::io::Result<()> {
    let mut frame = Vec::with_capacity(wire.len() + 2);
    frame.extend_from_slice(&(wire.len() as u16).to_be_bytes());
    frame.extend_from_slice(wire);
    stream.write_all(&frame).await
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
    // A peer of another version gets one line for each stream (T-346).
    let mut version_told = false;
    let (rate, burst) = mesh.check_rate();
    let mut bucket = limit::Bucket::new(rate, burst, std::time::Instant::now());
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
        let at = std::time::Instant::now();
        if !bucket.take(announce::signature_checks(&wire), at) {
            mesh.note_limited();
            if bucket.tell(at) {
                tracing::warn!(
                    peer = %peer.display_fingerprint(),
                    "announce: the peer sends more than {rate} signature checks each second: \
                     frames are dropped"
                );
            }
            continue;
        }
        let cost = link_cost(weight, link.rtt());
        match mesh.receive(&peer, cost, &wire, now_ms()) {
            Ok(_) => {}
            Err(why @ Dropped::Announcement(Rule::BadVersion(_))) => {
                if !version_told {
                    version_told = true;
                    tracing::warn!(
                        peer = %peer.display_fingerprint(),
                        "announce: dropped: {why}; each announcement of this peer is dropped"
                    );
                }
            }
            Err(why) => {
                tracing::warn!(peer = %peer.display_fingerprint(), "announce: dropped: {why}");
            }
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
        let (tx, rx) = mpsc::channel(OUTBOX);
        mesh.attach_link(peer, link.clone(), tx);
        let sender = tokio::spawn(run_sender(
            mesh.clone(),
            link.clone(),
            peer,
            weight,
            interval_ms,
            rx,
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
    serve_streams_with(mesh, link, peer, weight, crate::identity::Grant::open()).await;
}

/// [`serve_streams`] for a peer with the grant `grant` (T-290): the peer
/// has a stream relayed only when the grant says so.
pub async fn serve_streams_with(
    mesh: Arc<Mesh>,
    link: Arc<Link>,
    peer: PublicKey,
    weight: Option<u32>,
    grant: crate::identity::Grant,
) {
    while let Ok(mut stream) = link.accept().await {
        let (mesh, link, grant) = (mesh.clone(), link.clone(), grant.clone());
        tokio::spawn(async move {
            match service::accept(&mut stream).await {
                Ok(name) if name == NAME => {
                    receive_stream(mesh, link, peer, weight, &mut stream).await;
                }
                Ok(name) if name == crate::link_forward::NAME => {
                    // This end has no service: it relays, or it refuses.
                    if let Ok(crate::link_forward::Arrived::Local(_)) =
                        crate::link_forward::accept_with(
                            &mesh,
                            &mut stream,
                            grant.may_forward(),
                            || {},
                        )
                        .await
                    {
                        let _ = service::reply(&mut stream, service::NO_SERVICE).await;
                    }
                }
                Ok(_) => {
                    let _ = service::reply(&mut stream, service::NO_SERVICE).await;
                }
                Err(_) => {}
            }
        });
    }
}
