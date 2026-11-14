//! The signed route announcement of the link mesh (T-285).
//!
//! All functions are pure. They do no I/O and read no clock: the caller
//! gives the time in milliseconds. A verified announcement goes to the route
//! table ([`crate::mesh_table`]) through [`Verified::route`].
//!
//! # Wire format
//!
//! All integers are big-endian. The format is deterministic: one value has
//! one encoding, and the decoder refuses any other encoding.
//!
//! ```text
//! body (signed):
//!   u8        version (1)
//!   [u8; 32]  origin: the Ed25519 key of the announcing node
//!   u64       seq: it rises strictly for one origin
//!   u64       issued_ms: the time of the origin when it signed
//!   u64       expires_ms: the time when the announcement ends
//!   u16       neighbour count (at most 64)
//!     [u8; 32] key, u32 cost (ms or weight)   (strictly ascending by key)
//!   u16       service count (at most 16)
//!     u8 length (1 to 64), the UTF-8 name     (strictly ascending by bytes)
//! [u8; 64]    signature of the label and the body
//! path (the hop chain):
//!   u8        path length n (1 to 16)
//!   [u8; 32]  key of the origin
//!   n - 1 times, one for each forwarder (the sender is the last):
//!     [u8; 32] key of the forwarder
//!     u32      cost of the link of the forwarder to the node before it
//!     [u8; 64] signature of the hop
//! ```
//!
//! The origin signature covers the label [`LABEL`] and the body. The
//! signature of a hop covers the label [`HOP_LABEL`], the signature before
//! it (the origin signature for the first hop), the key of the forwarder and
//! its cost. The signatures form a chain: a hop cannot be dropped from the
//! middle, a key cannot be changed and a cost cannot be lowered, because the
//! next signature covers the one before it. A forwarder vouches only for
//! itself: it can cut the chain at its own hop. The session with the sender
//! authenticates the last node of the path.
//!
//! # Receive rules
//!
//! [`verify`] drops an announcement for the first rule that fails, in this
//! order: size and shape, the path (it must start at the origin, hold no node
//! two times and not hold this node; an own origin is a path with this node), the clock skew, the expiry, the
//! signature. [`Receiver::accept`] adds the `seq` rule and keeps the newest
//! `seq` of each origin (bounded, see [`MAX_SEEN`]).
//!
//! Only an announcement with the path `[origin]` is a neighbour claim
//! ([`Verified::neighbour_claims`]). A multi-hop announcement tells
//! reachability, not adjacency.

use std::collections::HashMap;
use std::fmt;

use pod_identity::{Identity, PublicKey};

use crate::mesh_table;

/// The version of the encoding.
pub const VERSION: u8 = 1;

/// The label that starts the signed bytes. No other signature of the key
/// uses it.
pub const LABEL: &[u8] = b"podnet-mesh-announce-v1\0";

/// The label that starts the signed bytes of a hop.
pub const HOP_LABEL: &[u8] = b"podnet-mesh-hop-v1\0";

/// The most neighbours of one announcement.
pub const MAX_NEIGHBOURS: usize = 64;
/// The most services of one announcement.
pub const MAX_SERVICES: usize = 16;
/// The most bytes of the name of a service.
pub const MAX_SERVICE_NAME: usize = 64;
/// The most nodes of a path.
pub const MAX_PATH: usize = 16;
/// The most bytes of a wire announcement: the body, the signature and a
/// path of [`MAX_PATH`] nodes.
pub const MAX_WIRE: usize = MAX_BODY + 64 + 1 + 32 + (MAX_PATH - 1) * (32 + 4 + 64);
/// The most bytes of the body (before the signature).
pub const MAX_BODY: usize = 8 * 1024;
/// The interval of announcements in milliseconds. The sender adds jitter.
pub const INTERVAL_MS: u64 = 10_000;
/// The life of an announcement: 3 intervals.
pub const LIFETIME_MS: u64 = 3 * INTERVAL_MS;
/// The clock skew that a receiver accepts, in each direction.
pub const SKEW_MS: u64 = 120_000;
/// The most origins that a [`Receiver`] keeps a `seq` for.
pub const MAX_SEEN: usize = 4096;

