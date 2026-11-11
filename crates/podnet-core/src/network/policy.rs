//! The named policies of a group (T-178). A policy is a pure function over
//! the snapshots of the members: it reads the health that T-097 gives and
//! changes no health step. A flow takes one member and never moves.
//!
//! * `weighted` (default): a random pick by weight.
//! * `rr`: the members that take flows, in turn. The weights do not count.
//! * `minrtt`: the head, the member with the least `rtt_floor`. The head
//!   changes only when another member is better by a margin for a dwell
//!   of probe rounds ([`next_head`]).
//! * `leastload`: the least busy member (the sort key of `policy::key`).
//! * `protect`: a weighted pick among the members that are `Up`. A member
//!   in `Cooldown` or `Probation` gets no flow while a member is `Up`; the
//!   probe of each interval is the only traffic that it gets.

use std::time::Duration;

use tokio::time::Instant;

use super::balance::Health;

/// A named policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Policy {
    #[default]
    Weighted,
    Rr,
    MinRtt,
    LeastLoad,
    Protect,
}

impl Policy {
    /// The policy of `name`: `weighted`, `rr`, `minrtt`, `leastload` or
    /// `protect`.
    pub fn parse(name: &str) -> Option<Policy> {
        Some(match name.trim() {
            "weighted" => Policy::Weighted,
            "rr" => Policy::Rr,
            "minrtt" => Policy::MinRtt,
            "leastload" => Policy::LeastLoad,
            "protect" => Policy::Protect,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Policy::Weighted => "weighted",
            Policy::Rr => "rr",
            Policy::MinRtt => "minrtt",
            Policy::LeastLoad => "leastload",
            Policy::Protect => "protect",
        }
    }
}

/// The least margin that moves the head of `minrtt`.
pub const HEAD_MARGIN_MIN: Duration = Duration::from_millis(5);
/// The margin that moves the head, as a part of the head's floor (percent).
pub const HEAD_MARGIN_PERCENT: u32 = 10;
/// The probe rounds in a row that a better member must keep its margin.
pub const HEAD_DWELL: u32 = 3;
/// The factor of the load of `leastload` for each second.
pub const LOAD_DECAY: f64 = 0.35;
/// `leastload`: the load under this is the first bucket (bytes).
pub const BUCKET_IDLE: f64 = 4096.0;
/// `leastload`: the load under this is the second bucket (bytes).
pub const BUCKET_BUSY: f64 = 65536.0;

/// What a policy reads of one member.
#[derive(Clone, Copy, Debug)]
pub struct Snapshot {
    pub weight: u32,
    pub health: Health,
    /// The least of the window of passing probe times.
    pub rtt_floor: Option<Duration>,
    /// The opens that have not ended their dial.
    pub opens: u64,
    /// The bytes with a decay, at the time of the pick.
    pub load: f64,
}

impl Snapshot {
    fn takes(&self) -> bool {
        self.health.takes_flows()
    }
}

/// The head of `minrtt` and the member that challenges it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Head {
    pub index: Option<usize>,
    challenger: Option<(usize, u32)>,
}

/// The load of a member after `dt` seconds, with `bytes` more.
pub fn decay(load: f64, dt: Duration, bytes: u64) -> f64 {
    load * LOAD_DECAY.powf(dt.as_secs_f64()) + bytes as f64
}

/// The sort key of `leastload` for member `index` of `n`: the busy bucket,
/// the opens, the load in units of 4096 bytes, and a tie-break that rotates
/// with `turn`. The least key wins.
fn key(s: &Snapshot, index: usize, n: usize, turn: u64) -> (u8, u64, u64, usize) {
    let bucket = if s.load < BUCKET_IDLE {
        0
    } else if s.load < BUCKET_BUSY {
        1
    } else {
        2
    };
    let rotation = (index + n - (turn % n as u64) as usize) % n;
    (bucket, s.opens, (s.load / BUCKET_IDLE) as u64, rotation)
}

