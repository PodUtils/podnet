//! The path-vector route table of the link mesh (T-286).
//!
//! The table is a pure data structure. It does no I/O and reads no clock:
//! each call that depends on time takes `now` in milliseconds from the
//! caller. The caller checks the signature of an announcement (T-285)
//! before it calls [`RouteTable::learn`].
//!
//! # Model
//!
//! * The table is keyed by the origin: the key of the node that made the
//!   announcement. An origin has one route for each next hop.
//! * An [`Announcement`] holds the origin, a `seq`, the cost from the sender
//!   to the origin, the `path` and the expiry. The path lists the nodes that
//!   the announcement passed: the origin first, the sender last. A direct
//!   neighbour sends the path `[origin]`.
//! * The cost of a route is the cost of the link to the sender plus the cost
//!   in the announcement. The hop count is the length of the path.
//!
//! # Rules
//!
//! * Selection ([`RouteTable::best`]): a fresh route comes before a held
//!   route. Then the lowest cost, then the fewest hops, then the newest
//!   `seq`.
//! * A path that holds this node, or holds a node two times, is refused
//!   (loop). A path of more than [`Limits::max_hops`] nodes is refused.
//! * An announcement with a `seq` that is not above the `seq` of the route
//!   of the same next hop, or below the newest `seq` of the origin, is
//!   refused.
//! * A direct route ends at its expiry. A multi-hop route is held for one
//!   expiry interval after its expiry and ends then. A held route is used
//!   only when no fresh route exists.
//! * [`RouteTable::session_closed`] ends the direct route of that peer at
//!   once. The multi-hop routes through that peer cannot be used from then
//!   on, and end after one expiry interval, unless the peer announces them
//!   again.
//! * The table holds [`Limits::origins`] origins. Eviction of the worst
//!   origin starts at 90% of the limit and is counted
//!   ([`RouteTable::evictions`]). A direct route is not evicted.

use std::collections::HashMap;
use std::fmt;

use pod_identity::PublicKey;

/// The most origins of a table.
pub const MAX_ORIGINS: usize = 4096;

/// The most nodes in a path.
pub const MAX_HOPS: usize = 16;

/// The expiry of an announcement: 3 intervals of 10 s.
pub const EXPIRY_MS: u64 = 30_000;

/// The most routes (next hops) that one origin keeps.
pub const MAX_ROUTES_PER_ORIGIN: usize = 8;

/// The limits of a table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// The most origins.
    pub origins: usize,
    /// The most nodes in a path.
    pub max_hops: usize,
    /// The expiry interval in milliseconds. It is also the hold time of a
    /// multi-hop route.
    pub expiry_ms: u64,
    /// The most routes of one origin.
    pub routes_per_origin: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            origins: MAX_ORIGINS,
            max_hops: MAX_HOPS,
            expiry_ms: EXPIRY_MS,
            routes_per_origin: MAX_ROUTES_PER_ORIGIN,
        }
    }
}

impl Limits {
    /// The origin count at which eviction starts: 90% of the limit.
    pub fn high_water(&self) -> usize {
        self.origins * 9 / 10
    }
}

/// An announcement that the caller has verified.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Announcement {
    /// The node that made the announcement.
    pub origin: PublicKey,
    /// The sequence number of the origin. It rises strictly.
    pub seq: u64,
    /// The cost from the sender to the origin. It is 0 for a direct
    /// neighbour.
    pub cost: u32,
    /// The nodes that the announcement passed: the origin first, the sender
    /// last.
    pub path: Vec<PublicKey>,
    /// The time (ms) when the announcement expires.
    pub expires_ms: u64,
}

/// One route of the table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Route {
    /// The node that the route leads to.
    pub origin: PublicKey,
    /// The peer to send to: the sender of the announcement.
    pub next_hop: PublicKey,
    /// The cost of the link to the next hop plus the cost to the origin.
    pub cost: u32,
    /// The number of nodes in the path.
    pub hops: usize,
    /// The sequence number of the announcement.
    pub seq: u64,
    /// The time (ms) when the announcement expires.
    pub expires_ms: u64,
    /// The path: the origin first, the next hop last.
    pub path: Vec<PublicKey>,
    /// The time (ms) when the session of the next hop closed.
    pub down_since_ms: Option<u64>,
}

impl Route {
    /// Whether the route leads to a direct neighbour.
    pub fn is_direct(&self) -> bool {
        self.hops == 1
    }

    /// The time (ms) when the route ends, with `interval` the hold time.
    fn ends_ms(&self, interval: u64) -> u64 {
        let hold = if self.is_direct() { 0 } else { interval };
        let by_expiry = self.expires_ms.saturating_add(hold);
        match self.down_since_ms {
            Some(t) => by_expiry.min(t.saturating_add(interval)),
            None => by_expiry,
        }
    }

