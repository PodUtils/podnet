//! The health of the members of a group (T-097): a TCP probe through each
//! member, and the steps of its state. The probe dials through the way of
//! its own member, never through the group. The tests give the time.
//!
//! The steps, with K misses to go down and R successes to come back:
//!
//! * `Up`: a miss gives `Cooldown` (the member still takes flows).
//! * `Cooldown`: R successes in a row, and the cooldown time since the
//!   first miss, give `Up`. K misses in a row give `Down`, when each is a
//!   clean failure and the control probe of the group passed. A probe
//!   that flaps keeps the member in `Cooldown`.
//! * `Down`: a success gives `Probation`.
//! * `Probation`: R successes in a row give `Up`; a clean miss with a
//!   passed control gives `Down`.
//!
//! A failure of the control probe is an outage of each member (the host,
//! not the member): no member goes down for it.

use std::collections::VecDeque;
use std::time::Duration;

use tokio::time::Instant;

use super::balance::Health;
use super::{Class, Failure, Stage, Target, Way};

/// The thresholds of the steps. K > R > 1 and R > K/2, so that a probe
/// that flaps does not take a member back at once (GitHub #373).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Thresholds {
    /// K: the misses in a row that take a member down.
    pub down_after: u32,
    /// R: the successes in a row that take a member up.
    pub up_after: u32,
    /// The least time in `Cooldown` before `Up`.
    pub cooldown: Duration,
    /// The time between two probes.
    pub interval: Duration,
    /// The limit of one probe.
    pub timeout: Duration,
    /// How many passing probe times `Tracker` keeps for `rtt_floor` (0: none).
    pub rtt_window: usize,
}

/// The most passing probe times that a window keeps.
pub const MAX_RTT_WINDOW: usize = 64;

impl Default for Thresholds {
    fn default() -> Thresholds {
        Thresholds {
            down_after: 4,
            up_after: 3,
            cooldown: Duration::from_secs(30),
            interval: Duration::from_secs(10),
            timeout: Duration::from_secs(5),
            rtt_window: 8,
        }
    }
}

impl Thresholds {
    /// Refuse thresholds that break K > R > 1 and R > K/2.
    pub fn check(&self) -> Result<(), Failure> {
        let (k, r) = (self.down_after, self.up_after);
        if self.rtt_window > MAX_RTT_WINDOW {
            return Err(Failure::new(
                Stage::Connect,
                Class::Config,
                format!("the window of round-trip times is 0 to {MAX_RTT_WINDOW}"),
            ));
        }
        if k > r && r > 1 && 2 * r > k {
            return Ok(());
        }
        Err(Failure::new(
            Stage::Connect,
            Class::Config,
            format!("the health thresholds need K > R > 1 and R > K/2 (K {k}, R {r})"),
        ))
    }
}

/// The result of one probe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProbeResult {
    /// The connect worked, in this time.
    Ok(Duration),
    /// The connect failed; `clean` is true for a refusal, an unreachable
    /// host or a reset (a failure that the member gives, not a time out).
    ConnectFailed { clean: bool },
    /// The probe left by another egress than the one of the member.
    WrongEgress,
    /// The member's credential is gone (a WARP session to renew).
    AuthExpired,
}

impl ProbeResult {
    fn ok(self) -> bool {
        matches!(self, ProbeResult::Ok(_))
    }

    fn clean(self) -> bool {
        matches!(
            self,
            ProbeResult::ConnectFailed { clean: true }
                | ProbeResult::WrongEgress
                | ProbeResult::AuthExpired
        )
    }
}

/// A TCP connect to `target` through `way` (the member's own way), closed
/// at once, with its time.
pub async fn probe(way: &Way, target: &Target, timeout: Duration) -> ProbeResult {
    let start = Instant::now();
    match tokio::time::timeout(timeout, way.dial.dial(target, start + timeout)).await {
        Ok(Ok(conn)) => {
            drop(conn);
            ProbeResult::Ok(start.elapsed())
        }
        Ok(Err(e)) => ProbeResult::ConnectFailed {
            clean: !matches!(e.class, Class::Timeout | Class::TempFail),
        },
        Err(_) => ProbeResult::ConnectFailed { clean: false },
    }
}

/// The state of one member, between the probes.
#[derive(Clone, Debug)]
pub struct Tracker {
    health: Health,
    misses: u32,
    hits: u32,
    since: Option<Instant>,
    rtt: Option<Duration>,
    /// The last passing probe times, the oldest first (`Thresholds::rtt_window`).
    window: VecDeque<Duration>,
}

/// The result of one step: the new health, and whether to renew the
/// member (a WARP member: `AuthExpired`, or the step to `Down`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Step {
    pub health: Health,
    pub renew: bool,
}

impl Default for Tracker {
    fn default() -> Tracker {
        Tracker {
            health: Health::Up,
            misses: 0,
            hits: 0,
            since: None,
            rtt: None,
            window: VecDeque::new(),
        }
    }
}

impl Tracker {
    pub fn health(&self) -> Health {
        self.health
    }

    /// The time of the last probe that passed.
    pub fn rtt(&self) -> Option<Duration> {
        self.rtt
    }

    /// The least of the window of passing probe times; the last passing
    /// time when the window is off. A probe that shares a loaded member
    /// reads long when the member is busy, so the least is the input of
    /// `minrtt`. The window changes no health step.
    pub fn rtt_floor(&self) -> Option<Duration> {
        self.window.iter().min().copied().or(self.rtt)
    }

    /// Take the probe `result` at `now`. `control` is the result of the
    /// control probe of the group in this interval.
    pub fn observe(
        &mut self,
        result: ProbeResult,
        control: bool,
        now: Instant,
        t: &Thresholds,
    ) -> Step {
        if let ProbeResult::Ok(rtt) = result {
            self.rtt = Some(rtt);
            if t.rtt_window == 0 {
                self.window.clear();
            } else {
                while self.window.len() >= t.rtt_window {
                    self.window.pop_front();
                }
                self.window.push_back(rtt);
            }
            self.hits += 1;
            self.misses = 0;
        } else {
            self.misses += 1;
            self.hits = 0;
        }
        // A miss counts for the step down only when it is clean and the
        // control passed: an outage of each member keeps each one.
        let may_go_down = !result.ok() && result.clean() && control;
        let before = self.health;
        self.health = match self.health {
            Health::Up if !result.ok() => {
                self.since = Some(now);
                Health::Cooldown
            }
            Health::Up => Health::Up,
            Health::Cooldown if may_go_down && self.misses >= t.down_after => Health::Down,
            Health::Cooldown
                if self.hits >= t.up_after
                    && self
                        .since
                        .is_none_or(|s| now.saturating_duration_since(s) >= t.cooldown) =>
            {
                Health::Up
            }
            Health::Cooldown => Health::Cooldown,
            Health::Down if result.ok() => Health::Probation,
            Health::Down => Health::Down,
            Health::Probation if may_go_down => Health::Down,
            Health::Probation if self.hits >= t.up_after => Health::Up,
            Health::Probation => Health::Probation,
        };
        if self.health == Health::Up {
            self.since = None;
        }
        Step {
            health: self.health,
            renew: result == ProbeResult::AuthExpired
                || (self.health == Health::Down && before != Health::Down),
        }
    }
}

#[cfg(test)]
mod tests;
