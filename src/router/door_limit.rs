//! The limits of a door (T-405): a rate, a quota over a period, and what
//! happens at the quota. The usage of a quota is kept in a file of the
//! state directory, so that a restart does not give a new allowance.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{bail, Result};
use pod_core::clock::{civil_from_days, days_from_civil};
use tokio::time::Instant;

/// The period of a quota (UTC).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Period {
    /// The quota holds for the life of the usage file.
    None,
    Day,
    /// From Monday 00:00 UTC.
    Week,
    /// From the first day of the month, 00:00 UTC.
    Month,
}

/// What a door does when its quota is used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exhaust {
    /// New flows are refused; the open flows finish.
    Refuse,
    /// New flows are refused, and the open flows end.
    Close,
}

/// The limits of one door. No rate and no quota: no limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Bytes each second, in each direction.
    pub rate: Option<u64>,
    /// Bytes in both directions for each period.
    pub quota: Option<u64>,
    pub period: Period,
    pub on_exhaust: Exhaust,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            rate: None,
            quota: None,
            period: Period::None,
            on_exhaust: Exhaust::Refuse,
        }
    }
}

/// A size: digits (a decimal point is allowed), then a unit of 1000:
/// none or `B`, `k`/`KB`, `M`/`MB`, `G`/`GB`, `T`/`TB`. `5MB` is 5000000.
pub fn parse_size(text: &str) -> Result<u64> {
    let text = text.trim();
    let split = text
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(text.len());
    let (number, unit) = text.split_at(split);
    let number: f64 = number
        .parse()
        .map_err(|_| anyhow::anyhow!("{text:?}: give a size, for example 5MB"))?;
    let scale: f64 = match unit.trim().to_ascii_uppercase().as_str() {
        "" | "B" => 1.0,
        "K" | "KB" => 1e3,
        "M" | "MB" => 1e6,
        "G" | "GB" => 1e9,
        "T" | "TB" => 1e12,
        _ => bail!("{text:?}: the unit is B, KB, MB, GB or TB"),
    };
    let bytes = (number * scale).round();
    if !(1.0..=1e18).contains(&bytes) {
        bail!("{text:?}: give 1 byte to 1000000 TB");
    }
    Ok(bytes as u64)
}

impl Limits {
    /// The limits of the keys of a door. `period` needs `quota`, and
    /// `on_exhaust` needs `quota`.
    pub fn parse(
        rate: Option<&str>,
        quota: Option<&str>,
        period: Option<&str>,
        on_exhaust: Option<&str>,
    ) -> Result<Limits> {
        let rate = rate
            .map(parse_size)
            .transpose()
            .map_err(|e| e.context("rate"))?;
        let quota = quota
            .map(parse_size)
            .transpose()
            .map_err(|e| e.context("quota"))?;
        if quota.is_none() && period.is_some() {
            bail!("period: give quota too");
        }
        if quota.is_none() && on_exhaust.is_some() {
            bail!("on_exhaust: give quota too");
        }
        let period = match period {
            None | Some("none") => Period::None,
            Some("day") => Period::Day,
            Some("week") => Period::Week,
            Some("month") => Period::Month,
            Some(other) => bail!("period: {other:?} is not day, week, month or none"),
        };
        let on_exhaust = match on_exhaust {
            None | Some("refuse") => Exhaust::Refuse,
            Some("close") => Exhaust::Close,
            Some(other) => bail!("on_exhaust: {other:?} is not refuse or close"),
        };
        Ok(Limits {
            rate,
            quota,
            period,
            on_exhaust,
        })
    }

    pub fn is_none(&self) -> bool {
        self.rate.is_none() && self.quota.is_none()
    }
}

