//! The verification harness of podnet#501, at the podnet surface.
//!
//! [`measure`] wraps [`pod_harness`]: it loads a fixture directory, probes
//! the capabilities of this host, runs the road by profile matrix with the
//! date of the machine, checks that each scored row is distinguishable, that
//! the budget holds and that the benign false-positive rate is under its
//! bound, and optionally writes the relocatable archive. The result is the
//! table, never a pass count.
//!
//! The text and the JSON name the conditions of the measurement: the host,
//! the versions, the input directory and the number of runs.
//!
//! A carrier that cannot run on this host is a named skip. The probe asks
//! for a raw socket before it scores the icmp rows, and for a bucket and its
//! credentials before it scores the s3 rows.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use pod_harness::capture::Capture;
use pod_harness::fixture::{Fixtures, Run, BENIGN_THRESHOLD};
use pod_harness::matrix::Benign;
use pod_harness::zip::Zip;
use pod_harness::{date, Archive, BUDGET_SECS};
use sha2::{Digest, Sha256};

use crate::exit::{self, Fault};

/// The highest benign false-positive rate that the gate accepts.
pub const BENIGN_LIMIT: f64 = 0.1;

/// The number of benign draws behind the false-positive rate.
pub const BENIGN_SAMPLES: usize = 32;

/// The versions of the components of a run.
pub const PODCRATES: &str = "0.79.0";

/// The named skip of a host that holds no raw socket.
const ICMP_SKIP: &str = "the icmp carrier needs a raw socket; this lab holds none";

/// The named skip of a host with no bucket and no credentials.
const S3_SKIP: &str = "the s3 carrier needs a bucket and credentials; this lab has neither";

/// What one run measured.
#[derive(Clone, Debug)]
pub struct Report {
    /// The rows of the run.
    pub run: Run,
    /// The fixtures that were read.
    pub data: PathBuf,
    /// The date of the machine at the run.
    pub date: String,
    /// The host of the run.
    pub host: String,
    /// The number of runs behind this table.
    pub runs: usize,
    /// The archive, when one was written.
    pub archive: Option<PathBuf>,
}

impl Report {
    /// The table and the conditions as text.
    pub fn text(&self) -> String {
        let mut out = self.run.table.render();
        out.push_str(&format!(
            "budget: {} s, elapsed {:.3} s, runs {}, input {}\n",
            BUDGET_SECS,
            self.run.elapsed.as_secs_f64(),
            self.runs,
            self.data.display()
        ));
        out.push_str(&format!(
            "benign limit: {BENIGN_LIMIT:.3} on the rate (podnet's gate)\n"
        ));
        out.push_str(&self.conditions());
        if let Some(path) = &self.archive {
            out.push_str(&format!("archive: {}\n", path.display()));
        }
        out
    }

    /// The table of the run as JSON, with the conditions beside it.
    pub fn json(&self) -> anyhow::Result<String> {
        let mut value: serde_json::Value = serde_json::from_str(&self.run.table.json())?;
        value["conditions"] = serde_json::json!({
            "host": self.host,
            "podnet": env!("CARGO_PKG_VERSION"),
            "podcrates": PODCRATES,
            "rustc": rustc_version(),
            "input": self.data.display().to_string(),
            "runs": self.runs,
        });
        value["elapsed_secs"] = serde_json::json!(self.run.elapsed.as_secs_f64());
        value["budget_secs"] = serde_json::json!(BUDGET_SECS);
        value["benign_limit"] = serde_json::json!(BENIGN_LIMIT);
        value["archive"] = match &self.archive {
            Some(path) => serde_json::json!(path.display().to_string()),
            None => serde_json::Value::Null,
        };
        Ok(serde_json::to_string_pretty(&value)?)
    }

    /// The conditions of the measurement, one per line.
    pub fn conditions(&self) -> String {
        format!(
            "conditions: host {}, podnet {}, podcrates {}, rustc {}, date {}, input {}, runs {}\n",
            self.host,
            env!("CARGO_PKG_VERSION"),
            PODCRATES,
            rustc_version(),
            self.date,
            self.data.display(),
            self.runs
        )
    }
}

