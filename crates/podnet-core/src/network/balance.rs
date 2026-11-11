//! `Balance`: a group of ways with weights and a kill switch (T-096). Each
//! flow takes one member by the policy of the group (T-178; the default is
//! a weighted pick) among the members that take flows. When
//! no member is healthy, the flow is refused: the group never falls back to
//! direct. The health of each member comes from the checks of T-097.

use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use tokio::time::Instant;

use super::health::{ProbeResult, Step, Thresholds, Tracker};
use super::meter::{Meter, Metered, OpenGuard};
use super::policy::{choose, next_head, Head, Policy, Snapshot};
use super::race::admit;
use super::split::{Split, SplitDatagrams};
use super::{BoxFuture, Class, Connected, Datagrams, Dial, Failure, Stage, Target, Way};

/// The health of a member. `Cooldown` and `Probation` (T-097) still take
/// flows; `Down` does not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Health {
    Up,
    Cooldown,
    Probation,
    Down,
}

impl Health {
    fn of(value: u8) -> Health {
        match value {
            0 => Health::Up,
            1 => Health::Cooldown,
            2 => Health::Probation,
            _ => Health::Down,
        }
    }

    fn value(self) -> u8 {
        match self {
            Health::Up => 0,
            Health::Cooldown => 1,
            Health::Probation => 2,
            Health::Down => 3,
        }
    }

    /// True when the member takes new flows.
    pub fn takes_flows(self) -> bool {
        self != Health::Down
    }

    /// `up`, `cooldown`, `probation` or `down`.
    pub fn name(self) -> &'static str {
        match self {
            Health::Up => "up",
            Health::Cooldown => "cooldown",
            Health::Probation => "probation",
            Health::Down => "down",
        }
    }
}

/// One member of a group: a way and its weight (1 or more).
pub struct Member {
    pub name: String,
    pub way: Way,
    pub weight: u32,
    health: AtomicU8,
    tracker: Mutex<Tracker>,
    flows: AtomicU64,
    meter: Arc<Meter>,
}

impl Member {
    pub fn new(name: &str, way: Way, weight: u32) -> Member {
        Member {
            name: name.to_string(),
            way,
            weight,
            health: AtomicU8::new(Health::Up.value()),
            tracker: Mutex::new(Tracker::default()),
            flows: AtomicU64::new(0),
            meter: Arc::new(Meter::default()),
        }
    }

    /// Add `bytes` of traffic at `now` to the load of the member
    /// (`leastload`).
    pub fn add_bytes(&self, bytes: u64, now: Instant) {
        self.meter.add(bytes, now);
    }

    /// Count one open dial of the member until the guard drops.
    pub fn open(&self) -> OpenGuard {
        self.meter.opens.fetch_add(1, Ordering::Relaxed);
        OpenGuard(self.meter.clone())
    }

    /// The dials of the member that have not ended.
    pub fn opens(&self) -> u64 {
        self.meter.opens.load(Ordering::Relaxed)
    }

    /// The bytes that the flows of the member carried, in both directions,
    /// since the start.
    pub fn bytes(&self) -> u64 {
        self.meter.total.load(Ordering::Relaxed)
    }

    /// The least of the window of passing probe times.
    pub fn rtt_floor(&self) -> Option<std::time::Duration> {
        self.tracker
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .rtt_floor()
    }

    fn snapshot(&self, now: Instant) -> Snapshot {
        Snapshot {
            weight: self.weight,
            health: self.health(),
            rtt_floor: self.rtt_floor(),
            opens: self.opens(),
            load: self.meter.load_at(now),
        }
    }

    /// The flows that the pick gave this member.
    pub fn flows(&self) -> u64 {
        self.flows.load(Ordering::Relaxed)
    }

    pub fn health(&self) -> Health {
        Health::of(self.health.load(Ordering::Relaxed))
    }
}

/// The choice of the group for one flow.
#[derive(Debug)]
pub enum Selection<'a> {
    /// The flow goes through this member.
    One(&'a Member),
    /// No member takes flows: the flow is refused (the kill switch).
    None,
}

impl std::fmt::Debug for Member {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Member")
            .field("name", &self.name)
            .field("weight", &self.weight)
            .field("health", &self.health())
            .finish()
    }
}