    fn alive(&self, now: u64, interval: u64) -> bool {
        now < self.ends_ms(interval)
    }

    /// Whether the route is used: its session is up and it is alive.
    fn usable(&self, now: u64, interval: u64) -> bool {
        self.down_since_ms.is_none() && self.alive(now, interval)
    }

    /// Whether the announcement of the route has expired (the route is held).
    pub fn held(&self, now: u64) -> bool {
        now >= self.expires_ms
    }

    /// The rank of the route: a lower value is better.
    fn rank(&self, now: u64) -> (bool, u32, usize, std::cmp::Reverse<u64>) {
        (
            self.held(now),
            self.cost,
            self.hops,
            std::cmp::Reverse(self.seq),
        )
    }
}

/// What [`RouteTable::learn`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Learned {
    /// The origin was new to the table.
    New,
    /// The origin was known. The route is new or replaces an older one.
    Updated,
}

/// Why [`RouteTable::learn`] refused an announcement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// The origin is this node.
    OwnOrigin,
    /// The path is empty, does not start at the origin, or does not end at
    /// the sender.
    BadPath,
    /// The path holds this node, or one node two times.
    Loop,
    /// The path has more nodes than the limit.
    TooManyHops,
    /// The announcement expired already.
    Expired,
    /// The `seq` is not above the `seq` that is known.
    StaleSeq,
    /// The table is full of direct routes, or the origin has its most routes
    /// and all of them are better.
    Full,
}

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Refused::OwnOrigin => "the origin is this node",
            Refused::BadPath => "the path does not run from the origin to the sender",
            Refused::Loop => "the path holds this node or a node two times",
            Refused::TooManyHops => "the path has too many hops",
            Refused::Expired => "the announcement has expired",
            Refused::StaleSeq => "the seq is not new",
            Refused::Full => "the route table is full",
        })
    }
}

impl std::error::Error for Refused {}

#[derive(Debug, Default)]
struct Entry {
    /// The newest `seq` that this origin showed.
    max_seq: u64,
    routes: Vec<Route>,
}

/// The route table.
#[derive(Debug)]
pub struct RouteTable {
    me: PublicKey,
    limits: Limits,
    origins: HashMap<PublicKey, Entry>,
    evictions: u64,
}

impl RouteTable {
    /// An empty table of the node `me`.
    pub fn new(me: PublicKey, limits: Limits) -> Self {
        RouteTable {
            me,
            limits,
            origins: HashMap::new(),
            evictions: 0,
        }
    }

    /// The limits.
    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    /// The number of origins.
    pub fn len(&self) -> usize {
        self.origins.len()
    }

    /// Whether the table has no origin.
    pub fn is_empty(&self) -> bool {
        self.origins.is_empty()
    }

    /// The number of origins that eviction removed.
    pub fn evictions(&self) -> u64 {
        self.evictions
    }

    /// The origins in the table, in no order.
    pub fn origins(&self) -> impl Iterator<Item = &PublicKey> {
        self.origins.keys()
    }

    /// Take an announcement that arrived from the peer `from`. `link_cost`
    /// is the cost of the link to `from`. The caller has checked the
    /// signature and the skew.
    pub fn learn(
        &mut self,
        from: PublicKey,
        link_cost: u32,
        ann: &Announcement,
        now: u64,
    ) -> Result<Learned, Refused> {
        if ann.origin == self.me {
            return Err(Refused::OwnOrigin);
        }
        if ann.path.first() != Some(&ann.origin) || ann.path.last() != Some(&from) {
            return Err(Refused::BadPath);
        }
        if ann.path.len() > self.limits.max_hops {
            return Err(Refused::TooManyHops);
        }
        if ann.path.contains(&self.me) || has_duplicate(&ann.path) {
            return Err(Refused::Loop);
        }
        if ann.expires_ms <= now {
            return Err(Refused::Expired);
        }
        let route = Route {
            origin: ann.origin,
            next_hop: from,
            cost: link_cost.saturating_add(ann.cost),
            hops: ann.path.len(),
            seq: ann.seq,
            expires_ms: ann.expires_ms,
            path: ann.path.clone(),
            down_since_ms: None,
        };
        let interval = self.limits.expiry_ms;
        let known = match self.origins.get_mut(&ann.origin) {
            Some(entry) => {
                entry.routes.retain(|r| r.alive(now, interval));
                if entry.routes.is_empty() {
                    self.origins.remove(&ann.origin);
                    false
                } else {
                    true
                }
            }
            None => false,
        };
        if !known {
            self.make_room(now)?;
            let entry = Entry {
                max_seq: route.seq,
                routes: vec![route],
            };
            self.origins.insert(ann.origin, entry);
            return Ok(Learned::New);
        }
        let per_origin = self.limits.routes_per_origin;
        let entry = self.origins.get_mut(&ann.origin).expect("known origin");
        if ann.seq < entry.max_seq {
            return Err(Refused::StaleSeq);
        }
        if let Some(slot) = entry.routes.iter_mut().find(|r| r.next_hop == from) {
            if ann.seq <= slot.seq {
                return Err(Refused::StaleSeq);
            }
            *slot = route;
        } else if entry.routes.len() < per_origin.max(1) {
            entry.routes.push(route);
        } else {
            // Replace the worst route, if the new one is better.
            let worst = (0..entry.routes.len())
                .max_by_key(|&i| entry.routes[i].rank(now))
                .expect("a route");
            if route.rank(now) < entry.routes[worst].rank(now) {
                entry.routes[worst] = route;
            } else {
                return Err(Refused::Full);
            }
        }
        entry.max_seq = entry.max_seq.max(ann.seq);
        Ok(Learned::Updated)
    }