/// Run the fixtures in `data`. An over-budget run, a benign rate over
/// [`BENIGN_LIMIT`], or two rows with the same scores is an error, not a
/// table that looks whole.
pub fn measure(data: &Path, archive: Option<&Path>) -> anyhow::Result<Report> {
    measure_with(data, archive, Duration::from_secs(BUDGET_SECS))
}

/// [`measure`] with the budget that decides the failure. The gate uses
/// [`BUDGET_SECS`]; a test gives a short budget and drives the refusal that
/// the gate cannot reach in its own run time.
fn measure_with(data: &Path, archive: Option<&Path>, budget: Duration) -> anyhow::Result<Report> {
    let started = Instant::now();
    let mut fixtures = Fixtures::load(data)
        .map_err(|e| exit::fault(Fault::Config, anyhow::anyhow!("{}: {e}", data.display())))?;
    apply_capability_skips(&mut fixtures);
    let today = date::today();
    let mut run = fixtures
        .run(&today)
        .map_err(|e| exit::fault(Fault::Config, anyhow::anyhow!("the harness run: {e}")))?;
    let benign = benign_rate(&fixtures)
        .map_err(|e| exit::fault(Fault::Config, anyhow::anyhow!("the benign cover site: {e}")))?;
    run.benign = benign.clone();
    run.table.set_benign(benign);
    check_distinct(&run).map_err(|e| exit::fault(Fault::Config, e))?;
    let rate = run.benign.rate();
    if rate > BENIGN_LIMIT {
        return Err(exit::fault(
            Fault::Unavailable,
            anyhow::anyhow!(
                "the benign cover site is flagged {} of {} samples, a false-positive rate of {rate:.3}, over the limit of {BENIGN_LIMIT:.3}",
                run.benign.flagged,
                run.benign.samples
            ),
        ));
    }
    run.elapsed = started.elapsed();
    if run.elapsed > budget {
        return Err(exit::fault(
            Fault::Unavailable,
            anyhow::anyhow!(
                "the harness run took {:.1} s, over the budget of {} s, so the table is not reported",
                run.elapsed.as_secs_f64(),
                budget.as_secs()
            ),
        ));
    }
    let host = hostname();
    let archive_path = match archive {
        Some(path) => {
            write_archive(&fixtures, &run, path).map_err(|e| {
                exit::fault(Fault::Config, anyhow::anyhow!("{}: {e}", path.display()))
            })?;
            Some(path.to_path_buf())
        }
        None => None,
    };
    Ok(Report {
        run,
        data: data.to_path_buf(),
        date: today,
        host,
        runs: 1,
        archive: archive_path,
    })
}

/// Whether this host lets an unprivileged process open a raw IPv4 socket.
/// The probe opens one and closes it at once; it reads nothing from the
/// network.
pub fn raw_socket_available() -> bool {
    // SAFETY: socket(2) and close(2) touch only the descriptor table.
    unsafe {
        let fd = libc::socket(libc::AF_INET, libc::SOCK_RAW, libc::IPPROTO_ICMP);
        if fd < 0 {
            return false;
        }
        let _ = libc::close(fd);
        true
    }
}

/// Replace a row that this host cannot run with a named skip. The case file
/// still holds the capture, so a host that can run the carrier scores it.
fn apply_capability_skips(fixtures: &mut Fixtures) {
    let raw = raw_socket_available();
    let s3 = s3_available(fixtures);
    for case in &mut fixtures.cases {
        match case.road.as_str() {
            "icmp" if !raw => {
                case.skip = Some(ICMP_SKIP.to_string());
                case.capture = None;
                case.faults.clear();
            }
            "s3" if !s3 => {
                case.skip = Some(S3_SKIP.to_string());
                case.capture = None;
                case.faults.clear();
            }
            _ => {}
        }
    }
}

/// Whether an s3 bucket and its credentials are present. The harness scores
/// a capture, not a live bucket, so both a capture and the bucket name are
/// needed before the s3 rows are a measurement.
fn s3_available(fixtures: &Fixtures) -> bool {
    fixtures.dir.join("captures/s3.cap").is_file() && std::env::var_os("PODNET_S3_BUCKET").is_some()
}

