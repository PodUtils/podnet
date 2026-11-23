//! FORGE-1 and FORGE-2: a repository identity is a key, not a hostname.
//!
//! [`clone_seed`] clones a local content-addressed seed with no forge
//! account and no network. The seed is a git repository on this host; its
//! object graph names the content, so the operator checks the commit hash
//! that the checkout must have. A mismatch is refused by name, before any
//! caller trusts the tree.
//!
//! The clone runs the installed `git` with `GIT_ALLOW_PROTOCOL=file`, so a
//! seed that names a remote URL cannot reach a network even when a caller
//! passes one.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::exit::{self, Fault};

/// What a successful clone verified.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cloned {
    /// The directory of the checkout.
    pub path: PathBuf,
    /// The commit hash of the checkout.
    pub hash: String,
}

/// Clone `seed` into `dest` and verify that the checkout is `expect`.
///
/// The seed is a directory of a git repository on this host. No account is
/// read and no network is allowed. A seed whose HEAD is another hash, or a
/// checkout that is another hash, is refused with both values and the seed
/// path, and the process does not report a clone.
pub fn clone_seed(seed: &Path, expect: &str, dest: &Path) -> anyhow::Result<Cloned> {
    let expect = expect.trim().to_ascii_lowercase();
    if expect.is_empty() || !expect.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(exit::fault(
            Fault::Usage,
            anyhow::anyhow!(
                "--expect {expect:?} is not a commit hash: give the 40 hex digits of the seed"
            ),
        ));
    }
    if !seed.is_dir() {
        return Err(exit::fault(
            Fault::NoInput,
            anyhow::anyhow!("--seed {}: not a directory", seed.display()),
        ));
    }
    if dest.exists() {
        return Err(exit::fault(
            Fault::Usage,
            anyhow::anyhow!("{}: the destination exists", dest.display()),
        ));
    }
    let seed_hash = head(seed).map_err(|why| {
        exit::fault(
            Fault::DataError,
            anyhow::anyhow!("--seed {}: {why}", seed.display()),
        )
    })?;
    if !seed_hash.eq_ignore_ascii_case(&expect) {
        return Err(mismatch(seed, &seed_hash, &expect, "the seed"));
    }
    let output = git(
        &[
            "clone",
            "--quiet",
            "--no-hardlinks",
            "--",
            &path_text(seed)?,
            &path_text(dest)?,
        ],
        None,
    )
    .map_err(|why| {
        exit::fault(
            Fault::Unavailable,
            anyhow::anyhow!("git clone {}: {why}", seed.display()),
        )
    })?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(exit::fault(
            Fault::DataError,
            anyhow::anyhow!(
                "git clone {}: {}",
                seed.display(),
                err.trim().replace('\n', ": ")
            ),
        ));
    }
    let hash = head(dest).map_err(|why| {
        exit::fault(
            Fault::DataError,
            anyhow::anyhow!("{}: {why}", dest.display()),
        )
    })?;
    if !hash.eq_ignore_ascii_case(&expect) {
        return Err(mismatch(dest, &hash, &expect, "the checkout"));
    }
    Ok(Cloned {
        path: dest.to_path_buf(),
        hash,
    })
}

/// The refusal of a hash that is not the key that the caller expected.
fn mismatch(where_: &Path, got: &str, expect: &str, what: &str) -> anyhow::Error {
    exit::fault(
        Fault::DataError,
        anyhow::anyhow!(
            "{what} {} has HEAD {}, but --expect is {}: the checkout hash does not match",
            where_.display(),
            grouped(got),
            grouped(expect)
        ),
    )
}

/// A hash in groups of 8 digits. A run of 32 or more hexadecimal characters
/// is hidden by the report's redaction (it reads as a token), and a hash is
/// not a secret: the groups keep every digit and stay readable.
fn grouped(hash: &str) -> String {
    let mut out = String::with_capacity(hash.len() + hash.len() / 8);
    for (i, chunk) in hash.as_bytes().chunks(8).enumerate() {
        if i > 0 {
            out.push(' ');
        }
        out.push_str(&String::from_utf8_lossy(chunk));
    }
    out
}

/// The commit hash at `dir`.
fn head(dir: &Path) -> Result<String, String> {
    let output = git(&["rev-parse", "HEAD"], Some(dir)).map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "not a git repository: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let hash = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if hash.is_empty() {
        return Err("HEAD is empty".into());
    }
    Ok(hash)
}

/// Run `git` with `args` in `dir` (the current directory when `None`), with
/// only the `file` protocol and no prompt.
fn git(args: &[&str], dir: Option<&Path>) -> std::io::Result<std::process::Output> {
    let mut command = Command::new("git");
    command
        .args(args)
        .env("GIT_ALLOW_PROTOCOL", "file")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(std::process::Stdio::null());
    if let Some(dir) = dir {
        command.current_dir(dir);
    }
    command.output()
}

/// A path as git's argv. A path that is not UTF-8 is refused rather than
/// lossily converted.
fn path_text(path: &Path) -> anyhow::Result<String> {
    path.to_str().map(str::to_string).ok_or_else(|| {
        exit::fault(
            Fault::Usage,
            anyhow::anyhow!("{}: the path is not UTF-8", path.display()),
        )
    })
}