    /// The best route to `origin`, or `None`.
    pub fn best(&self, origin: &PublicKey, now: u64) -> Option<&Route> {
        self.ranked(origin, now).into_iter().next()
    }

    /// The usable routes to `origin`, best first. A forwarder takes the next
    /// one when the best one has no stream to give (T-287).
    pub fn ranked(&self, origin: &PublicKey, now: u64) -> Vec<&Route> {
        let interval = self.limits.expiry_ms;
        let Some(entry) = self.origins.get(origin) else {
            return Vec::new();
        };
        let mut routes: Vec<&Route> = entry
            .routes
            .iter()
            .filter(|r| r.usable(now, interval))
            .collect();
        routes.sort_by_key(|r| r.rank(now));
        routes
    }

    /// The session of `peer` closed. End its direct route at once and mark
    /// the routes through it as down. Return the number of routes that ended
    /// at once.
    pub fn session_closed(&mut self, peer: &PublicKey, now: u64) -> usize {
        let interval = self.limits.expiry_ms;
        let mut ended = 0;
        for entry in self.origins.values_mut() {
            let before = entry.routes.len();
            entry
                .routes
                .retain(|r| !(r.next_hop == *peer && r.is_direct()));
            ended += before - entry.routes.len();
            for r in entry.routes.iter_mut().filter(|r| r.next_hop == *peer) {
                r.down_since_ms.get_or_insert(now);
            }
        }
        self.origins
            .retain(|_, e| e.routes.iter().any(|r| r.alive(now, interval)));
        ended
    }

    /// Remove each route that ended, and each origin with no route. Return
    /// the number of routes removed. A read gives the same answer without
    /// it: a call is for memory only.
    pub fn purge(&mut self, now: u64) -> usize {
        let interval = self.limits.expiry_ms;
        let mut removed = 0;
        for entry in self.origins.values_mut() {
            let before = entry.routes.len();
            entry.routes.retain(|r| r.alive(now, interval));
            removed += before - entry.routes.len();
        }
        self.origins.retain(|_, e| !e.routes.is_empty());
        removed
    }

    /// Make room for one new origin. At the high water mark, first drop the
    /// routes that ended, then evict the worst origin.
    fn make_room(&mut self, now: u64) -> Result<(), Refused> {
        let high = self.limits.high_water().max(1);
        if self.origins.len() < high {
            return Ok(());
        }
        self.purge(now);
        while self.origins.len() >= high {
            let victim = self
                .origins
                .iter()
                .filter(|(_, e)| !e.routes.iter().any(Route::is_direct))
                .map(|(k, e)| (*k, worst_rank(e, now)))
                .max_by_key(|(_, rank)| *rank)
                .map(|(k, _)| k);
            match victim {
                Some(key) => {
                    self.origins.remove(&key);
                    self.evictions += 1;
                }
                None => {
                    // Only direct routes remain. Keep them.
                    return Err(Refused::Full);
                }
            }
        }
        Ok(())
    }
}

/// The rank of the best route of an origin, as the value of its worth to
/// keep: a larger value is a worse origin.
fn worst_rank(e: &Entry, now: u64) -> (bool, u32, usize, std::cmp::Reverse<u64>) {
    e.routes.iter().map(|r| r.rank(now)).min().unwrap_or((
        true,
        u32::MAX,
        usize::MAX,
        std::cmp::Reverse(0),
    ))
}

fn has_duplicate(path: &[PublicKey]) -> bool {
    (1..path.len()).any(|i| path[..i].contains(&path[i]))
}