/// The benign cover-site rate over [`BENIGN_SAMPLES`] draws. Each draw is
/// the benign capture with a deterministic timing jitter, so the rate is a
/// count over draws rather than a bound over one sample.
fn benign_rate(fixtures: &Fixtures) -> anyhow::Result<Benign> {
    let base = Capture::load(&fixtures.dir.join("captures/benign.cap"))
        .map_err(|e| anyhow::anyhow!("captures/benign.cap: {e}"))?;
    let mut flagged = 0;
    for index in 0..BENIGN_SAMPLES {
        let sample = benign_sample(&base, index);
        let (field, _) = fixtures.field.score(&sample);
        let (flow, _) = fixtures.flow.score(&sample);
        let (prefix, _) = fixtures.prefix.score(&sample);
        if [field, flow, prefix]
            .iter()
            .any(|score| *score >= BENIGN_THRESHOLD)
        {
            flagged += 1;
        }
    }
    Ok(Benign {
        samples: BENIGN_SAMPLES,
        flagged,
        threshold: BENIGN_THRESHOLD,
    })
}

/// One benign draw: the capture with a deterministic jitter of each
/// timestamp. The order and the bytes are unchanged.
fn benign_sample(base: &Capture, index: usize) -> Capture {
    let mut sample = base.clone();
    for (number, packet) in sample.packets.iter_mut().enumerate() {
        let jitter = (index as u64 * 37 + number as u64 * 11) % 1000;
        packet.timestamp_micros = packet.timestamp_micros.saturating_add(jitter);
    }
    sample
}

/// Refuse a table in which two scored rows have the same scores. A row that
/// cannot be distinguished from another is not a measurement.
fn check_distinct(run: &Run) -> anyhow::Result<()> {
    let mut seen: BTreeMap<Vec<(String, u64)>, String> = BTreeMap::new();
    for row in &run.table.rows {
        let Some(scores) = row.scores() else {
            continue;
        };
        let key: Vec<(String, u64)> = scores
            .iter()
            .map(|score| (score.classifier.clone(), score.score.to_bits()))
            .collect();
        let name = format!("{} / {}", row.road, row.profile.as_str());
        if let Some(other) = seen.insert(key, name.clone()) {
            return Err(anyhow::anyhow!(
                "the rows {other} and {name} have identical scores; a row that cannot be distinguished is not a measurement"
            ));
        }
    }
    Ok(())
}

/// The script that the archive runs on a host with no repository and no
/// toolchain. The archive carries the binary at `bin/podnet`.
const RUN_SH: &str = r#"#!/bin/sh
# The harness of the archive. No repository and no toolchain are needed.
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
if [ "${1:-}" = "--list" ]; then
  cat "$here/manifest.txt"
  exit 0
fi
if [ ! -x "$here/bin/podnet" ]; then
  chmod +x "$here/bin/podnet"
fi
exec "$here/bin/podnet" harness --data "$here" "$@"
"#;

const README: &str = r#"# pod-harness archive

This directory is the relocatable form of one harness run. It holds the
harness binary, the fixtures, the classifier models, the middlebox simulator
source and its decisions, the expected scores of each row, and `run.sh`.

The archive needs no repository and no toolchain:

    sh run.sh
    sh run.sh --json
    sh run.sh --list

`run.sh` runs `bin/podnet harness --data .` and prints the road by
classifier table with the date. A carrier that cannot run on this host is a
named skip. The result is the table in `expected/table.txt`, not a pass
count. Each row of the table has a file in `expected/`, with its scores or
its skip. The false-positive report of the benign cover site is at the end
of the table.

`manifest.txt` lists each file with its size and its SHA-256.

`simulator/middlebox.rs` is the source of the fault model. The binary runs
the same model; `simulator/decisions.json` holds the decisions of this build.
"#;

/// Write the relocatable archive: the fixtures, the models, the simulator,
/// the expected scores, the manifest, `run.sh`, and the running binary.
fn write_archive(fixtures: &Fixtures, run: &Run, path: &Path) -> anyhow::Result<()> {
    let base = Archive::build(fixtures, run).map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut zip = Zip::new();
    for entry in base.zip().entries() {
        if matches!(entry.name.as_str(), "run.sh" | "README.md" | "manifest.txt") {
            continue;
        }
        zip.add(entry.name.clone(), entry.data.clone())
            .map_err(|e| anyhow::anyhow!("{e}"))?;
    }
    let exe = std::env::current_exe().map_err(|e| anyhow::anyhow!("the running binary: {e}"))?;
    let binary = std::fs::read(&exe).map_err(|e| anyhow::anyhow!("{}: {e}", exe.display()))?;
    zip.add_executable("bin/podnet", binary)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    zip.add_executable("run.sh", RUN_SH)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    zip.add("README.md", README)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut manifest = String::new();
    for entry in zip.entries() {
        manifest.push_str(&format!(
            "{} {:>8} {}\n",
            sha256(&entry.data),
            entry.data.len(),
            entry.name
        ));
    }
    zip.add("manifest.txt", manifest.into_bytes())
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    zip.write(path).map_err(|e| anyhow::anyhow!("{e}"))
}

