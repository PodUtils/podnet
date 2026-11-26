//! The direction split of the TCP flows of a group (T-611, the design
//! `docs/research/podnet/direction-split-tcp.md`).
//!
//! The value `tcp=FINGERPRINT@UP|DOWN` of `splits.NAME` names a far host
//! that runs `podnet link listen 'UP|DOWN' --up-carrier A --down-carrier B
//! --service socks5`. The router dials one split session of the link to it
//! ([`crate::link_split`]): the up half goes through the way of the member
//! `up`, the down half through the way of the member `down`. Each TCP flow
//! of the group is one stream of that session, with a SOCKS5 `CONNECT` to
//! the service `socks5` of the far host ([`crate::link_door::Way`]). The
//! session puts the bytes of the flow on the halves by their direction.
//!
//! A lost half ends the session (the ruling of 2026-11-14). Each new TCP
//! flow then takes the pick of the policy, as with no split, and the
//! fallback is counted. The router dials a new session after a back-off
//! (1 s, then twice the last wait, at most 30 s). The datagrams of the
//! group keep the split of `up=` and `down=` ([`Balance`]).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use anyhow::{anyhow, Result};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::time::Instant;

use crate::carriers::Options;
use crate::identity::{AllowList, Identity};
use crate::link::{Link, Settings};
use crate::link_door::{self, Resolve};
use crate::link_keyed::Tune;
use crate::link_split;
use crate::network::{
    parse_group, Balance, BoxFuture, BoxedIo, Connected, Datagrams, Dial, Failure, Target,
};

/// The carrier of a half when the value names none.
pub const DEFAULT_CARRIER: &str = "tcp";

/// The first wait before a new session, and the longest.
pub const BACKOFF: (Duration, Duration) = (Duration::from_secs(1), Duration::from_secs(30));

/// The far end of a TCP split: `tcp=FINGERPRINT@UP|DOWN` and
/// `carriers=UP|DOWN` of a `splits` value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spec {
    /// The key of the far host, as `podnet link status` shows it.
    pub far: String,
    /// The address of the up half, with no key.
    pub up: String,
    /// The address of the down half, with no key.
    pub down: String,
    pub up_carrier: String,
    pub down_carrier: String,
}

impl Spec {
    /// The dial address of the session: `KEY@UP|KEY@DOWN`.
    pub fn address(&self) -> String {
        format!("{0}@{1}|{0}@{2}", self.far, self.up, self.down)
    }
}

/// The split of one group: the names of `up` and `down`, and the far end
/// of the TCP flows when the value has one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Parsed {
    pub up: String,
    pub down: String,
    pub tcp: Option<Spec>,
}

/// Read `splits.NAME` (`value`) of the group `name` with the members
/// `list` (the value of `groups.NAME`): `None` for `off`. An error names
/// the key `splits.NAME` (or `groups.NAME`); the router file exits 78.
pub fn read(name: &str, value: &str, list: &str) -> Result<Option<Parsed>> {
    let (_, members) = parse_group(&format!("{name}={list}"))
        .map_err(|e| anyhow!("groups.{name}: {}", e.text()))?;
    let names: Vec<String> = members.into_iter().map(|(m, _)| m).collect();
    let bad = |e: String| anyhow!("splits.{name}: {e}");
    let (rest, tcp) = take(value).map_err(bad)?;
    match podnet_core::network::split::parse(&rest, &names).map_err(bad)? {
        Some((up, down)) => Ok(Some(Parsed { up, down, tcp })),
        None if tcp.is_some() => Err(bad(
            "tcp=FINGERPRINT@UP|DOWN needs up=MEMBER down=MEMBER".to_string()
        )),
        None => Ok(None),
    }
}

