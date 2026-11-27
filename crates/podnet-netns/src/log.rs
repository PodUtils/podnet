//! Stderr log formatting for `podnet pod` (T-632): plain `podnet pod: ...`
//! lines by default, or one JSON object per line when `--log-format json` is
//! active.

use std::io::Write as _;
use std::sync::atomic::{AtomicBool, Ordering};

static LOG_JSON: AtomicBool = AtomicBool::new(false);

/// Set whether `podnet-netns` writes stderr lines as JSON (`--log-format json`).
pub fn set_log_json(on: bool) {
    LOG_JSON.store(on, Ordering::Relaxed);
}

/// True when `podnet-netns` writes stderr lines as JSON.
pub fn log_json() -> bool {
    LOG_JSON.load(Ordering::Relaxed)
}

/// Format one JSON log line with an RFC 3339 UTC timestamp (microseconds),
/// `level`, `fields.message`, optional `fields.exit_code`, and `target`.
pub(crate) fn json_line(level: &str, text: &str, exit_code: Option<i32>) -> String {
    let message = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .collect::<Vec<_>>()
        .join("; ");
    let now = std::time::SystemTime::now();
    let micros = now
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_micros());
    let second = pod_core::clock::rfc3339(now);
    let timestamp = format!("{}.{micros:06}Z", second.trim_end_matches('Z'));
    let fields = match exit_code {
        Some(code) => serde_json::json!({
            "message": message.trim_end(),
            "exit_code": code,
        }),
        None => serde_json::json!({
            "message": message.trim_end(),
        }),
    };
    serde_json::json!({
        "timestamp": timestamp,
        "level": level,
        "fields": fields,
        "target": "podnet_netns",
    })
    .to_string()
}

/// Write a line to stderr at `level` (`"INFO"`, `"WARN"`, or `"ERROR"`).
pub(crate) fn write(level: &str, text: &str) {
    if log_json() {
        let line = json_line(level, text, None);
        let _ = writeln!(std::io::stderr(), "{line}");
    } else {
        let _ = writeln!(std::io::stderr(), "{}: {text}", crate::cli::program_name());
    }
}

/// Write a final error line with `exit_code`.
pub(crate) fn write_exit(code: i32, text: &str) {
    if log_json() {
        let line = json_line("ERROR", text, Some(code));
        let _ = writeln!(std::io::stderr(), "{line}");
    } else {
        let _ = writeln!(std::io::stderr(), "{}: {text}", crate::cli::program_name());
    }
}

/// Write an informational line to stderr.
pub(crate) fn info(text: &str) {
    write("INFO", text);
}

/// Write a warning line to stderr.
pub(crate) fn warn(text: &str) {
    write("WARN", text);
}

/// Write an error line to stderr.
pub(crate) fn error(text: &str) {
    write("ERROR", text);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_line_has_expected_fields_and_optional_exit_code() {
        let warn = json_line("WARN", "first line\nsecond line", None);
        let v: serde_json::Value = serde_json::from_str(&warn).unwrap();
        assert_eq!(v["level"], "WARN");
        assert_eq!(v["target"], "podnet_netns");
        assert_eq!(v["fields"]["message"], "first line; second line");
        assert!(v["fields"].get("exit_code").is_none());
        assert!(v["timestamp"].as_str().unwrap().ends_with('Z'));

        let err = json_line("ERROR", "no proxy", Some(64));
        let v: serde_json::Value = serde_json::from_str(&err).unwrap();
        assert_eq!(v["level"], "ERROR");
        assert_eq!(v["fields"]["message"], "no proxy");
        assert_eq!(v["fields"]["exit_code"], 64);
    }
}