fn sha256(data: &[u8]) -> String {
    pod_harness::capture::encode_hex(&Sha256::digest(data))
}

/// The name of this host, for the conditions of a run.
pub fn hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
        .or_else(|| std::env::var("HOSTNAME").ok())
        .unwrap_or_else(|| "unknown".into())
}

/// The version of the compiler that built this binary.
pub fn rustc_version() -> &'static str {
    option_env!("PODNET_RUSTC_VERSION").unwrap_or("unknown")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/harness")
    }

    #[test]
    fn the_run_names_the_date_the_profiles_and_the_skips() {
        let report = measure(&fixture_dir(), None).unwrap();
        let text = report.text();
        assert!(text.contains("road"), "{text}");
        assert!(text.contains("profile"), "{text}");
        assert!(text.contains("direct"), "{text}");
        assert!(text.contains("proxy"), "{text}");
        assert!(text.contains("restricted"), "{text}");
        assert!(text.contains(&report.date), "{text}");
        assert!(text.contains("benign cover site"), "{text}");
        assert!(text.contains("conditions: host"), "{text}");
        assert!(text.contains("skip"), "{text}");
        let json: serde_json::Value = serde_json::from_str(&report.json().unwrap()).unwrap();
        assert_eq!(json["conditions"]["runs"], 1);
        assert_eq!(json["rows"].as_array().unwrap().len(), 33);
        assert_eq!(json["benign"]["samples"], BENIGN_SAMPLES);
        assert!(json["benign"]["rate"].as_f64().unwrap() <= BENIGN_LIMIT);
        assert!(report.run.elapsed < Duration::from_secs(BUDGET_SECS));
        assert!(report.run.benign.rate() <= BENIGN_LIMIT);
    }

    #[test]
    fn a_short_budget_refuses_the_table_with_exit_69() {
        let err = measure_with(&fixture_dir(), None, Duration::ZERO).unwrap_err();
        assert_eq!(exit::code_of(&err), exit::UNAVAILABLE);
        let text = format!("{err:#}");
        assert!(text.contains("over the budget of 0 s"), "{text}");
        assert!(text.contains("not reported"), "{text}");
    }

    #[test]
    fn every_scored_row_is_distinguishable() {
        let report = measure(&fixture_dir(), None).unwrap();
        check_distinct(&report.run).unwrap();
        let mut scores = BTreeMap::new();
        for row in &report.run.table.rows {
            let Some(list) = row.scores() else {
                continue;
            };
            let key: Vec<u64> = list.iter().map(|score| score.score.to_bits()).collect();
            assert!(
                scores.insert(key, row.road.clone()).is_none(),
                "{} shares its scores",
                row.road
            );
        }
    }

    #[test]
    fn a_road_that_cannot_run_is_a_named_skip() {
        let report = measure(&fixture_dir(), None).unwrap();
        for row in &report.run.table.rows {
            if let Some(reason) = row.skip_reason() {
                assert!(!reason.trim().is_empty(), "{} has no reason", row.road);
            }
            if row.road == "s3" {
                assert_eq!(row.skip_reason(), Some(S3_SKIP));
            }
        }
        if !raw_socket_available() {
            for row in &report.run.table.rows {
                if row.road == "icmp" {
                    assert_eq!(row.skip_reason(), Some(ICMP_SKIP));
                }
            }
        }
    }

    #[test]
    fn a_missing_fixture_directory_is_a_configuration_error() {
        let err = measure(&fixture_dir().join("absent"), None).unwrap_err();
        assert_eq!(exit::code_of(&err), exit::CONFIG);
    }
}