/// Take the words `tcp=` and `carriers=` out of `value`. It gives the
/// other words (for the split of the datagrams) and the far end.
pub fn take(value: &str) -> Result<(String, Option<Spec>), String> {
    let mut rest = Vec::new();
    let (mut tcp, mut carriers) = (None, None);
    for word in value
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|w| !w.is_empty())
    {
        match word.split_once('=') {
            Some(("tcp", v)) if tcp.is_none() => tcp = Some(v),
            Some(("carriers", v)) if carriers.is_none() => carriers = Some(v),
            Some(("tcp" | "carriers", _)) => return Err(format!("{word:?}: give each key once")),
            _ => rest.push(word),
        }
    }
    let rest = rest.join(" ");
    let Some(tcp) = tcp else {
        if carriers.is_some() {
            return Err("carriers=UP|DOWN needs tcp=FINGERPRINT@UP|DOWN".to_string());
        }
        return Ok((rest, None));
    };
    let form = || format!("tcp={tcp}: give tcp=FINGERPRINT@UP|DOWN");
    let (far, halves) = tcp.split_once('@').ok_or_else(form)?;
    let (up, down) = halves.split_once('|').ok_or_else(form)?;
    if far.is_empty() || up.is_empty() || down.is_empty() || down.contains('|') {
        return Err(form());
    }
    AllowList::parse(far)
        .map_err(|e| format!("tcp={tcp}: {far:?} is no key: {}", crate::exit::text(&e)))?;
    let (up_carrier, down_carrier) = match carriers {
        None => (DEFAULT_CARRIER, DEFAULT_CARRIER),
        Some(v) => v
            .split_once('|')
            .filter(|(u, d)| !u.is_empty() && !d.is_empty())
            .ok_or_else(|| format!("carriers={v}: give carriers=UP|DOWN"))?,
    };
    for carrier in [up_carrier, down_carrier] {
        if !link_split::takes_way(carrier) {
            return Err(crate::exit::text(&link_split::no_way(carrier)));
        }
    }
    let spec = Spec {
        far: far.to_string(),
        up: up.to_string(),
        down: down.to_string(),
        up_carrier: up_carrier.to_string(),
        down_carrier: down_carrier.to_string(),
    };
    Ok((rest, Some(spec)))
}

/// The state of the session.
enum State {
    /// Not started: a dry run, or a router that has not come up.
    Idle,
    Dialing,
    Up {
        link: Arc<Link>,
        way: Arc<link_door::Way>,
    },
    Down {
        why: String,
    },
}

impl State {
    fn name(&self) -> &'static str {
        match self {
            State::Idle => "idle",
            State::Dialing => "dialing",
            State::Up { .. } => "up",
            State::Down { .. } => "down",
        }
    }
}

/// The TCP split of one group: the session and its counters.
pub struct TcpSplit {
    name: String,
    group: Arc<Balance>,
    up: usize,
    down: usize,
    spec: Spec,
    backoff: (Duration, Duration),
    state: Mutex<State>,
    flows: AtomicU64,
    open: Arc<AtomicU64>,
    fallbacks: AtomicU64,
    sessions: AtomicU64,
    lost: AtomicU64,
    task: Mutex<Option<tokio::task::AbortHandle>>,
}

impl TcpSplit {
    /// The TCP split of `group`, which must have a split of `up` and
    /// `down`. Nothing is dialed until [`TcpSplit::start`].
    pub fn new(group: Arc<Balance>, spec: Spec) -> Result<TcpSplit> {
        let name = group.name().to_string();
        let split = group
            .split()
            .ok_or_else(|| anyhow!("splits.{name}: a split of TCP needs up=MEMBER down=MEMBER"))?;
        if group.way().addressing != crate::network::Addressing::Destination {
            return Err(anyhow!(
                "splits.{name}: a split of TCP needs members that dial a destination"
            ));
        }
        Ok(TcpSplit {
            name,
            group,
            up: split.up,
            down: split.down,
            spec,
            backoff: BACKOFF,
            state: Mutex::new(State::Idle),
            flows: AtomicU64::new(0),
            open: Arc::new(AtomicU64::new(0)),
            fallbacks: AtomicU64::new(0),
            sessions: AtomicU64::new(0),
            lost: AtomicU64::new(0),
            task: Mutex::new(None),
        })
    }

    /// The same split with the waits before a new session (the tests).
    pub fn with_backoff(mut self, first: Duration, most: Duration) -> TcpSplit {
        self.backoff = (first, most.max(first));
        self
    }

    pub fn spec(&self) -> &Spec {
        &self.spec
    }

    /// The build holds the carrier of each half: else exit 69 and the name
    /// of the feature.
    pub fn check(&self) -> Result<()> {
        for carrier in [&self.spec.up_carrier, &self.spec.down_carrier] {
            let feature = crate::link_keyed::feature_of(carrier).unwrap_or(carrier);
            crate::build_features::require(feature)
                .map_err(|e| e.context(format!("splits.{}", self.name)))?;
        }
        Ok(())
    }