/// A group of ways with weights and a kill switch.
pub struct Balance {
    name: String,
    members: Vec<Member>,
    rng: Mutex<u64>,
    policy: Policy,
    turn: AtomicU64,
    head: Mutex<Head>,
    refused: AtomicU64,
    /// The split of the directions of the datagrams (T-179); off by default.
    split: Option<Split>,
    split_sockets: AtomicU64,
    split_fallbacks: AtomicU64,
}

impl Balance {
    /// The group `name` of `members`. Each member dials a destination and
    /// has a weight of 1 or more; a group needs one member or more.
    pub fn new(name: &str, members: Vec<Member>) -> Result<Balance, Failure> {
        let ways: Vec<Way> = members.iter().map(|m| m.way.clone()).collect();
        admit(&ways, &format!("the group {name}"))?;
        if let Some(m) = members.iter().find(|m| m.weight == 0) {
            return Err(usage(format!(
                "the member {} of the group {name} has the weight 0",
                m.name
            )));
        }
        let mut seed = [0u8; 8];
        let _ = getrandom::fill(&mut seed);
        Ok(Balance {
            name: name.to_string(),
            members,
            rng: Mutex::new(u64::from_le_bytes(seed) | 1),
            policy: Policy::default(),
            turn: AtomicU64::new(0),
            head: Mutex::new(Head::default()),
            refused: AtomicU64::new(0),
            split: None,
            split_sockets: AtomicU64::new(0),
            split_fallbacks: AtomicU64::new(0),
        })
    }

    /// The same group with a fixed seed of its pick (the tests).
    pub fn with_seed(self, seed: u64) -> Balance {
        *self.rng.lock().unwrap_or_else(|e| e.into_inner()) = seed | 1;
        self
    }

    /// The same group with the policy `policy` (T-178).
    pub fn with_policy(mut self, policy: Policy) -> Balance {
        self.policy = policy;
        self
    }

    pub fn policy(&self) -> Policy {
        self.policy
    }

    /// The same group with the split of the datagrams (T-179): the socket
    /// sends on the member `up` and receives on the member `down`. TCP
    /// flows keep one member. Both must be members of the group, and not
    /// the same one.
    pub fn with_split(mut self, up: &str, down: &str) -> Result<Balance, Failure> {
        let at = |n: &str| self.members.iter().position(|m| m.name == n);
        let (Some(u), Some(d)) = (at(up), at(down)) else {
            return Err(usage(format!(
                "the split of the group {}: {up} and {down} must be members",
                self.name
            )));
        };
        if u == d {
            return Err(usage(format!(
                "the split of the group {}: up and down are the member {up}; a split needs two",
                self.name
            )));
        }
        self.split = Some(Split { up: u, down: d });
        Ok(self)
    }

    pub fn split(&self) -> Option<Split> {
        self.split
    }