/// One neighbour of the origin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Neighbour {
    /// The key of the neighbour.
    pub key: PublicKey,
    /// The measured RTT in milliseconds, or a configured weight.
    pub cost: u32,
}

/// The content that an origin signs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Body {
    /// The announcing node.
    pub origin: PublicKey,
    /// The sequence number. It rises strictly.
    pub seq: u64,
    /// The time (ms) when the origin signed.
    pub issued_ms: u64,
    /// The time (ms) when the announcement ends.
    pub expires_ms: u64,
    /// The direct neighbours of the origin.
    pub neighbours: Vec<Neighbour>,
    /// The names of the services that the origin offers.
    pub services: Vec<String>,
}

/// A verified announcement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verified {
    /// The signed content.
    pub body: Body,
    /// The path: the origin first, the sender last.
    pub path: Vec<PublicKey>,
    /// The signed cost of each node of the path to the node before it. The
    /// origin has 0.
    pub costs: Vec<u32>,
}

/// Why an announcement was dropped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Drop {
    /// The bytes end before the announcement ends.
    Truncated,
    /// Bytes remain after the announcement.
    Trailing,
    /// The version is not 1.
    BadVersion,
    /// The body is longer than 8 KiB.
    BodyTooLarge,
    /// More than 64 neighbours.
    TooManyNeighbours,
    /// More than 16 services.
    TooManyServices,
    /// A service name is empty, is longer than 64 bytes or is not UTF-8
    /// without control characters.
    BadServiceName,
    /// The neighbours or the services are not in the sorted order without
    /// repeats (the encoding would not be unique).
    NotCanonical,
    /// The path is empty, or longer than 16 nodes.
    BadPathLength,
    /// The path does not start at the origin.
    PathNotFromOrigin,
    /// The path holds a node two times.
    PathRepeats,
    /// The path holds this node.
    PathHasSelf,
    /// The time of issue is more than 120 s ahead of the clock.
    FromFuture,
    /// The time of issue is more than 120 s behind the clock.
    TooOld,
    /// The expiry is not after the time of issue, or the life is more than
    /// 3 intervals.
    BadLifetime,
    /// The announcement has expired.
    Expired,
    /// The signature does not match the origin and the body.
    BadSignature,
    /// The signature of a hop does not match its key, its cost and the
    /// signature before it.
    BadHopSignature,
    /// The `seq` is not above the newest `seq` of the origin.
    SeenSeq,
}

impl fmt::Display for Drop {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Drop::Truncated => "the announcement is cut short",
            Drop::Trailing => "bytes remain after the announcement",
            Drop::BadVersion => "the version is not known",
            Drop::BodyTooLarge => "the body is larger than 8 KiB",
            Drop::TooManyNeighbours => "more than 64 neighbours",
            Drop::TooManyServices => "more than 16 services",
            Drop::BadServiceName => "a service name is not valid",
            Drop::NotCanonical => "the encoding is not the canonical one",
            Drop::BadPathLength => "the path is empty or has more than 16 nodes",
            Drop::PathNotFromOrigin => "the path does not start at the origin",
            Drop::PathRepeats => "the path holds a node two times",
            Drop::PathHasSelf => "the path holds this node",
            Drop::FromFuture => "the announcement is from the future",
            Drop::TooOld => "the announcement is too old",
            Drop::BadLifetime => "the lifetime is not valid",
            Drop::Expired => "the announcement has expired",
            Drop::BadSignature => "the signature is not valid",
            Drop::BadHopSignature => "the signature of a hop is not valid",
            Drop::SeenSeq => "the seq is not new",
        })
    }
}

impl std::error::Error for Drop {}