impl Period {
    /// The start and the end (UNIX seconds) of the period that holds `now`.
    /// [`Period::None`] is one period from 0 to the end of time.
    pub fn bounds(self, now: i64) -> (i64, i64) {
        let day = now.div_euclid(86400);
        match self {
            Period::None => (0, i64::MAX),
            Period::Day => (day * 86400, (day + 1) * 86400),
            Period::Week => {
                // 1970-01-01 was a Thursday: Monday is 3 days before.
                let monday = day - (day + 3).rem_euclid(7);
                (monday * 86400, (monday + 7) * 86400)
            }
            Period::Month => {
                let (y, m, _) = civil_from_days(day);
                let first = days_from_civil(y, m, 1);
                let next = if m == 12 {
                    days_from_civil(y + 1, 1, 1)
                } else {
                    days_from_civil(y, m + 1, 1)
                };
                (first * 86400, next * 86400)
            }
        }
    }
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// A token bucket of one direction: `rate` bytes each second, a burst of
/// a tenth of a second (at least 16 KiB).
#[derive(Debug)]
struct Bucket {
    tokens: f64,
    last: Instant,
}

/// The state of the limits of one door.
#[derive(Debug)]
pub struct Limiter {
    pub limits: Limits,
    used: AtomicU64,
    period_start: AtomicU64,
    buckets: [Mutex<Bucket>; 2],
    /// The usage file, and the key of the door in it.
    file: Option<(PathBuf, String)>,
    saved: AtomicU64,
    /// The UNIX second of the last write of the file.
    last_save: AtomicU64,
}

/// The usage goes to the file at most this often, and at the end.
const SAVE_EVERY: u64 = 10;

impl Drop for Limiter {
    fn drop(&mut self) {
        if let Err(e) = self.save() {
            tracing::warn!("the usage of a door was not written: {e:#}");
        }
    }
}

/// The direction of a byte: to the far end, or from it.
#[derive(Debug, Clone, Copy)]
pub enum Way {
    Out = 0,
    In = 1,
}

impl Limiter {
    pub fn new(limits: Limits, file: Option<(PathBuf, String)>) -> Limiter {
        let now = Instant::now();
        let bucket = || {
            Mutex::new(Bucket {
                tokens: 0.0,
                last: now,
            })
        };
        let limiter = Limiter {
            limits,
            used: AtomicU64::new(0),
            period_start: AtomicU64::new(limits.period.bounds(unix_now()).0 as u64),
            buckets: [bucket(), bucket()],
            file,
            saved: AtomicU64::new(0),
            last_save: AtomicU64::new(unix_now() as u64),
        };
        limiter.load();
        limiter
    }

    fn burst(&self) -> f64 {
        self.limits
            .rate
            .map_or(0.0, |r| (r as f64 / 10.0).max(16384.0))
    }

    /// How many of `want` bytes may pass now in the direction `way`; with
    /// 0, the time to wait for the next byte.
    pub fn take(&self, way: Way, want: usize) -> Result<usize, Duration> {
        let Some(rate) = self.limits.rate else {
            return Ok(want);
        };
        let mut bucket = self.buckets[way as usize].lock().unwrap();
        let now = Instant::now();
        let refill = now.duration_since(bucket.last).as_secs_f64() * rate as f64;
        bucket.tokens = (bucket.tokens + refill).min(self.burst());
        bucket.last = now;
        if bucket.tokens >= 1.0 {
            let n = (bucket.tokens as usize).min(want);
            bucket.tokens -= n as f64;
            return Ok(n);
        }
        let wait = (1.0 - bucket.tokens) / rate as f64;
        Err(Duration::from_secs_f64(wait.max(0.001)))
    }

    /// Put back `n` bytes that `take` gave and that did not pass.
    pub fn give_back(&self, way: Way, n: usize) {
        if n == 0 || self.limits.rate.is_none() {
            return;
        }
        let mut bucket = self.buckets[way as usize].lock().unwrap();
        bucket.tokens = (bucket.tokens + n as f64).min(self.burst());
    }

    /// The period starts again when it changed: the usage is 0.
    fn roll(&self) {
        let start = self.limits.period.bounds(unix_now()).0 as u64;
        if self.period_start.swap(start, Ordering::Relaxed) != start {
            self.used.store(0, Ordering::Relaxed);
        }
    }

    /// Count `bytes` against the quota.
    pub fn count(&self, bytes: u64) {
        if self.limits.quota.is_some() {
            self.roll();
            self.used.fetch_add(bytes, Ordering::Relaxed);
            let now = unix_now() as u64;
            let last = self.last_save.load(Ordering::Relaxed);
            if now >= last + SAVE_EVERY
                && self
                    .last_save
                    .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
                    .is_ok()
            {
                if let Err(e) = self.save() {
                    tracing::warn!("the usage of a door was not written: {e:#}");
                }
            }
        }
    }