    /// End a probe round of the group: `minrtt` may move its head.
    pub fn end_round(&self) {
        let now = Instant::now();
        let snaps: Vec<Snapshot> = self.members.iter().map(|m| m.snapshot(now)).collect();
        let mut head = self.head.lock().unwrap_or_else(|e| e.into_inner());
        *head = next_head(*head, &snaps);
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn members(&self) -> &[Member] {
        &self.members
    }

    /// Set the health of the member `index`.
    pub fn set_health(&self, index: usize, health: Health) {
        if let Some(m) = self.members.get(index) {
            m.health.store(health.value(), Ordering::Relaxed);
        }
    }

    /// Take the probe `result` of the member `index` (T-097): its tracker
    /// steps, and the pick follows the new health. `control` is the result
    /// of the control probe of this interval.
    pub fn observe(
        &self,
        index: usize,
        result: ProbeResult,
        control: bool,
        now: Instant,
        t: &Thresholds,
    ) -> Option<Step> {
        let member = self.members.get(index)?;
        let step = member
            .tracker
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .observe(result, control, now, t);
        member.health.store(step.health.value(), Ordering::Relaxed);
        Some(step)
    }

    /// The status of the group (`podnet route status --json`, T-097): each
    /// member with its health, its weight, the time of its last probe that
    /// passed and its flows, and the refusals of the kill switch.
    pub fn status(&self) -> serde_json::Value {
        let members: Vec<serde_json::Value> = self
            .members
            .iter()
            .map(|m| {
                let (rtt, floor) = {
                    let t = m.tracker.lock().unwrap_or_else(|e| e.into_inner());
                    (t.rtt(), t.rtt_floor())
                };
                let ms = |d: std::time::Duration| d.as_secs_f64() * 1000.0;
                serde_json::json!({
                    "name": m.name,
                    "way": m.way.label(),
                    "weight": m.weight,
                    "health": m.health().name(),
                    "rtt_ms": rtt.map(ms),
                    "rtt_floor_ms": floor.map(ms),
                    "bytes": m.bytes(),
                    "opens": m.opens(),
                    "flows": m.flows(),
                })
            })
            .collect();
        let head = self.head.lock().unwrap_or_else(|e| e.into_inner()).index;
        serde_json::json!({
            "group": self.name,
            "policy": self.policy.name(),
            "split": self.split_status(),
            "head": head.and_then(|i| self.members.get(i)).map(|m| m.name.as_str()),
            "members": members,
            "refused": self.refused(),
        })
    }

    /// The `split` key of the status: the mode (`off` or `udp`), the
    /// members, the split sockets and the sockets that fell back to one
    /// member.
    fn split_status(&self) -> serde_json::Value {
        let name = |i: usize| self.members.get(i).map(|m| m.name.as_str());
        serde_json::json!({
            "mode": if self.split.is_some() { "udp" } else { "off" },
            "up": self.split.and_then(|s| name(s.up)),
            "down": self.split.and_then(|s| name(s.down)),
            "sockets": self.split_sockets.load(Ordering::Relaxed),
            "fallbacks": self.split_fallbacks.load(Ordering::Relaxed),
        })
    }

    /// The count of the flows that the kill switch refused.
    pub fn refused(&self) -> u64 {
        self.refused.load(Ordering::Relaxed)
    }

    /// The pick of the policy among the members that take flows.
    pub fn pick(&self) -> Selection<'_> {
        self.pick_at(Instant::now())
    }

    /// [`Balance::pick`] with the time `now` (the tests give it).
    pub fn pick_at(&self, now: Instant) -> Selection<'_> {
        let snaps: Vec<Snapshot> = self.members.iter().map(|m| m.snapshot(now)).collect();
        let head = *self.head.lock().unwrap_or_else(|e| e.into_inner());
        let turn = self.turn.fetch_add(1, Ordering::Relaxed);
        let chosen = if snaps.iter().any(|s| s.health.takes_flows()) {
            choose(self.policy, &snaps, self.next(), turn, &head)
        } else {
            None
        };
        match chosen.and_then(|i| self.members.get(i)) {
            Some(member) => {
                member.flows.fetch_add(1, Ordering::Relaxed);
                Selection::One(member)
            }
            None => {
                self.refused.fetch_add(1, Ordering::Relaxed);
                Selection::None
            }
        }
    }

    /// A pick that also counts an open dial of the member until the guard
    /// drops (`leastload`).
    pub fn pick_open(&self, now: Instant) -> (Selection<'_>, Option<OpenGuard>) {
        match self.pick_at(now) {
            Selection::One(m) => (Selection::One(m), Some(m.open())),
            Selection::None => (Selection::None, None),
        }
    }

    /// The next number of the pick (splitmix64).
    fn next(&self) -> u64 {
        let mut state = self.rng.lock().unwrap_or_else(|e| e.into_inner());
        *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = *state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// The split socket, or `None` when the group falls back to one member
    /// (a member of the split does not take flows). The fallback keeps the
    /// pick of the group, so it never leaves the members that take flows.
    async fn split_datagrams(&self) -> Result<Option<Arc<dyn Datagrams>>, Failure> {
        let Some(split) = self.split else {
            return Ok(None);
        };
        let (up, down) = (&self.members[split.up], &self.members[split.down]);
        if !(up.health().takes_flows() && down.health().takes_flows()) {
            self.split_fallbacks.fetch_add(1, Ordering::Relaxed);
            return Ok(None);
        }
        let sender = up.way.dial.datagrams().await?;
        up.flows.fetch_add(1, Ordering::Relaxed);
        let Ok(receiver) = down.way.dial.datagrams().await else {
            // The member `down` has no datagrams: the socket keeps `up`.
            self.split_fallbacks.fetch_add(1, Ordering::Relaxed);
            return Ok(Some(sender));
        };
        down.flows.fetch_add(1, Ordering::Relaxed);
        self.split_sockets.fetch_add(1, Ordering::Relaxed);
        Ok(Some(Arc::new(SplitDatagrams::new(sender, receiver))))
    }

    fn refusal(&self, what: &str) -> Failure {
        Failure::new(
            Stage::Connect,
            Class::Policy,
            format!(
                "the group {} has no healthy member: {what} is refused (the kill switch; no direct way)",
                self.name
            ),
        )
    }
}