/// Why [`encode`] refused a body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EncodeError {
    /// More than 64 neighbours.
    TooManyNeighbours,
    /// More than 16 services.
    TooManyServices,
    /// A service name is empty, longer than 64 bytes or has a control
    /// character.
    BadServiceName,
    /// A neighbour or a service is listed two times.
    Repeat,
}

impl fmt::Display for EncodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            EncodeError::TooManyNeighbours => "more than 64 neighbours",
            EncodeError::TooManyServices => "more than 16 services",
            EncodeError::BadServiceName => "a service name is not valid",
            EncodeError::Repeat => "a neighbour or a service is listed two times",
        })
    }
}

impl std::error::Error for EncodeError {}

fn name_ok(name: &str) -> bool {
    !name.is_empty() && name.len() <= MAX_SERVICE_NAME && !name.chars().any(char::is_control)
}

/// The signed bytes of a body: the label, then the body.
fn signed_bytes(body: &[u8]) -> Vec<u8> {
    [LABEL, body].concat()
}

/// The canonical bytes of a body. The neighbours and the services are sorted.
pub fn encode_body(body: &Body) -> Result<Vec<u8>, EncodeError> {
    if body.neighbours.len() > MAX_NEIGHBOURS {
        return Err(EncodeError::TooManyNeighbours);
    }
    if body.services.len() > MAX_SERVICES {
        return Err(EncodeError::TooManyServices);
    }
    if !body.services.iter().all(|s| name_ok(s)) {
        return Err(EncodeError::BadServiceName);
    }
    let mut neighbours: Vec<&Neighbour> = body.neighbours.iter().collect();
    neighbours.sort_by_key(|n| n.key.0);
    if neighbours.windows(2).any(|w| w[0].key == w[1].key) {
        return Err(EncodeError::Repeat);
    }
    let mut services: Vec<&String> = body.services.iter().collect();
    services.sort();
    if services.windows(2).any(|w| w[0] == w[1]) {
        return Err(EncodeError::Repeat);
    }
    let mut out = vec![VERSION];
    out.extend_from_slice(&body.origin.0);
    out.extend_from_slice(&body.seq.to_be_bytes());
    out.extend_from_slice(&body.issued_ms.to_be_bytes());
    out.extend_from_slice(&body.expires_ms.to_be_bytes());
    out.extend_from_slice(&(neighbours.len() as u16).to_be_bytes());
    for n in neighbours {
        out.extend_from_slice(&n.key.0);
        out.extend_from_slice(&n.cost.to_be_bytes());
    }
    out.extend_from_slice(&(services.len() as u16).to_be_bytes());
    for s in services {
        out.push(s.len() as u8);
        out.extend_from_slice(s.as_bytes());
    }
    Ok(out)
}

/// The signed bytes of a hop: the label, the signature before it, the key
/// of the forwarder and its cost.
fn hop_bytes(prev: &[u8; 64], key: &PublicKey, cost: u32) -> Vec<u8> {
    let mut out = HOP_LABEL.to_vec();
    out.extend_from_slice(prev);
    out.extend_from_slice(&key.0);
    out.extend_from_slice(&cost.to_be_bytes());
    out
}

/// Sign `body` with `identity` and write the wire bytes of an announcement
/// that has not passed a forwarder: the path is `[origin]`. The origin of the
/// body should be the key of `identity` (else [`verify`] drops it with
/// [`Drop::BadSignature`]).
pub fn encode(identity: &Identity, body: &Body) -> Result<Vec<u8>, EncodeError> {
    let bytes = encode_body(body)?;
    let signature = identity.sign(&signed_bytes(&bytes));
    let mut out = bytes;
    out.extend_from_slice(&signature);
    out.push(1);
    out.extend_from_slice(&body.origin.0);
    Ok(out)
}

