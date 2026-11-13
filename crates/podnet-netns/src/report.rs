//! `--report FILE` (T-330): one JSON object that names the backend, the hold
//! and the UDP limit of a run, and the exit status at its end.

#[allow(unused_imports)]
use crate::prelude::*;

use std::cell::RefCell;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

/// The version of the JSON form of the report.
pub const REPORT_SCHEMA: u32 = 1;

/// The most bytes of a UDP datagram that the netns backend carries.
pub(crate) const NETNS_UDP_LIMIT: u32 = 1500;

#[derive(Default)]
struct State {
    backend: Option<BackendKind>,
    choice: Option<String>,
    reason: Option<String>,
}

/// The report of one run. With no path, each call does nothing.
pub(crate) struct Report {
    path: Option<PathBuf>,
    state: RefCell<State>,
}

impl Report {
    /// Write the first report (no backend yet, `exit` null). Fails when the
    /// file cannot be written.
    pub(crate) fn begin(path: Option<PathBuf>) -> Result<Report> {
        let report = Report {
            path,
            state: RefCell::default(),
        };
        if let Some(path) = &report.path {
            write_atomic(path, &report.json(None)).with_context(|| {
                format!("--report {}: the file cannot be written", path.display())
            })?;
        }
        Ok(report)
    }

    /// The backend that runs, and the reason of the choice. Written before
    /// the program starts.
    pub(crate) fn backend(&self, kind: BackendKind, choice: &str) {
        let mut state = self.state.borrow_mut();
        state.backend = Some(kind);
        state.choice = Some(choice.to_string());
        drop(state);
        self.write(None);
    }

    /// The backend did not start: the report names none.
    pub(crate) fn no_backend(&self) {
        self.state.borrow_mut().backend = None;
    }

    /// Record the reason of a failure.
    pub(crate) fn reason(&self, text: &str) {
        self.state.borrow_mut().reason = Some(text.to_string());
    }

    /// Print `text` on stderr as an error, record it, and return `code`.
    pub(crate) fn fail(&self, code: i32, text: &str) -> i32 {
        eprintln!("{}: {text}", crate::cli::program_name());
        self.reason(text);
        code
    }

    /// Write the last report, with the exit status.
    pub(crate) fn finish(&self, code: i32) {
        // A child of the pod can exit 3 (no backend, or no program) with
        // its own line on stderr: the parent has no text of it.
        if code == EXIT_NO_BACKEND && self.state.borrow().reason.is_none() {
            self.reason(
                "the pod did not start the program, or the program exited 3; stderr has the cause",
            );
        }
        self.write(Some(code));
    }

    fn write(&self, exit: Option<i32>) {
        let Some(path) = &self.path else { return };
        if let Err(e) = write_atomic(path, &self.json(exit)) {
            crate::warn(&format!("--report {}: {e}", path.display()));
        }
    }

    /// The object of the report.
    pub(crate) fn json(&self, exit: Option<i32>) -> String {
        let state = self.state.borrow();
        let (hold, not_held, udp_limit): (&str, &[&str], Option<u32>) = match state.backend {
            Some(BackendKind::Netns) => (
                Hold::of(true, false).netns,
                crate::doctor::NETNS_NOT_HELD,
                Some(NETNS_UDP_LIMIT),
            ),
            // T-153: the namespace holds the program as in `netns`; the
            // path has no MTU of 1500.
            Some(BackendKind::NetnsKernel) => (
                Hold::of(true, false).netns,
                crate::doctor::NETNS_NOT_HELD,
                None,
            ),
            Some(BackendKind::Seccomp) => (
                Hold::of(false, true).seccomp,
                crate::doctor::SECCOMP_NOT_HELD,
                None,
            ),
            _ => (Hold::of(false, false).netns, &[], None),
        };
        let backend = match state.backend {
            Some(BackendKind::Netns) => Some("netns"),
            Some(BackendKind::NetnsKernel) => Some("netns-kernel"),
            Some(BackendKind::Seccomp) => Some("seccomp"),
            _ => None,
        };
        let mut text = serde_json::to_string_pretty(&serde_json::json!({
            "schema": REPORT_SCHEMA,
            "version": env!("CARGO_PKG_VERSION"),
            "backend": backend,
            "choice": state.choice,
            "hold": hold,
            "not_held": not_held,
            "udp_limit": udp_limit,
            "exit": exit,
            "reason": state.reason,
        }))
        .unwrap_or_else(|_| "{}".to_string());
        text.push('\n');
        text
    }
}

/// Write `text` to `path`: a temporary file of mode 0600 in the same
/// directory, then a rename.
fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    let dir = match path.parent() {
        Some(d) if !d.as_os_str().is_empty() => d,
        _ => Path::new("."),
    };
    let name = path.file_name().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "no file name in the path")
    })?;
    let temp = dir.join(format!(
        ".{}.{}.tmp",
        name.to_string_lossy(),
        std::process::id()
    ));
    let _ = std::fs::remove_file(&temp);
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(report: &Report, exit: Option<i32>) -> serde_json::Value {
        serde_json::from_str(&report.json(exit)).unwrap()
    }

    // T-330: a run with no backend names none, holds nothing and keeps the
    // reason.
    #[test]
    fn no_backend_report_has_a_reason() {
        let report = Report::begin(None).unwrap();
        report.fail(EXIT_NO_BACKEND, "no backend can run on this host");
        let json = parse(&report, Some(EXIT_NO_BACKEND));
        assert_eq!(json["backend"], serde_json::Value::Null);
        assert_eq!(json["hold"], "none");
        assert_eq!(json["exit"], 3);
        assert_eq!(json["reason"], "no backend can run on this host");
    }

    // T-330: the hold words are the words of `pod doctor`.
    #[test]
    fn hold_words_are_the_doctor_words() {
        let report = Report::begin(None).unwrap();
        report.backend(BackendKind::Netns, "test");
        let json = parse(&report, None);
        assert_eq!(json["hold"], "enforced");
        assert_eq!(json["udp_limit"], NETNS_UDP_LIMIT);
        assert_eq!(json["exit"], serde_json::Value::Null);
        report.backend(BackendKind::Seccomp, "test");
        let json = parse(&report, None);
        assert_eq!(json["hold"], "advisory");
        assert_eq!(json["udp_limit"], serde_json::Value::Null);
    }
}
