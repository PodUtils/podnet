//! `podnet doctor --shape`: the declared shape of each road beside the
//! observed one (T-370, DECL-1 and DECL-2).
//!
//! A road declares the profile that it imitates: the buckets of its record
//! bodies, its padded fraction and its rule. The check shapes a sample of
//! records through [`pod_shape::Shaper`] and compares the result with the
//! declared distribution.
//!
//! The state of one road is one of three words:
//!
//! * `declared`: the profile is declared and no record was observed;
//! * `observed`: the sample holds each bucket within
//!   [`pod_shape::ShapeReport::TOLERANCE`];
//! * `diverged`: a bucket or the padded fraction is outside the tolerance.
//!
//! A divergence is a warning line, never a failed session (DECL-2): the
//! command exits 0.

use pod_shape::{PaddedFraction, Profile, Shaper, State};

use crate::link_record::{report, ShapeCounters};

/// One road and the profile that it declares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Road {
    /// The name of the road.
    pub name: &'static str,
    /// The profile that the road declares.
    pub profile: &'static str,
}

/// The roads of the sealed record layer, each with its profile. The four
/// profiles of `pod-shape` are the four kinds of traffic that a stream can
/// imitate; each road name is the traffic that the profile covers.
pub const ROADS: [Road; 4] = [
    Road {
        name: "page-load",
        profile: "page-load",
    },
    Road {
        name: "video-call",
        profile: "video-call",
    },
    Road {
        name: "terminal",
        profile: "terminal",
    },
    Road {
        name: "file-transfer",
        profile: "file-transfer",
    },
];

/// The records that one road samples. 1000 records compare each bucket of
/// the declared histogram within the tolerance of the crate.
pub const SAMPLES: usize = 1000;

/// One line of the report, and its warnings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub road: &'static str,
    pub profile: String,
    /// `declared`, `observed` or `diverged`.
    pub state: &'static str,
    /// The one line of the road.
    pub text: String,
    /// One warning for each divergence, in words.
    pub warnings: Vec<String>,
}

impl Line {
    /// The warnings as `warning ROAD: what` lines.
    pub fn warning_lines(&self) -> Vec<String> {
        self.warnings
            .iter()
            .map(|warning| format!("warning {}: {warning}", self.road))
            .collect()
    }
}

/// The word of a state: the three states of DECL-1.
pub fn word(state: State) -> &'static str {
    match state {
        State::Unseen => "declared",
        State::Holds => "observed",
        State::Fails => "diverged",
    }
}

/// The bodies of `count` records of `profile`. Each session pads its own
/// records, so a rule `first:N` is sampled across enough sessions to reach
/// `count` padded records. The payload is short enough that every rule
/// pads it.
fn samples(profile: &Profile, count: usize) -> Vec<usize> {
    // One byte pads under each `Below` rule of the four profiles, and the
    // opening records of the other rules pad as well.
    let payload = match profile.rule {
        pod_shape::Rule::Below(_) => 1,
        _ => 64,
    };
    let session = match profile.rule {
        pod_shape::Rule::First(n) => (n as usize).max(1),
        _ => count.max(1),
    };
    let mut shaper = Shaper::new(profile.clone(), 0x5eed_5eed_0001);
    let mut out = Vec::with_capacity(count);
    for at in 0..count {
        if at > 0 && at % session == 0 {
            shaper = Shaper::new(profile.clone(), 0x5eed_5eed_0001 ^ at as u64);
        }
        out.push(shaper.shape(payload).body);
    }
    out
}

/// The counters of a sampled road: the body bytes and the padding bytes.
fn counters(profile: &Profile, bodies: &[usize]) -> std::sync::Arc<ShapeCounters> {
    let counters = ShapeCounters::new();
    let payload = match profile.rule {
        pod_shape::Rule::Below(_) => 1,
        _ => 64,
    };
    for body in bodies {
        counters.record(payload, *body);
    }
    counters
}

/// The line of `declared`, with samples that `observed` produces. The test
/// gives another profile for `observed` to perturb the shape.
pub fn line_with(declared: &Profile, observed: Option<&Profile>, count: usize) -> Line {
    let bodies = observed
        .map(|profile| samples(profile, count))
        .unwrap_or_default();
    let counters = observed.map(|profile| counters(profile, &bodies));
    let padded = counters
        .as_ref()
        .map_or(PaddedFraction::default(), |c| c.fraction());
    let report = report(declared, &bodies, padded);
    let mut text = format!(
        "road {}  profile {}  state {}  records {}  padded {:.1}%  buckets",
        declared.name,
        declared.name,
        word(report.state),
        bodies.len(),
        padded.percent()
    );
    for (size, fraction) in &report.declared.buckets {
        text.push_str(&format!(" {size}:{:.0}%", fraction * 100.0));
    }
    Line {
        road: road_of(&declared.name),
        profile: declared.name.clone(),
        state: word(report.state),
        text,
        warnings: report.warnings,
    }
}

/// The line of one road, sampled from its own profile.
pub fn line(road: Road, count: usize) -> Line {
    match Profile::load(road.profile) {
        Ok(profile) => {
            let mut line = line_with(&profile, Some(&profile), count);
            line.road = road.name;
            line
        }
        Err(error) => Line {
            road: road.name,
            profile: road.profile.to_string(),
            state: "declared",
            text: format!(
                "road {}  profile {}  error {error}",
                road.name, road.profile
            ),
            warnings: Vec::new(),
        },
    }
}

/// Each road, one line each.
pub fn lines(count: usize) -> Vec<Line> {
    ROADS.iter().map(|road| line(*road, count)).collect()
}

/// The line of a declared profile with samples from `observed`.
pub fn perturbed(declared: &str, observed: &str, count: usize) -> anyhow::Result<Line> {
    let declared = Profile::load(declared).map_err(anyhow::Error::from)?;
    let observed = Profile::load(observed).map_err(anyhow::Error::from)?;
    Ok(line_with(&declared, Some(&observed), count))
}

fn road_of(profile: &str) -> &'static str {
    ROADS
        .iter()
        .find(|road| road.profile == profile)
        .map_or("stream", |road| road.name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_profile_holds_its_own_histogram() {
        for road in ROADS {
            let line = line(road, 1000);
            assert_eq!(line.state, "observed", "{}: {line:?}", road.name);
            assert!(line.warnings.is_empty(), "{}: {line:?}", road.name);
            assert!(line.text.contains("profile"), "{}", line.text);
        }
    }

    #[test]
    fn a_perturbed_profile_diverges_with_a_named_warning() {
        let line = perturbed("page-load", "video-call", 1000).unwrap();
        assert_eq!(line.state, "diverged");
        assert!(!line.warnings.is_empty());
        let warnings = line.warning_lines();
        assert!(
            warnings[0].starts_with("warning page-load: "),
            "{warnings:?}"
        );
        assert!(warnings[0].contains("bucket"), "{warnings:?}");
    }
}