/// Add the forwarder `identity` to the end of the path of an encoded
/// announcement. `cost` is the cost of the link of the forwarder to the
/// node that sent the announcement to it. The hop signs the key, the cost
/// and the signature before it. The caller has verified the announcement
/// first. A path of 16 nodes is full ([`Drop::BadPathLength`]); a forwarder
/// that is in the path already gets [`Drop::PathRepeats`].
pub fn append_hop(wire: &[u8], identity: &Identity, cost: u32) -> Result<Vec<u8>, Drop> {
    let parsed = parse(wire)?;
    if parsed.path.len() >= MAX_PATH {
        return Err(Drop::BadPathLength);
    }
    let key = identity.public();
    if parsed.path.contains(&key) {
        return Err(Drop::PathRepeats);
    }
    let prev = parsed.hops.last().map_or(&parsed.signature, |h| &h.2);
    let signature = identity.sign(&hop_bytes(prev, &key, cost));
    let mut out = wire[..parsed.path_at].to_vec();
    out.push((parsed.path.len() + 1) as u8);
    out.extend_from_slice(&wire[parsed.path_at + 1..]);
    out.extend_from_slice(&key.0);
    out.extend_from_slice(&cost.to_be_bytes());
    out.extend_from_slice(&signature);
    Ok(out)
}

struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], Drop> {
        let end = self.at.checked_add(n).ok_or(Drop::Truncated)?;
        let s = self.data.get(self.at..end).ok_or(Drop::Truncated)?;
        self.at = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, Drop> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, Drop> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, Drop> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, Drop> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn key(&mut self) -> Result<PublicKey, Drop> {
        Ok(PublicKey(self.take(32)?.try_into().unwrap()))
    }
}

struct Parsed {
    body: Body,
    /// The length of the signed body.
    body_len: usize,
    signature: [u8; 64],
    /// The offset of the path length byte.
    path_at: usize,
    path: Vec<PublicKey>,
    /// The forwarders: key, cost, signature.
    hops: Vec<(PublicKey, u32, [u8; 64])>,
}

fn parse(wire: &[u8]) -> Result<Parsed, Drop> {
    let mut r = Reader { data: wire, at: 0 };
    if r.u8()? != VERSION {
        return Err(Drop::BadVersion);
    }
    let origin = r.key()?;
    let seq = r.u64()?;
    let issued_ms = r.u64()?;
    let expires_ms = r.u64()?;
    let count = r.u16()? as usize;
    if count > MAX_NEIGHBOURS {
        return Err(Drop::TooManyNeighbours);
    }
    let mut neighbours: Vec<Neighbour> = Vec::with_capacity(count);
    for _ in 0..count {
        let key = r.key()?;
        let cost = r.u32()?;
        if neighbours.last().is_some_and(|p| p.key.0 >= key.0) {
            return Err(Drop::NotCanonical);
        }
        neighbours.push(Neighbour { key, cost });
    }
    let count = r.u16()? as usize;
    if count > MAX_SERVICES {
        return Err(Drop::TooManyServices);
    }
    let mut services: Vec<String> = Vec::with_capacity(count);
    for _ in 0..count {
        let len = r.u8()? as usize;
        let raw = r.take(len)?;
        let name = std::str::from_utf8(raw).map_err(|_| Drop::BadServiceName)?;
        if !name_ok(name) {
            return Err(Drop::BadServiceName);
        }
        if services.last().is_some_and(|p| p.as_str() >= name) {
            return Err(Drop::NotCanonical);
        }
        services.push(name.to_string());
    }
    let body_len = r.at;
    if body_len > MAX_BODY {
        return Err(Drop::BodyTooLarge);
    }
    let signature: [u8; 64] = r.take(64)?.try_into().unwrap();
    let path_at = r.at;
    let n = r.u8()? as usize;
    if n == 0 || n > MAX_PATH {
        return Err(Drop::BadPathLength);
    }
    let mut path = Vec::with_capacity(n);
    let mut hops = Vec::with_capacity(n - 1);
    path.push(r.key()?);
    for _ in 1..n {
        let key = r.key()?;
        let cost = r.u32()?;
        let sig: [u8; 64] = r.take(64)?.try_into().unwrap();
        path.push(key);
        hops.push((key, cost, sig));
    }
    if r.at != wire.len() {
        return Err(Drop::Trailing);
    }
    Ok(Parsed {
        body: Body {
            origin,
            seq,
            issued_ms,
            expires_ms,
            neighbours,
            services,
        },
        body_len,
        signature,
        path_at,
        path,
        hops,
    })
}

