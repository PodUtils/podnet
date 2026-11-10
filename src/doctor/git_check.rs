//! The installed `git` (T-094): `podnet git serve` and `podnet git init`
//! run it. The doctor reports its version, or that it is missing.

use super::*;

/// The `git` program of `PATH`.
#[derive(Clone, Debug)]
pub struct Git {
    /// `ok` with a version, `fail` when `git` does not run.
    pub verdict: Verdict,
    /// The version line of `git --version`.
    pub version: Option<String>,
    pub detail: String,
}

impl Git {
    pub(super) fn to_json(&self) -> Value {
        json!({
            "verdict": self.verdict.name(),
            "status": self.verdict.name(),
            "version": self.version,
            "detail": self.detail,
        })
    }

    pub(super) fn line(&self) -> String {
        format!("\ngit    {}: {}\n", self.verdict.name(), self.detail)
    }
}

/// Run `git --version` (`program` is `git` of `PATH`).
pub(super) fn git(program: &str) -> Git {
    let out = std::process::Command::new(program)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .output();
    match out {
        Ok(out) if out.status.success() => {
            let version = String::from_utf8_lossy(&out.stdout).trim().to_string();
            Git {
                verdict: Verdict::Ok,
                detail: format!("{version}: podnet git serve can run"),
                version: Some(version),
            }
        }
        Ok(out) => Git {
            verdict: Verdict::Refused,
            version: None,
            detail: format!(
                "{program} --version exits with {}: podnet git serve needs a working git",
                out.status
            ),
        },
        Err(err) => Git {
            verdict: Verdict::Refused,
            version: None,
            detail: format!(
                "{program} is not on PATH ({err}): podnet git serve and git init need git"
            ),
        },
    }
}