fn usage(text: String) -> Failure {
    Failure::new(Stage::Connect, Class::Usage, text)
}

impl Dial for Balance {
    fn label(&self) -> String {
        let members: Vec<String> = self
            .members
            .iter()
            .map(|m| format!("{}*{}", m.name, m.weight))
            .collect();
        let policy = match self.policy {
            Policy::Weighted => String::new(),
            p => format!(" {}", p.name()),
        };
        format!("group {}{policy}({})", self.name, members.join(", "))
    }

    fn dial<'a>(
        &'a self,
        target: &'a Target,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(async move {
            let (selection, open) = self.pick_open(Instant::now());
            match selection {
                Selection::One(member) => {
                    let mut conn = member.way.dial.dial(target, deadline).await?;
                    // The meter counts each byte of the flow for the member;
                    // the open dial ends at the first byte.
                    conn.io = Box::new(Metered::new(conn.io, member.meter.clone(), open));
                    Ok(conn)
                }
                Selection::None => Err(self.refusal(&format!("the flow to {target}"))),
            }
        })
    }

    /// True when one member or more sends the name.
    fn names_inside(&self) -> bool {
        self.members.iter().any(|m| m.way.dial.names_inside())
    }

    /// The datagrams of one member of the pick: each datagram of the socket
    /// keeps that member.
    fn datagrams<'a>(&'a self) -> BoxFuture<'a, Result<Arc<dyn Datagrams>, Failure>> {
        Box::pin(async move {
            if let Some(socket) = self.split_datagrams().await? {
                return Ok(socket);
            }
            match self.pick() {
                Selection::One(member) => member.way.dial.datagrams().await,
                Selection::None => Err(self.refusal("a datagram socket")),
            }
        })
    }
}

/// One group of `--group NAME=a*2,b*1`: the name, and each member with its
/// weight (1 when the member has no `*N`).
pub fn parse_group(spec: &str) -> Result<(String, Vec<(String, u32)>), Failure> {
    let (name, list) = spec
        .split_once('=')
        .ok_or_else(|| usage(format!("--group {spec:?}: give NAME=MEMBER*WEIGHT,...")))?;
    let name = name.trim();
    if name.is_empty()
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(usage(format!(
            "--group {spec:?}: the name is letters, digits, - and _"
        )));
    }
    let mut members = Vec::new();
    for item in list.split(',').map(str::trim).filter(|i| !i.is_empty()) {
        let (member, weight) = match item.split_once('*') {
            Some((m, w)) => {
                let weight = w
                    .trim()
                    .parse::<u32>()
                    .ok()
                    .filter(|w| (1..=1000).contains(w))
                    .ok_or_else(|| {
                        usage(format!("--group {name}: the weight of {m} is 1 to 1000"))
                    })?;
                (m.trim(), weight)
            }
            None => (item, 1),
        };
        if member.is_empty() {
            return Err(usage(format!("--group {name}: a member has no name")));
        }
        if members.iter().any(|(m, _): &(String, u32)| m == member) {
            return Err(usage(format!(
                "--group {name}: the member {member} is there twice"
            )));
        }
        members.push((member.to_string(), weight));
    }
    if members.is_empty() {
        return Err(usage(format!("--group {name}: give one member or more")));
    }
    Ok((name.to_string(), members))
}

#[cfg(test)]
mod tests;
