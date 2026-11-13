//! The rules of `podnet warp proxy` (issues 132 and 136): `pod-warp`'s
//! table, as one value of the process, with podnet's reload of the rules
//! file.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use anyhow::Result;

pub use pod_warp::rules::*;

/// The rules of the proxies of this process.
pub fn rules() -> &'static Arc<Rules> {
    static RULES: std::sync::LazyLock<Arc<Rules>> =
        std::sync::LazyLock::new(|| Arc::new(Rules::new(Table::default())));
    &RULES
}

/// The rules of the flags, then the rules of `file`.
pub fn build(specs: &[String], file: Option<&Path>, default: Action) -> Result<Table> {
    let (table, warnings) =
        Table::build(specs, file, default, &NoLint, true).map_err(super::failed)?;
    for warning in warnings {
        tracing::warn!("{warning}");
    }
    Ok(table)
}

/// Set the rules of the proxies of this process.
pub fn set(table: Table) {
    rules().set(table);
}

/// The rules of now (no rule, default tunnel, when none were set).
pub fn current() -> Arc<Table> {
    rules().current()
}

/// How often the rules file is checked for a change.
pub const RELOAD_EVERY: Duration = Duration::from_secs(2);

/// Read `file` again when it changes, and use its rules with the rules of
/// the flags. A file with an error is reported, and the rules before stay.
pub fn watch(specs: Vec<String>, file: PathBuf, default: Action) {
    let stamp = |p: &Path| -> Option<(SystemTime, u64)> {
        let meta = std::fs::metadata(p).ok()?;
        Some((meta.modified().ok()?, meta.len()))
    };
    tokio::spawn(async move {
        let mut last = stamp(&file);
        let mut tick = tokio::time::interval(RELOAD_EVERY);
        tick.tick().await;
        loop {
            tick.tick().await;
            let now = stamp(&file);
            if now == last {
                continue;
            }
            last = now;
            match build(&specs, Some(&file), default) {
                Ok(table) => {
                    tracing::info!(file = %file.display(), rules = table.rules.len(), "the rules of the WARP proxy are read again");
                    rules().counters.reloads.fetch_add(1, Relaxed);
                    set(table);
                }
                Err(err) => tracing::warn!(
                    error = %format!("{err:#}"),
                    "the rules file has an error; the rules before stay"
                ),
            }
        }
    });
}
