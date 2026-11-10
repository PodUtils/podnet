//! Git hosting with the installed `git` (`docs/decisions.md`, Git hosting).
//!
//! * [`ssh`]: an SSH server (russh) that runs `git upload-pack`,
//!   `git receive-pack` and `git upload-archive` for a repository under a
//!   root (T-093).
//! * [`parse_exec`] reads the command of an SSH `exec` request.
//! * [`repo_dir`] maps the path of a request to a directory under the
//!   root. It refuses `..`, `.`, a segment that starts with `.`, `~` and
//!   a path out of the root through a symlink.

pub mod ssh;

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

/// A service of git that a client can ask for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Service {
    /// `git-upload-pack`: clone and fetch.
    UploadPack,
    /// `git-receive-pack`: push.
    ReceivePack,
    /// `git-upload-archive`: `git archive --remote`.
    UploadArchive,
}

impl Service {
    /// The subcommand of `git` for the service.
    pub fn subcommand(self) -> &'static str {
        match self {
            Service::UploadPack => "upload-pack",
            Service::ReceivePack => "receive-pack",
            Service::UploadArchive => "upload-archive",
        }
    }

    /// True when the service changes the repository.
    pub fn writes(self) -> bool {
        self == Service::ReceivePack
    }
}

/// Read the command of an `exec` request: `git-upload-pack 'PATH'`,
/// `git-receive-pack 'PATH'` or `git-upload-archive 'PATH'`. The path has
/// the quotes of a POSIX shell (git sends `'` as `'\''`). No shell runs
/// the command. The error names the cause.
pub fn parse_exec(command: &str) -> Result<(Service, String), String> {
    let (name, arg) = command
        .split_once(' ')
        .ok_or_else(|| format!("podnet runs only git-upload-pack, git-receive-pack and git-upload-archive with one path, not {command:?}"))?;
    let service = match name {
        "git-upload-pack" => Service::UploadPack,
        "git-receive-pack" => Service::ReceivePack,
        "git-upload-archive" => Service::UploadArchive,
        _ => {
            return Err(format!(
            "podnet runs only git-upload-pack, git-receive-pack and git-upload-archive, not {name:?}"
        ))
        }
    };
    let path = unquote(arg.trim_start())?;
    if path.is_empty() {
        return Err("the command has no repository path".to_string());
    }
    Ok((service, path))
}

/// Remove the quotes of a POSIX shell from one word. A blank out of the
/// quotes (a second word) is an error.
fn unquote(word: &str) -> Result<String, String> {
    let mut out = String::new();
    let mut chars = word.chars();
    while let Some(c) = chars.next() {
        match c {
            '\'' => loop {
                match chars.next() {
                    Some('\'') => break,
                    Some(c) => out.push(c),
                    None => return Err("the path has no closing quote".to_string()),
                }
            },
            '\\' => match chars.next() {
                Some(c) => out.push(c),
                None => return Err("the path ends with a backslash".to_string()),
            },
            c if c.is_whitespace() => {
                return Err("the command has more than one argument".to_string())
            }
            c => out.push(c),
        }
    }
    if out.chars().any(char::is_control) {
        return Err("the path has a control character".to_string());
    }
    Ok(out)
}

/// Map `path` (from a request) to a directory under `root`. The leading
/// and the trailing `/` are removed. When `PATH` is not a directory and
/// `PATH.git` is, the result is `PATH.git`. The error names the cause.
pub fn repo_dir(root: &Path, path: &str) -> Result<PathBuf, String> {
    let rel = path.trim_matches('/');
    if rel.is_empty() {
        return Err("the path names no repository".to_string());
    }
    for segment in rel.split('/') {
        if segment.is_empty() {
            return Err(format!("the path {path:?} has an empty segment"));
        }
        if segment.starts_with('.') {
            return Err(format!(
                "the path {path:?} has a segment that starts with '.'"
            ));
        }
        if segment.starts_with('~') {
            return Err(format!("the path {path:?} names a home directory"));
        }
    }
    let root = root
        .canonicalize()
        .map_err(|err| format!("the repository root {}: {err}", root.display()))?;
    let mut dir = root.join(rel);
    if !dir.is_dir() {
        let with_git = root.join(format!("{rel}.git"));
        if with_git.is_dir() {
            dir = with_git;
        } else {
            return Err(format!("no repository {rel:?}"));
        }
    }
    let real = dir
        .canonicalize()
        .map_err(|err| format!("the repository {rel:?}: {err}"))?;
    if !real.starts_with(&root) {
        return Err(format!("the repository {rel:?} is out of the root"));
    }
    Ok(real)
}