    /// Dial the session with the key `me`, and dial again after each loss.
    pub fn start(self: &Arc<Self>, me: Arc<Identity>) {
        let task = tokio::spawn(run(Arc::downgrade(self), me));
        let old = self
            .task
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .replace(task.abort_handle());
        if let Some(old) = old {
            old.abort();
        }
    }

    fn set(&self, state: State) {
        *self.state.lock().unwrap_or_else(|e| e.into_inner()) = state;
    }

    /// The way of the session, when it is up and both members take flows.
    fn session(&self) -> Option<Arc<link_door::Way>> {
        let members = self.group.members();
        if !(members[self.up].health().takes_flows() && members[self.down].health().takes_flows()) {
            return None;
        }
        match &*self.state.lock().unwrap_or_else(|e| e.into_inner()) {
            State::Up { link, way } if link.ended().is_none() => Some(way.clone()),
            _ => None,
        }
    }

    /// True when the session that was up has ended.
    fn ended(&self) -> bool {
        match &*self.state.lock().unwrap_or_else(|e| e.into_inner()) {
            State::Up { link, .. } => link.ended().is_some(),
            _ => true,
        }
    }

    /// Open one session: each half through the way of its member.
    async fn dial_session(&self, me: &Identity) -> Result<Link> {
        let allow = AllowList::parse(&self.spec.far)?;
        let (tune, options) = (Tune::default(), Options::default());
        let half = |carrier: &str, index: usize| {
            let way: Arc<dyn Dial> = Arc::new(Half {
                group: self.group.clone(),
                index,
            });
            link_split::half(carrier, &tune, me, &allow, &options, Some(way))
        };
        let split = pod_link::split::Split::new(
            half(&self.spec.up_carrier, self.up)?,
            half(&self.spec.down_carrier, self.down)?,
        );
        let address = self.spec.address();
        let (link, _) = link_split::dial(&split, me, allow, &address, Settings::default()).await?;
        Ok(link)
    }

    /// The `split` key of the status of the group with the session, its
    /// open streams and the TCP flows that fell back to the pick.
    pub fn merge(&self, split: &mut serde_json::Value) {
        let (state, why) = {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let why = match &*state {
                State::Down { why } => Some(why.clone()),
                _ => None,
            };
            (state.name(), why)
        };
        let tcp_fallbacks = self.fallbacks.load(Ordering::Relaxed);
        let udp_fallbacks = split["fallbacks"].as_u64().unwrap_or(0);
        split["mode"] = "tcp".into();
        split["session"] = serde_json::json!({
            "state": state,
            "far": self.spec.far,
            "address": self.spec.address(),
            "carriers": format!("{}|{}", self.spec.up_carrier, self.spec.down_carrier),
            "sessions": self.sessions.load(Ordering::Relaxed),
            "lost": self.lost.load(Ordering::Relaxed),
            "error": why,
        });
        split["streams"] = self.open.load(Ordering::Relaxed).into();
        split["flows"] = self.flows.load(Ordering::Relaxed).into();
        split["tcp_fallbacks"] = tcp_fallbacks.into();
        split["fallbacks"] = (udp_fallbacks + tcp_fallbacks).into();
    }

    /// The status of the split alone: the status of the group, with
    /// [`TcpSplit::merge`].
    pub fn status(&self) -> serde_json::Value {
        let mut status = self.group.status();
        self.merge(&mut status["split"]);
        status["split"].take()
    }
}

impl Drop for TcpSplit {
    fn drop(&mut self) {
        if let Some(task) = self.task.lock().unwrap_or_else(|e| e.into_inner()).take() {
            task.abort();
        }
    }
}

