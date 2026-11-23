//! A traffic generator with application profiles as data (T-371, SIM-1 to
//! SIM-4).
//!
//! Each profile is a TOML file named for the application that it imitates:
//! a page load, a video call, a terminal or a file transfer. It holds the
//! object sizes, the gap between two objects and the pause while a user
//! reads. [`Generator::stream`] turns a profile into a determined sequence
//! of requests: the same profile and seed give the same sequence.
//!
//! The generator is a tool for the tests and for the classifier harness. It
//! is not a mode of a road (SIM-4).

use std::fmt;
use std::path::Path;
use std::time::Duration;

use serde::Deserialize;

/// The profiles of the crate, in its order.
pub const NAMES: [&str; 4] = ["page-load", "video-call", "terminal", "file-transfer"];

const FILES: [(&str, &str); 4] = [
    ("page-load", include_str!("profiles/page-load.toml")),
    ("video-call", include_str!("profiles/video-call.toml")),
    ("terminal", include_str!("profiles/terminal.toml")),
    ("file-transfer", include_str!("profiles/file-transfer.toml")),
];

/// One range of object sizes, with its relative weight.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub struct Size {
    pub min: usize,
    pub max: usize,
    /// The relative weight of this range.
    #[serde(default = "one")]
    pub weight: u32,
}

fn one() -> u32 {
    1
}

/// One application as data.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Profile {
    pub name: String,
    /// The application that the profile imitates.
    pub application: String,
    /// The sizes of the objects of one request.
    pub object: Vec<Size>,
    /// The gap between two objects, in milliseconds.
    pub gap_ms: (u64, u64),
    /// The pause while the user reads, in milliseconds. `None` for a
    /// continuous application such as a video call.
    #[serde(default)]
    pub read_ms: Option<(u64, u64)>,
}

impl Profile {
    /// The profile called `name` from the files of the crate.
    pub fn load(name: &str) -> Result<Profile, String> {
        let (_, text) = FILES
            .iter()
            .find(|(file, _)| *file == name)
            .ok_or_else(|| format!("{name:?} is no profile: use {}", NAMES.join(", ")))?;
        Profile::parse(text)
    }

    /// The profile in `text`.
    pub fn parse(text: &str) -> Result<Profile, String> {
        let profile: Profile = toml::from_str(text).map_err(|e| e.to_string())?;
        profile.check()?;
        Ok(profile)
    }

    /// Each profile of the crate.
    pub fn all() -> Result<Vec<Profile>, String> {
        NAMES.iter().map(|name| Profile::load(name)).collect()
    }

    /// The profile in the file at `path`.
    pub fn read(path: impl AsRef<Path>) -> Result<Profile, String> {
        let text = std::fs::read_to_string(path.as_ref())
            .map_err(|e| format!("{}: {e}", path.as_ref().display()))?;
        Profile::parse(&text)
    }

    fn check(&self) -> Result<(), String> {
        if self.object.is_empty() {
            return Err(format!("{}: no object size", self.name));
        }
        for size in &self.object {
            if size.min == 0 || size.min > size.max || size.weight == 0 {
                return Err(format!(
                    "{}: the object range {}..={} weight {} is not a size",
                    self.name, size.min, size.max, size.weight
                ));
            }
        }
        let (low, high) = self.gap_ms;
        if low > high || high > 60_000 {
            return Err(format!(
                "{}: the gap {low}..{high} ms is not a gap",
                self.name
            ));
        }
        if let Some((low, high)) = self.read_ms {
            if low > high || high > 600_000 {
                return Err(format!(
                    "{}: the pause {low}..{high} ms is not a pause",
                    self.name
                ));
            }
        }
        Ok(())
    }
}

impl fmt::Display for Profile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.name, self.application)
    }
}

/// One request of a generated sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    /// The instant of the request from the start of the stream.
    pub at: Duration,
    /// The bytes of its object.
    pub size: usize,
    /// True when the user is reading before this request.
    pub read: bool,
}

/// The generator of one profile.
pub struct Generator {
    profile: Profile,
    seed: u64,
}

impl Generator {
    /// A generator of `profile`, with `seed`.
    pub fn new(profile: Profile, seed: u64) -> Generator {
        Generator { profile, seed }
    }

    /// The profile of this generator.
    pub fn profile(&self) -> &Profile {
        &self.profile
    }

    /// `count` requests at the times of the profile. The same profile and
    /// seed give the same requests.
    pub fn stream(&self, count: usize) -> Vec<Request> {
        let mut rng = Rng::new(self.seed ^ hash(&self.profile.name));
        let mut at = Duration::ZERO;
        let mut out = Vec::with_capacity(count);
        for _ in 0..count {
            let size = pick_size(&self.profile.object, &mut rng);
            let (read, pause) = match self.profile.read_ms {
                Some((low, high)) if rng.below(100) < 15 => {
                    (true, Duration::from_millis(between(low, high, &mut rng)))
                }
                _ => (false, Duration::ZERO),
            };
            at += pause;
            out.push(Request { at, size, read });
            let (low, high) = self.profile.gap_ms;
            at += Duration::from_millis(between(low, high, &mut rng));
        }
        out
    }
}

fn pick_size(sizes: &[Size], rng: &mut Rng) -> usize {
    let total: u64 = sizes.iter().map(|s| u64::from(s.weight)).sum();
    let mut point = rng.below(total.max(1));
    for size in sizes {
        let weight = u64::from(size.weight);
        if point < weight {
            return between(size.min as u64, size.max as u64, rng) as usize;
        }
        point -= weight;
    }
    let last = sizes.last().expect("a checked profile");
    between(last.min as u64, last.max as u64, rng) as usize
}

fn between(low: u64, high: u64, rng: &mut Rng) -> u64 {
    if high <= low {
        return low;
    }
    low + rng.below(high - low + 1)
}

fn hash(text: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// SplitMix64: a small determined generator.
struct Rng {
    state: u64,
}

impl Rng {
    fn new(seed: u64) -> Rng {
        Rng { state: seed }
    }

    fn next(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, limit: u64) -> u64 {
        self.next() % limit.max(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_profile_loads_and_its_stream_stays_inside_its_ranges() {
        for name in NAMES {
            let profile = Profile::load(name).unwrap();
            let stream = Generator::new(profile.clone(), 7).stream(100);
            assert_eq!(stream.len(), 100);
            for (at, request) in stream.iter().enumerate() {
                assert!(
                    profile
                        .object
                        .iter()
                        .any(|range| (range.min..=range.max).contains(&request.size)),
                    "{name}: a request of {} bytes",
                    request.size
                );
                if at > 0 {
                    let gap = request.at - stream[at - 1].at;
                    assert!(gap <= Duration::from_millis(600_000), "{name}: {gap:?}");
                }
            }
        }
    }

    #[test]
    fn a_page_load_holds_a_human_gap_and_a_read_pause() {
        let profile = Profile::load("page-load").unwrap();
        let stream = Generator::new(profile, 3).stream(200);
        assert!(stream.iter().any(|request| request.read));
        for pair in stream.windows(2) {
            let gap = pair[1].at - pair[0].at;
            assert!(gap >= Duration::from_millis(100), "{gap:?}");
        }
    }

    #[test]
    fn the_same_seed_gives_the_same_stream() {
        let profile = Profile::load("terminal").unwrap();
        let a = Generator::new(profile.clone(), 11).stream(50);
        let b = Generator::new(profile, 11).stream(50);
        assert_eq!(a, b);
    }
}
