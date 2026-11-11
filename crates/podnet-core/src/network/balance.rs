//! `Balance`: a group of ways with weights and a kill switch (T-096). Each
//! flow takes one member by a weighted pick among the healthy members. When
//! no member is healthy, the flow is refused: the group never falls back to
//! direct. The health of each member comes from the checks of T-097.

use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use tokio::time::Instant;

use super::race::admit;
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
}

/// One member of a group: a way and its weight (1 or more).
pub struct Member {
    pub name: String,
    pub way: Way,
    pub weight: u32,
    health: AtomicU8,
}

impl Member {
    pub fn new(name: &str, way: Way, weight: u32) -> Member {
        Member {
            name: name.to_string(),
            way,
            weight,
            health: AtomicU8::new(Health::Up.value()),
        }
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
    refused: AtomicU64,
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
            refused: AtomicU64::new(0),
        })
    }

    /// The same group with a fixed seed of its pick (the tests).
    pub fn with_seed(self, seed: u64) -> Balance {
        *self.rng.lock().unwrap_or_else(|e| e.into_inner()) = seed | 1;
        self
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

    /// The count of the flows that the kill switch refused.
    pub fn refused(&self) -> u64 {
        self.refused.load(Ordering::Relaxed)
    }

    /// A weighted pick among the members that take flows.
    pub fn pick(&self) -> Selection<'_> {
        let total: u64 = self
            .members
            .iter()
            .filter(|m| m.health().takes_flows())
            .map(|m| u64::from(m.weight))
            .sum();
        if total == 0 {
            self.refused.fetch_add(1, Ordering::Relaxed);
            return Selection::None;
        }
        let mut at = self.next() % total;
        for member in self.members.iter().filter(|m| m.health().takes_flows()) {
            if at < u64::from(member.weight) {
                return Selection::One(member);
            }
            at -= u64::from(member.weight);
        }
        Selection::None
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
        format!("group {}({})", self.name, members.join(", "))
    }

    fn dial<'a>(
        &'a self,
        target: &'a Target,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(async move {
            match self.pick() {
                Selection::One(member) => member.way.dial.dial(target, deadline).await,
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
