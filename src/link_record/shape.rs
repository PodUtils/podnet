//! The shape of the records that one stream seals (T-370).
//!
//! A [`ShapeCounters`] holds the records, the payload bytes, the body bytes
//! and the padding bytes of one end. The session, or the instance that holds
//! the stream, reads it to report the padded fraction. The counters are
//! atomic because the sender task writes them and the command reads them.
//!
//! [`profile_of`] loads a profile from its name; a name that this build does
//! not have exits 64. [`first_flight`] is the opening write of a profile at
//! a measured rate: two seconds of the rate, bounded by the target.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use anyhow::anyhow;
use pod_shape::{Entropy, PaddedFraction, Profile, Shape, ShapeReport};

use crate::exit::{self, Fault};

/// The records, the payload, the bodies and the padding of one end.
#[derive(Debug)]
pub struct ShapeCounters {
    records: AtomicU64,
    payload: AtomicU64,
    body: AtomicU64,
    padded: AtomicU64,
    entropy_records: AtomicU64,
    entropy_bytes: AtomicU64,
    entropy_low: AtomicU64,
    entropy_high: AtomicU64,
}

impl Default for ShapeCounters {
    fn default() -> ShapeCounters {
        ShapeCounters {
            records: AtomicU64::new(0),
            payload: AtomicU64::new(0),
            body: AtomicU64::new(0),
            padded: AtomicU64::new(0),
            entropy_records: AtomicU64::new(0),
            entropy_bytes: AtomicU64::new(0),
            entropy_low: AtomicU64::new(u64::MAX),
            entropy_high: AtomicU64::new(0),
        }
    }
}

impl ShapeCounters {
    /// A counter at zero.
    pub fn new() -> Arc<ShapeCounters> {
        Arc::new(ShapeCounters::default())
    }

    /// Count one record whose payload is `payload` bytes and whose body is
    /// `body` bytes.
    pub fn record(&self, payload: usize, body: usize) {
        self.records.fetch_add(1, Ordering::Relaxed);
        self.payload.fetch_add(payload as u64, Ordering::Relaxed);
        self.body.fetch_add(body as u64, Ordering::Relaxed);
        self.padded
            .fetch_add((body - payload) as u64, Ordering::Relaxed);
    }

    /// The records that this end sealed.
    pub fn records(&self) -> u64 {
        self.records.load(Ordering::Relaxed)
    }

    /// The payload bytes of those records.
    pub fn payload(&self) -> u64 {
        self.payload.load(Ordering::Relaxed)
    }

    /// The body bytes of those records.
    pub fn body(&self) -> u64 {
        self.body.load(Ordering::Relaxed)
    }

    /// The padding bytes of those records.
    pub fn padded(&self) -> u64 {
        self.padded.load(Ordering::Relaxed)
    }

    /// The records that this end sealed with an entropy tail (PAD-3).
    pub fn entropy_records(&self) -> u64 {
        self.entropy_records.load(Ordering::Relaxed)
    }

    /// The bytes of those tails.
    pub fn entropy_bytes(&self) -> u64 {
        self.entropy_bytes.load(Ordering::Relaxed)
    }

    /// The lowest entropy of one tail, in bits per byte, or `None` before
    /// the first tail.
    pub fn entropy_low(&self) -> Option<f64> {
        (self.entropy_records() > 0).then(|| self.entropy_low.load(Ordering::Relaxed) as f64 / 1e6)
    }

    /// The highest entropy of one tail, in bits per byte.
    pub fn entropy_high(&self) -> Option<f64> {
        (self.entropy_records() > 0).then(|| self.entropy_high.load(Ordering::Relaxed) as f64 / 1e6)
    }

    /// Count one tail of `bytes` between the payload and the end of the
    /// body, and keep the band of its histogram.
    pub fn entropy(&self, bytes: &[u8]) {
        self.entropy_records.fetch_add(1, Ordering::Relaxed);
        self.entropy_bytes
            .fetch_add(bytes.len() as u64, Ordering::Relaxed);
        let bits = (bits_per_byte(bytes) * 1e6) as u64;
        self.entropy_low.fetch_min(bits, Ordering::Relaxed);
        self.entropy_high.fetch_max(bits, Ordering::Relaxed);
    }