fn weighted(s: &[Snapshot], rand: u64, only_up: bool) -> Option<usize> {
    let ok = |m: &Snapshot| m.takes() && (!only_up || m.health == Health::Up);
    let total: u64 = s
        .iter()
        .filter(|m| ok(m))
        .map(|m| u64::from(m.weight))
        .sum();
    if total == 0 {
        return None;
    }
    let mut at = rand % total;
    for (i, m) in s.iter().enumerate().filter(|(_, m)| ok(m)) {
        if at < u64::from(m.weight) {
            return Some(i);
        }
        at -= u64::from(m.weight);
    }
    None
}

/// The index of the member that `policy` picks, or `None` when no member
/// takes flows. `rand` is a random number, `turn` a counter that rises for
/// each pick, `head` the head of `minrtt`.
pub fn choose(policy: Policy, s: &[Snapshot], rand: u64, turn: u64, head: &Head) -> Option<usize> {
    if !s.iter().any(Snapshot::takes) {
        return None;
    }
    match policy {
        Policy::Weighted => weighted(s, rand, false),
        Policy::Rr => {
            let takers: Vec<usize> = (0..s.len()).filter(|&i| s[i].takes()).collect();
            Some(takers[(turn % takers.len() as u64) as usize])
        }
        Policy::MinRtt => head
            .index
            .filter(|&i| s.get(i).is_some_and(Snapshot::takes))
            .or_else(|| best(s))
            .or_else(|| weighted(s, rand, false)),
        Policy::LeastLoad => (0..s.len())
            .filter(|&i| s[i].takes())
            .min_by_key(|&i| key(&s[i], i, s.len(), turn)),
        Policy::Protect => weighted(s, rand, true).or_else(|| weighted(s, rand, false)),
    }
}

/// The member that takes flows and has the least floor (the least index
/// on a tie); `None` when no member has a floor.
fn best(s: &[Snapshot]) -> Option<usize> {
    (0..s.len())
        .filter(|&i| s[i].takes())
        .filter_map(|i| s[i].rtt_floor.map(|f| (f, i)))
        .min()
        .map(|(_, i)| i)
}

/// The head after one probe round. With no head (or a head that is down,
/// or has no floor) the best member is the head at once. Else another
/// member replaces the head after `HEAD_DWELL` rounds in a row in which
/// its floor is less than the floor of the head by `max(5 ms, 10%)`.
pub fn next_head(head: Head, s: &[Snapshot]) -> Head {
    let Some(best) = best(s) else {
        return head;
    };
    let current = head
        .index
        .filter(|&i| s.get(i).is_some_and(|m| m.takes() && m.rtt_floor.is_some()));
    let Some(cur) = current else {
        return Head {
            index: Some(best),
            challenger: None,
        };
    };
    let (cur_floor, best_floor) = (
        s[cur].rtt_floor.unwrap_or_default(),
        s[best].rtt_floor.unwrap_or_default(),
    );
    let margin = HEAD_MARGIN_MIN.max(cur_floor * HEAD_MARGIN_PERCENT / 100);
    if best == cur || best_floor + margin > cur_floor {
        return Head {
            index: Some(cur),
            challenger: None,
        };
    }
    let rounds = match head.challenger {
        Some((who, n)) if who == best => n + 1,
        _ => 1,
    };
    if rounds >= HEAD_DWELL {
        Head {
            index: Some(best),
            challenger: None,
        }
    } else {
        Head {
            index: Some(cur),
            challenger: Some((best, rounds)),
        }
    }
}

/// The time of a load, for the pick.
#[derive(Clone, Copy, Debug)]
pub struct Load {
    pub value: f64,
    pub at: Option<Instant>,
}

impl Load {
    pub fn new() -> Load {
        Load {
            value: 0.0,
            at: None,
        }
    }

    /// The load at `now`.
    pub fn at(&self, now: Instant) -> f64 {
        match self.at {
            Some(t) => decay(self.value, now.saturating_duration_since(t), 0),
            None => 0.0,
        }
    }

    /// Add `bytes` at `now`.
    pub fn add(&mut self, bytes: u64, now: Instant) {
        self.value = decay(self.at(now), Duration::ZERO, bytes);
        self.at = Some(now);
    }
}

impl Default for Load {
    fn default() -> Load {
        Load::new()
    }
}

#[cfg(test)]
mod tests;