/// Check an announcement received at time `now` (ms) by the node `me`.
///
/// This does not check the `seq`: use [`Receiver::accept`] for that.
pub fn verify(wire: &[u8], me: &PublicKey, now: u64) -> Result<Verified, Drop> {
    // The body limit holds before any parse work that grows with the input.
    if wire.len() > MAX_WIRE {
        return Err(Drop::BodyTooLarge);
    }
    let p = parse(wire)?;
    let origin = p.body.origin;
    if p.path[0] != origin {
        return Err(Drop::PathNotFromOrigin);
    }
    for (i, node) in p.path.iter().enumerate() {
        if p.path[..i].contains(node) {
            return Err(Drop::PathRepeats);
        }
    }
    if p.path.contains(me) {
        return Err(Drop::PathHasSelf);
    }
    if p.body.issued_ms > now.saturating_add(SKEW_MS) {
        return Err(Drop::FromFuture);
    }
    if p.body.issued_ms.saturating_add(SKEW_MS) < now {
        return Err(Drop::TooOld);
    }
    if p.body.expires_ms <= p.body.issued_ms || p.body.expires_ms - p.body.issued_ms > LIFETIME_MS {
        return Err(Drop::BadLifetime);
    }
    if p.body.expires_ms <= now {
        return Err(Drop::Expired);
    }
    if !origin.verifies(&signed_bytes(&wire[..p.body_len]), &p.signature) {
        return Err(Drop::BadSignature);
    }
    let mut prev = p.signature;
    let mut costs = vec![0];
    for (key, cost, sig) in &p.hops {
        if !key.verifies(&hop_bytes(&prev, key, *cost), sig) {
            return Err(Drop::BadHopSignature);
        }
        prev = *sig;
        costs.push(*cost);
    }
    Ok(Verified {
        body: p.body,
        path: p.path,
        costs,
    })
}

impl Verified {
    /// The neighbours that the origin claims, only when the announcement
    /// came direct from the origin (path `[origin]`). A multi-hop
    /// announcement is not a claim of adjacency.
    pub fn neighbour_claims(&self) -> Option<&[Neighbour]> {
        (self.path.len() == 1).then_some(self.body.neighbours.as_slice())
    }

    /// The cost from the sender to the origin: the sum of the signed costs
    /// of the hops. It is 0 for a direct neighbour.
    pub fn path_cost(&self) -> u32 {
        self.costs.iter().fold(0u32, |a, c| a.saturating_add(*c))
    }

    /// The announcement for the route table, with the signed cost of the
    /// path ([`Verified::path_cost`]).
    pub fn route(&self) -> mesh_table::Announcement {
        mesh_table::Announcement {
            origin: self.body.origin,
            seq: self.body.seq,
            cost: self.path_cost(),
            path: self.path.clone(),
            expires_ms: self.body.expires_ms,
        }
    }
}

/// The receiver: [`verify`] and the newest `seq` of each origin.
///
/// The `seq` of an origin is kept while a replay of it could pass the skew
/// check: until `issued_ms + SKEW_MS`. After that, an old announcement fails
/// [`Drop::TooOld`] and the entry is removed. When [`MAX_SEEN`] origins are
/// held, the entry with the oldest time of issue is removed. A replay of
/// that origin is then possible until its skew window ends.
#[derive(Debug)]
pub struct Receiver {
    me: PublicKey,
    cap: usize,
    /// Origin to (newest seq, issued_ms of that announcement).
    seen: HashMap<PublicKey, (u64, u64)>,
}

impl Receiver {
    /// A receiver for the node `me`.
    pub fn new(me: PublicKey) -> Self {
        Receiver::with_capacity(me, MAX_SEEN)
    }

