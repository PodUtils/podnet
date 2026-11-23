//! The pacer of the records of one stream (T-370, PACE-1 and PACE-3).
//!
//! The session owns the pacing decision, so a stream that a forwarded port
//! carries gets the same pacer as a stream of a terminal. Each stream has
//! its own window: [`WINDOW`] of the measured rate. A record that fits the
//! rest of the window goes at once; a window that is full waits for the
//! next one. A record that is larger than a whole window goes alone, so the
//! pacer never withholds the last record.
//!
//! [`Paced::keepalive`] gives the next keepalive, moved by the jitter of the
//! profile (about 20 percent). The caller sends an empty record when the
//! channel is idle for that interval.

use std::time::{Duration, Instant};

use pod_shape::{Pacer, Profile};

/// The window of one channel: the bytes of this interval of the rate may
/// go at once.
pub const WINDOW: Duration = Duration::from_millis(100);

/// The schedule of the records of one stream.
pub struct Paced {
    pacer: Pacer,
    rate_bps: u64,
    start: Instant,
    used: u64,
}

impl Paced {
    /// The pacer of `profile` at `measured_bps`. A measured rate of zero
    /// takes the rate that the profile declares (PACE-1).
    pub fn new(profile: &Profile, measured_bps: u64, now: Instant) -> Paced {
        let rate = match measured_bps {
            0 => profile.pace.map_or(1, |p| p.rate_bps),
            rate => rate,
        }
        .max(1);
        Paced {
            pacer: Pacer::from_profile(profile, measured_bps),
            rate_bps: rate,
            start: now,
            used: 0,
        }
    }

    /// The bytes of one window of the rate.
    pub fn capacity(&self) -> u64 {
        (self.rate_bps / 8 / 10).max(1)
    }

    /// The measured rate of this channel, in bits per second.
    pub fn rate_bps(&self) -> u64 {
        self.rate_bps
    }

    /// Wait until the window holds `bytes` more, and count them.
    pub async fn wait(&mut self, bytes: usize) {
        self.wait_at(bytes, Instant::now()).await;
    }

    /// [`Paced::wait`] at `now`. The test gives the clock.
    pub async fn wait_at(&mut self, bytes: usize, now: Instant) {
        let bytes = bytes as u64;
        let capacity = self.capacity();
        // A record larger than a window goes alone: no window ever holds it.
        if bytes > capacity {
            return;
        }
        loop {
            if now.saturating_duration_since(self.start) >= WINDOW {
                self.start = now;
                self.used = 0;
            }
            if self.used + bytes <= capacity {
                self.used += bytes;
                return;
            }
            let left = WINDOW.saturating_sub(now.saturating_duration_since(self.start));
            tokio::time::sleep(left.max(Duration::from_millis(1))).await;
            self.start = Instant::now();
            self.used = 0;
        }
    }

    /// The delay of `bytes` at the measured rate, as the pacer of the
    /// profile gives it.
    pub fn delay(&self, bytes: usize) -> Duration {
        self.pacer.delay(bytes)
    }

    /// The next keepalive: the mean of the profile, moved by its jitter.
    pub fn keepalive(&mut self) -> Duration {
        self.pacer.keepalive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_holds_its_share_of_the_rate() {
        let profile = Profile::load("terminal").unwrap();
        let pacer = Paced::new(&profile, 800_000, Instant::now());
        assert_eq!(pacer.capacity(), 10_000);
        let declared = Paced::new(&profile, 0, Instant::now());
        assert_eq!(declared.rate_bps(), profile.pace.unwrap().rate_bps);
    }

    #[test]
    fn the_keepalive_moves_by_the_jitter_of_the_profile() {
        let profile = Profile::load("terminal").unwrap();
        let mut pacer = Paced::new(&profile, 0, Instant::now());
        let mean = Duration::from_secs_f64(profile.pace.unwrap().keepalive_secs);
        for _ in 0..64 {
            let got = pacer.keepalive();
            assert!(got >= mean.mul_f64(0.8), "{got:?} is below the band");
            assert!(got <= mean.mul_f64(1.2), "{got:?} is above the band");
        }
    }
}