/// The loop of the session: dial, wait for the end, wait the back-off.
async fn run(split: Weak<TcpSplit>, me: Arc<Identity>) {
    let Some(first) = split.upgrade().map(|s| s.backoff) else {
        return;
    };
    let (mut wait, most) = first;
    loop {
        let Some(this) = split.upgrade() else {
            return;
        };
        this.set(State::Dialing);
        let link = match this.dial_session(&me).await {
            Ok(link) => Arc::new(link),
            Err(e) => {
                eprintln!(
                    "podnet: route  split {}: no session: {e:#}; next in {} s",
                    this.name,
                    wait.as_secs_f64()
                );
                this.set(State::Down {
                    why: format!("{e:#}"),
                });
                drop(this);
                tokio::time::sleep(wait).await;
                wait = (wait * 2).min(most);
                continue;
            }
        };
        let way = Arc::new(link_door::Way::new(link.clone(), Resolve::Peer));
        this.sessions.fetch_add(1, Ordering::Relaxed);
        this.set(State::Up {
            link: link.clone(),
            way,
        });
        eprintln!("podnet: route  split {}: session up", this.name);
        wait = first.0;
        drop(this);
        let why = link.closed().await;
        let Some(this) = split.upgrade() else {
            return;
        };
        this.lost.fetch_add(1, Ordering::Relaxed);
        eprintln!(
            "podnet: route  split {}: session lost: {why}; next in {} s",
            this.name,
            wait.as_secs_f64()
        );
        this.set(State::Down {
            why: why.to_string(),
        });
        drop(this);
        drop(link);
        tokio::time::sleep(wait).await;
        wait = (wait * 2).min(most);
    }
}

impl Dial for TcpSplit {
    fn label(&self) -> String {
        format!("{} split tcp {}", self.group.label(), self.spec.address())
    }

    fn names_inside(&self) -> bool {
        self.group.names_inside()
    }

    /// One stream of the session; the pick of the group when the session
    /// is not up. A pin of the group (T-180) takes the pick, as with no
    /// split, and is no fallback.
    fn dial<'a>(
        &'a self,
        target: &'a Target,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(async move {
            if self.group.pinned().is_some() {
                return self.group.dial(target, deadline).await;
            }
            if let Some(way) = self.session() {
                match way.dial(target, deadline).await {
                    Ok(mut c) => {
                        self.flows.fetch_add(1, Ordering::Relaxed);
                        self.open.fetch_add(1, Ordering::Relaxed);
                        c.io = Box::new(Stream {
                            io: c.io,
                            open: self.open.clone(),
                        });
                        return Ok(c);
                    }
                    // The far end refused the target: it is the exit.
                    Err(e) if !self.ended() => return Err(e),
                    Err(_) => {}
                }
            }
            self.fallbacks.fetch_add(1, Ordering::Relaxed);
            self.group.dial(target, deadline).await
        })
    }

    fn datagrams<'a>(&'a self) -> BoxFuture<'a, Result<Arc<dyn Datagrams>, Failure>> {
        self.group.datagrams()
    }
}

/// The way of one half: the way of the member, with its bytes counted for
/// the member.
struct Half {
    group: Arc<Balance>,
    index: usize,
}

impl Dial for Half {
    fn label(&self) -> String {
        let m = &self.group.members()[self.index];
        format!("{} ({})", m.name, m.way.label())
    }

    fn names_inside(&self) -> bool {
        self.group.members()[self.index].way.dial.names_inside()
    }

    fn dial<'a>(
        &'a self,
        target: &'a Target,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(async move {
            let member = &self.group.members()[self.index];
            let mut c = member.way.dial.dial(target, deadline).await?;
            c.io = member.metered_io(c.io);
            Ok(c)
        })
    }

    fn datagrams<'a>(&'a self) -> BoxFuture<'a, Result<Arc<dyn Datagrams>, Failure>> {
        Box::pin(async move {
            let member = &self.group.members()[self.index];
            Ok(member.metered(member.way.dial.datagrams().await?))
        })
    }
}

/// A stream of the session: the count of open streams falls when it drops.
struct Stream {
    io: BoxedIo,
    open: Arc<AtomicU64>,
}

impl Drop for Stream {
    fn drop(&mut self) {
        self.open.fetch_sub(1, Ordering::Relaxed);
    }
}

impl AsyncRead for Stream {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.io).poll_read(cx, buf)
    }
}

impl AsyncWrite for Stream {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        data: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        std::pin::Pin::new(&mut self.io).poll_write(cx, data)
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.io).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.io).poll_shutdown(cx)
    }
}

#[cfg(test)]
#[path = "split_tcp_tests.rs"]
mod tests;