    /// A receiver that keeps the `seq` of `cap` origins at most (for tests).
    pub fn with_capacity(me: PublicKey, cap: usize) -> Self {
        Receiver {
            me,
            cap: cap.max(1),
            seen: HashMap::new(),
        }
    }

    /// The number of origins with a kept `seq`.
    pub fn len(&self) -> usize {
        self.seen.len()
    }

    /// Whether no `seq` is kept.
    pub fn is_empty(&self) -> bool {
        self.seen.is_empty()
    }

    /// The newest `seq` that was accepted for `origin`.
    pub fn seq_of(&self, origin: &PublicKey) -> Option<u64> {
        self.seen.get(origin).map(|e| e.0)
    }

    /// Verify the announcement and check its `seq`. On success the `seq` is
    /// kept. A dropped announcement changes nothing.
    pub fn accept(&mut self, wire: &[u8], now: u64) -> Result<Verified, Drop> {
        let v = verify(wire, &self.me, now)?;
        self.note(v, now)
    }

    /// The `seq` rule for an announcement that [`verify`] passed. On
    /// success the `seq` is kept. A caller that has more rules than
    /// [`verify`] runs them before this call, so that a dropped
    /// announcement does not use the `seq` of its origin.
    pub fn note(&mut self, v: Verified, now: u64) -> Result<Verified, Drop> {
        let origin = v.body.origin;
        if self.seen.get(&origin).is_some_and(|e| v.body.seq <= e.0) {
            return Err(Drop::SeenSeq);
        }
        if !self.seen.contains_key(&origin) && self.seen.len() >= self.cap {
            self.seen.retain(|_, e| e.1.saturating_add(SKEW_MS) >= now);
            if self.seen.len() >= self.cap {
                let oldest = self.seen.iter().min_by_key(|(_, e)| e.1).map(|(k, _)| *k);
                if let Some(k) = oldest {
                    self.seen.remove(&k);
                }
            }
        }
        self.seen.insert(origin, (v.body.seq, v.body.issued_ms));
        Ok(v)
    }
}

/// The fuzz target `mesh_announce` (`crate::fuzzing`): untrusted wire bytes.
///
/// * [`verify`] and [`append_hop`] never panic.
/// * A wire that parses has one encoding: [`encode_body`] of its body gives
///   its body bytes again.
/// * The body of a wire that parses, signed again by an own origin, passes
///   [`verify`] at its time of issue when its lifetime is valid, also after
///   one hop. The path of the hop is the origin and the forwarder.
pub(crate) fn fuzz(wire: &[u8]) {
    let origin = Identity::from_seed(&[1u8; 32]);
    let forwarder = Identity::from_seed(&[2u8; 32]);
    let me = PublicKey([3u8; 32]);
    let Ok(parsed) = parse(wire) else {
        let _ = verify(wire, &me, 0);
        let _ = append_hop(wire, &forwarder, 1);
        return;
    };
    assert_eq!(parsed.path.len(), parsed.hops.len() + 1);
    let body = encode_body(&parsed.body).expect("a parsed body encodes");
    assert_eq!(body, wire[..parsed.body_len], "one value, one encoding");
    let now = parsed.body.issued_ms;
    let _ = verify(wire, &me, now);
    let mut own = parsed.body;
    own.origin = origin.public();
    let signed = encode(&origin, &own).expect("a parsed body encodes");
    let lifetime = own.expires_ms > own.issued_ms && own.expires_ms - own.issued_ms <= LIFETIME_MS;
    match verify(&signed, &me, now) {
        Ok(v) => {
            assert!(lifetime);
            assert_eq!(v.body, own);
            assert_eq!(v.path, vec![own.origin]);
        }
        Err(e) => assert!(!lifetime, "{e}"),
    }
    let hop = append_hop(&signed, &forwarder, 7).expect("one hop");
    if lifetime {
        let v = verify(&hop, &me, now).expect("a signed hop verifies");
        assert_eq!(v.path, vec![own.origin, forwarder.public()]);
        assert_eq!(v.path_cost(), 7);
    }
}
