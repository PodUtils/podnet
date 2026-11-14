//! The bound of the signature checks of one announce stream (T-347).
//!
//! A frame of 16 nodes costs 17 signature checks: about 0.9 ms of one core
//! in a release build (measured on 2026-11-14), so a peer that sends
//! about 1100 such frames each second takes a core. A replay of a valid
//! announcement passes each check and is dropped only at the `seq` rule.
//! Each stream has a bucket of checks: [`RATE`] each second, [`BURST`] at
//! most. A frame whose checks are over the bucket is dropped before a check,
//! and counted (`limited` of the verb `routes`). The log gets one line for
//! each stream in each [`LOG_EVERY`].

use std::time::{Duration, Instant};

/// The signature checks that a stream can take each second: the
/// announcements of 4096 origins with paths of 5 nodes each 10 s.
pub const RATE: u32 = 2048;
/// The most checks that a stream can take at once.
pub const BURST: u32 = 8192;
/// The time between two log lines of one stream.
pub const LOG_EVERY: Duration = Duration::from_secs(10);

/// A bucket of signature checks.
#[derive(Debug)]
pub struct Bucket {
    rate: u32,
    burst: u32,
    tokens: f64,
    at: Instant,
    logged: Option<Instant>,
}

impl Bucket {
    /// A full bucket of `burst` checks that fills at `rate` each second.
    pub fn new(rate: u32, burst: u32, now: Instant) -> Bucket {
        Bucket {
            rate,
            burst,
            tokens: f64::from(burst),
            at: now,
            logged: None,
        }
    }

    /// Take `cost` checks at `now`; false when the bucket has fewer.
    pub fn take(&mut self, cost: u32, now: Instant) -> bool {
        let gained = now.saturating_duration_since(self.at).as_secs_f64() * f64::from(self.rate);
        self.tokens = (self.tokens + gained).min(f64::from(self.burst));
        self.at = now;
        if self.tokens < f64::from(cost) {
            return false;
        }
        self.tokens -= f64::from(cost);
        true
    }

    /// True when a refusal at `now` gets a log line: one in each
    /// [`LOG_EVERY`].
    pub fn tell(&mut self, now: Instant) -> bool {
        if self
            .logged
            .is_some_and(|at| now.saturating_duration_since(at) < LOG_EVERY)
        {
            return false;
        }
        self.logged = Some(now);
        true
    }
}
