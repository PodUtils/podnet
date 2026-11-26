//! The crash log of podnet#500, LOG-1 to LOG-3.
//!
//! A panic hook writes one file: the backtrace, the component versions and
//! the last events. The text goes through [`crate::report::mask`] at write
//! time, and the file ends with a marker that names how many values were
//! hidden. A test seeds a secret in the environment and in an event, panics
//! a child process, and searches the file for the value.
//!
//! The log is bounded and rotated: a panic loop fills at most
//! `slots` files of `max_bytes` bytes. The oldest file is removed when
//! the next one is written, so a host with a full disk keeps its last
//! crashes and no more.

use std::collections::VecDeque;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

/// The number of files that the crash log keeps.
pub const DEFAULT_SLOTS: usize = 3;
/// The limit of one crash file, in bytes.
pub const DEFAULT_MAX_BYTES: u64 = 64 * 1024;
/// The number of events that the log keeps.
pub const EVENTS: usize = 64;
/// The name of the current file; the older ones add `.1`, `.2` and so on.
pub const FILE: &str = "crash.log";
/// The variable that sets the directory.
pub const DIR_VARIABLE: &str = "PODNET_CRASH_LOG_DIR";
/// The variable that sets the limit of one file.
pub const MAX_VARIABLE: &str = "PODNET_CRASH_LOG_MAX";
/// The variable that sets the number of files.
pub const SLOTS_VARIABLE: &str = "PODNET_CRASH_LOG_SLOTS";
/// The version of the shared crates of podcrates that this build takes: the
/// tag of `pod-core` in `Cargo.toml`, read by `build.rs`. The test
/// `the_podcrates_version_is_the_tag_of_the_manifest` compares the two.
pub const PODCRATES: &str = env!("PODNET_PODCRATES");

struct State {
    /// The directory of the log. `None` takes the state directory when a
    /// crash is written, so that a command that reads makes no directory.
    dir: Option<PathBuf>,
    max_bytes: u64,
    slots: usize,
    events: Mutex<VecDeque<String>>,
    versions: Vec<(&'static str, String)>,
}

static STATE: OnceLock<State> = OnceLock::new();
static INSTALLED: AtomicBool = AtomicBool::new(false);

fn state() -> Option<&'static State> {
    STATE.get()
}

/// The events that the hook writes, oldest first.
fn events() -> Option<&'static Mutex<VecDeque<String>>> {
    state().map(|s| &s.events)
}

/// Record one event for the next crash log. The text stays in memory and is
/// masked when the log is written.
pub fn record(event: impl Into<String>) {
    let Some(events) = events() else {
        return;
    };
    let mut events = events.lock().unwrap_or_else(|e| e.into_inner());
    if events.len() == EVENTS {
        events.pop_front();
    }
    events.push_back(event.into());
}

/// The last `n` events, oldest first, with the text that was recorded.
pub fn last_events(n: usize) -> Vec<String> {
    let Some(events) = events() else {
        return Vec::new();
    };
    let events = events.lock().unwrap_or_else(|e| e.into_inner());
    events.iter().rev().take(n).rev().cloned().collect()
}

/// Install the hook for `dir`, with the limits of the arguments. A second
/// call has no effect.
pub fn install(dir: &Path, max_bytes: u64, slots: usize) {
    install_at(Some(dir.to_path_buf()), max_bytes, slots);
}

fn install_at(dir: Option<PathBuf>, max_bytes: u64, slots: usize) {
    if INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }
    let versions = vec![
        ("podnet", env!("CARGO_PKG_VERSION").to_string()),
        ("podnet commit", env!("PODNET_BUILD_COMMIT").to_string()),
        ("podcrates", PODCRATES.to_string()),
        ("os", std::env::consts::OS.to_string()),
        ("arch", std::env::consts::ARCH.to_string()),
    ];
    let _ = STATE.set(State {
        dir,
        max_bytes: max_bytes.max(1024),
        slots: slots.max(1),
        events: Mutex::new(VecDeque::with_capacity(EVENTS)),
        versions,
    });
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        write_crash(info);
        previous(info);
    }));
}

/// Install the hook from the environment. The directory is
/// `PODNET_CRASH_LOG_DIR`, else the state directory, which is chosen when a
/// crash is written: a command that only reads makes no directory. With
/// neither, the default hook stays: a crash log needs a place for it.
pub fn install_from_env() {
    let dir = std::env::var_os(DIR_VARIABLE).map(PathBuf::from);
    let max = std::env::var(MAX_VARIABLE)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_MAX_BYTES);
    let slots = std::env::var(SLOTS_VARIABLE)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_SLOTS);
    install_at(dir, max, slots);
}

/// The path of one file of the log. Slot 0 is the current file.
pub fn path(dir: &Path, slot: usize) -> PathBuf {
    match slot {
        0 => dir.join(FILE),
        n => dir.join(format!("{FILE}.{n}")),
    }
}