    /// True when the quota of this period is used.
    pub fn exhausted(&self) -> bool {
        match self.limits.quota {
            Some(quota) => {
                self.roll();
                self.used.load(Ordering::Relaxed) >= quota
            }
            None => false,
        }
    }

    /// True when an open flow must end now (`on_exhaust: close`).
    pub fn must_close(&self) -> bool {
        self.limits.on_exhaust == Exhaust::Close && self.exhausted()
    }

    /// The bytes that one read or write of an open flow may carry
    /// (GitHub #637): with `on_exhaust: close`, at most the bytes left of
    /// the quota, so a flow ends at the quota and not one buffer after it.
    /// Else `want`.
    pub fn cap(&self, want: usize) -> usize {
        match self.limits.quota {
            Some(quota) if self.limits.on_exhaust == Exhaust::Close => {
                self.roll();
                let left = quota.saturating_sub(self.used.load(Ordering::Relaxed));
                want.min(usize::try_from(left).unwrap_or(usize::MAX))
            }
            _ => want,
        }
    }

    pub fn used(&self) -> u64 {
        self.used.load(Ordering::Relaxed)
    }

    /// The limits and the usage, for the status document.
    pub fn json(&self) -> serde_json::Value {
        self.roll();
        let start = self.period_start.load(Ordering::Relaxed) as i64;
        let end = self.limits.period.bounds(start).1;
        serde_json::json!({
            "rate": self.limits.rate,
            "quota": self.limits.quota,
            "period": format!("{:?}", self.limits.period).to_ascii_lowercase(),
            "on_exhaust": format!("{:?}", self.limits.on_exhaust).to_ascii_lowercase(),
            "used": self.used(),
            "period_end": (end != i64::MAX).then_some(end),
            "exhausted": self.exhausted(),
        })
    }

    /// Read the usage of this door from the file, when it is of this period.
    fn load(&self) {
        let Some((path, key)) = &self.file else {
            return;
        };
        let Ok(text) = std::fs::read_to_string(path) else {
            return;
        };
        let Ok(doc) = serde_json::from_str::<serde_json::Value>(&text) else {
            tracing::warn!(path = %path.display(), "the usage file of the doors does not parse; the usage starts at 0");
            return;
        };
        let row = &doc[key.as_str()];
        if row["period_start"].as_u64() == Some(self.period_start.load(Ordering::Relaxed)) {
            let used = row["used"].as_u64().unwrap_or(0);
            self.used.store(used, Ordering::Relaxed);
            self.saved.store(used, Ordering::Relaxed);
        }
    }

    /// Write the usage of this door to the file when it changed (a file
    /// of mode 0600; the other doors of the file stay).
    pub fn save(&self) -> Result<()> {
        let Some((path, key)) = &self.file else {
            return Ok(());
        };
        if self.limits.quota.is_none() {
            return Ok(());
        }
        self.roll();
        let used = self.used();
        if self.saved.swap(used, Ordering::Relaxed) == used && path.exists() {
            return Ok(());
        }
        let mut doc: serde_json::Value = std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .filter(serde_json::Value::is_object)
            .unwrap_or_else(|| serde_json::json!({}));
        doc[key.as_str()] = serde_json::json!({
            "period_start": self.period_start.load(Ordering::Relaxed),
            "used": used,
        });
        if let Some(dir) = path.parent() {
            podnet_core::state::prepare(dir)?;
        }
        podnet_core::state::replace_file(path, doc.to_string().as_bytes(), 0o600)
    }
}

/// The usage file of the doors: `router-usage.json` in the `state_dir` of
/// the router file, else in the state directory of podnet (GitHub #619).
pub fn usage_file(state_dir: Option<&std::path::Path>) -> Option<PathBuf> {
    match state_dir {
        Some(dir) => Some(dir.to_path_buf()),
        None => podnet_core::state::state_dir().ok().flatten(),
    }
    .map(|d| d.join("router-usage.json"))
}

#[cfg(test)]
#[path = "door_limit_tests.rs"]
mod tests;