    /// The padding over the body bytes.
    pub fn fraction(&self) -> PaddedFraction {
        PaddedFraction::of(self.padded(), self.body())
    }

    /// The line of `link status` and of the summary of a transfer. A
    /// transfer with no record says so.
    pub fn line(&self) -> String {
        format!(
            "padded {} of {} bytes ({:.1}%)  records {}",
            self.padded(),
            self.body(),
            self.fraction().percent(),
            self.records()
        )
    }
}

/// The profile called `name`. An unknown name exits 64 with the names of
/// this build.
pub fn profile_of(name: &str) -> anyhow::Result<Profile> {
    Profile::load(name).map_err(|e| {
        exit::fault(
            Fault::Usage,
            anyhow!(
                "the profile {name:?}: {e}; the profiles are {}",
                pod_shape::NAMES.join(", ")
            ),
        )
    })
}

/// The opening write of `profile` at `measured_bps`. An explicit `target`
/// replaces the target of the profile. A measured rate bounds both: the
/// opening write fits two seconds of it (FLY-1). A rate of zero is unknown,
/// so the target stays as it is.
pub fn first_flight(profile: Option<&Profile>, target: Option<usize>, measured_bps: u64) -> usize {
    let want = target
        .or_else(|| profile.and_then(|profile| profile.first_flight))
        .unwrap_or(0);
    if measured_bps == 0 {
        return want;
    }
    let fit = (measured_bps / 8).saturating_mul(2);
    want.min(usize::try_from(fit).unwrap_or(usize::MAX))
}

/// The entropy of `profile`, or `None` when the profile does not declare a
/// band. The seed is separate from the one of the shaper.
pub fn entropy_of(profile: &Profile, seed: u64) -> Option<Entropy> {
    let band = profile.entropy?;
    Entropy::new(band, seed).ok()
}

/// The Shannon entropy of `bytes`, in bits per byte.
fn bits_per_byte(bytes: &[u8]) -> f64 {
    if bytes.is_empty() {
        return 0.0;
    }
    let mut counts = [0u64; 256];
    for byte in bytes {
        counts[*byte as usize] += 1;
    }
    let total = bytes.len() as f64;
    let mut bits = 0.0;
    for count in counts {
        if count > 0 {
            let p = count as f64 / total;
            bits -= p * p.log2();
        }
    }
    bits
}

/// The declared shape of `profile` beside the shape of `samples`.
pub fn report(profile: &Profile, samples: &[usize], padded: PaddedFraction) -> ShapeReport {
    let declared = Shape::of_profile(profile);
    let observed = (!samples.is_empty()).then(|| Shape::of(samples, padded));
    ShapeReport::compare(declared, observed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_opening_write_is_two_seconds_at_a_slow_rate() {
        let profile = Profile::load("page-load").unwrap();
        assert_eq!(first_flight(Some(&profile), None, 10_000_000), 16384);
        assert_eq!(first_flight(Some(&profile), None, 64_000), 16000);
        assert_eq!(first_flight(Some(&profile), Some(9000), 64_000), 9000);
        assert_eq!(first_flight(None, Some(9000), 64_000), 9000);
        assert_eq!(first_flight(None, Some(100_000), 64_000), 16_000);
        assert_eq!(first_flight(None, None, 64_000), 0);
    }

    #[test]
    fn the_counters_hold_the_padded_fraction() {
        let counters = ShapeCounters::new();
        counters.record(100, 128);
        counters.record(200, 512);
        assert_eq!(counters.records(), 2);
        assert_eq!(counters.payload(), 300);
        assert_eq!(counters.body(), 640);
        assert_eq!(counters.padded(), 340);
        assert!(counters.line().contains("53.1%"));
    }

    #[test]
    fn the_entropy_counter_holds_the_band() {
        let counters = ShapeCounters::new();
        counters.entropy(&[0, 0, 255]);
        assert_eq!(counters.entropy_records(), 1);
        assert_eq!(counters.entropy_bytes(), 3);
        assert_eq!(counters.entropy_low(), counters.entropy_high());
    }
}