/// The text of one crash, before it is written.
fn crash_text(info: &std::panic::PanicHookInfo<'_>, events: &[String]) -> String {
    let mut text = String::new();
    text.push_str("podnet crash log\n");
    text.push_str("format: podnet.crash/1\n");
    text.push_str(&format!("pid: {}\n", std::process::id()));
    let message = info
        .payload()
        .downcast_ref::<&str>()
        .map(|s| (*s).to_string())
        .or_else(|| info.payload().downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "panic with no text".into());
    text.push_str(&format!("panic: {message}\n"));
    match info.location() {
        Some(at) => text.push_str(&format!(
            "at: {}:{}:{}\n",
            at.file(),
            at.line(),
            at.column()
        )),
        None => text.push_str("at: unknown\n"),
    }
    if let Some(state) = state() {
        text.push_str("components:\n");
        for (name, version) in &state.versions {
            text.push_str(&format!("  {name}: {version}\n"));
        }
    }
    text.push_str(&format!(
        "thread: {}\n",
        std::thread::current().name().unwrap_or("unnamed")
    ));
    text.push_str("backtrace:\n");
    let backtrace = std::backtrace::Backtrace::force_capture();
    text.push_str(&format!("{backtrace}\n"));
    text.push_str(&format!("last events ({}):\n", events.len()));
    for event in events {
        text.push_str(&format!("  {event}\n"));
    }
    text
}

/// Mask the text and add the marker of the redaction.
fn redact(text: &str) -> String {
    let masked = crate::report::mask(text);
    let hidden = masked.matches("<hidden>").count();
    let marker = if hidden > 0 {
        format!("redaction: on ({hidden} value(s) hidden at write time)\n")
    } else {
        "redaction: on (0 values matched)\n".to_string()
    };
    let mut out = masked;
    out.push_str(&marker);
    out
}

fn write_crash(info: &std::panic::PanicHookInfo<'_>) {
    let Some(state) = state() else {
        return;
    };
    let events = state
        .events
        .lock()
        .map(|e| e.iter().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    let dir = match &state.dir {
        Some(dir) => Some(dir.clone()),
        None => crate::state::state_dir().ok().flatten(),
    };
    let Some(dir) = dir else {
        return;
    };
    let text = redact(&crash_text(info, &events));
    let _ = write_bounded(&dir, state.slots, state.max_bytes, text.as_bytes());
}

/// Write one crash, rotating the older files. The total is at most `slots`
/// files of `max_bytes`. A file larger than the limit is cut, so a long
/// backtrace does not break the bound.
pub fn write_bounded(dir: &Path, slots: usize, max_bytes: u64, text: &[u8]) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let slots = slots.max(1);
    let text = if text.len() as u64 > max_bytes {
        &text[..max_bytes as usize]
    } else {
        text
    };
    let current = path(dir, 0);
    let size = std::fs::metadata(&current).map(|m| m.len()).unwrap_or(0);
    if size > 0 && size + text.len() as u64 > max_bytes {
        for slot in (1..slots).rev() {
            let from = path(dir, slot - 1);
            let to = path(dir, slot);
            let _ = std::fs::remove_file(&to);
            let _ = std::fs::rename(&from, &to);
        }
        let _ = std::fs::remove_file(&current);
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&current)?;
    file.write_all(text)?;
    file.sync_all()
}

/// The files that a crash log holds, newest first, with their sizes.
pub fn files(dir: &Path, slots: usize) -> Vec<(PathBuf, u64)> {
    (0..slots)
        .map(|slot| path(dir, slot))
        .filter_map(|path| std::fs::metadata(&path).ok().map(|meta| (path, meta.len())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// LOG-3: the bound holds when the next file would pass it, and the
    /// older file stays as `.1`.
    #[test]
    fn the_log_rotates_at_its_bound() {
        let dir = tempfile::tempdir().unwrap();
        write_bounded(dir.path(), 2, 60, b"first crash\n").unwrap();
        write_bounded(
            dir.path(),
            2,
            60,
            b"second crash that is long enough to rotate the log\n",
        )
        .unwrap();
        let files = files(dir.path(), 2);
        assert_eq!(files.len(), 2, "{files:?}");
        assert!(files.iter().all(|(_, size)| *size <= 60), "{files:?}");
        assert!(path(dir.path(), 1).exists());
    }

    /// LOG-2: the marker is present and a seeded value is absent.
    #[test]
    fn the_marker_names_the_redaction() {
        crate::report::hide("seeded-crash-secret");
        let text = redact("the value seeded-crash-secret is here\n");
        assert!(!text.contains("seeded-crash-secret"), "{text}");
        assert!(text.contains("redaction: on (1 value(s) hidden"), "{text}");
    }

    /// LOG-1: the last events are kept, up to the bound.
    #[test]
    fn the_events_are_bounded() {
        let dir = tempfile::tempdir().unwrap();
        install(dir.path(), DEFAULT_MAX_BYTES, DEFAULT_SLOTS);
        for n in 0..EVENTS + 3 {
            record(format!("event {n}"));
        }
        let kept = last_events(usize::MAX);
        assert_eq!(kept.len(), EVENTS);
        assert_eq!(kept[0], "event 3");
    }

    /// The version that the crash log and the harness name is the tag of
    /// each `pod-*` dependency of the manifest (it went stale at 0.79.0).
    #[test]
    fn the_podcrates_version_is_the_tag_of_the_manifest() {
        let manifest = include_str!("../Cargo.toml");
        let tags: std::collections::BTreeSet<&str> = manifest
            .lines()
            .filter(|l| l.starts_with("pod-") && l.contains("PodUtils/podcrates"))
            .filter_map(|l| l.split("tag = \"v").nth(1)?.split('"').next())
            .collect();
        assert_eq!(tags.into_iter().collect::<Vec<_>>(), vec![PODCRATES]);
    }
}
