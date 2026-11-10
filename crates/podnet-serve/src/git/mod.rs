//! Git hosting with the installed `git` (`docs/decisions.md`, Git hosting).
//!
//! * [`ssh`]: an SSH server (russh) that runs `git upload-pack`,
//!   `git receive-pack` and `git upload-archive` for a repository under a
//!   root (T-093).
//! * [`http`]: the smart HTTP router, with `git http-backend` under the
//!   CGI runner of [`cgi`] (T-092).
//! * [`parse_exec`] reads the command of an SSH `exec` request.
//! * [`repo_dir`] maps the path of a request to a directory under the
//!   root. It refuses `..`, `.`, a segment that starts with `.`, `~` and
//!   a path out of the root through a symlink.
//! * [`create_repo`] makes a bare repository for a push to a new path
//!   (`--create-on-push`, T-094).

pub mod cgi;
pub mod http;
pub mod ssh;

#[cfg(test)]
mod http_tests;
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
    let rel = repo_rel(path)?;
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

/// The path of a repository under the root, with no leading and no
/// trailing `/`. The error names a refused segment.
pub fn repo_rel(path: &str) -> Result<&str, String> {
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
        if segment.contains('\\') || segment.chars().any(char::is_control) {
            return Err(format!("the path {path:?} has a refused character"));
        }
    }
    Ok(rel)
}

/// Make the bare repository `path` under `root` with `git init --bare`
/// (`--create-on-push`). The name gets `.git` at its end when it has
/// none. A path that exists, also as `PATH.git`, is refused. The result
/// is the real directory of the new repository.
pub async fn create_repo(
    root: &Path,
    path: &str,
    git: &std::ffi::OsStr,
) -> Result<PathBuf, String> {
    let rel = repo_rel(path)?;
    let name = if rel.ends_with(".git") {
        rel.to_string()
    } else {
        format!("{rel}.git")
    };
    let root = root
        .canonicalize()
        .map_err(|err| format!("the repository root {}: {err}", root.display()))?;
    let bare = rel.strip_suffix(".git").unwrap_or(rel);
    if std::fs::symlink_metadata(root.join(bare)).is_ok()
        || std::fs::symlink_metadata(root.join(&name)).is_ok()
    {
        return Err(format!("the path {rel:?} exists and is not a repository"));
    }
    // Each parent must stay in the root (no link out of it).
    if let Some(parent) = Path::new(&name)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
    {
        let mut at = root.clone();
        for part in parent.components() {
            at.push(part);
            match std::fs::symlink_metadata(&at) {
                Ok(meta) if meta.file_type().is_symlink() => {
                    return Err(format!("the path {rel:?} goes through a link"))
                }
                Ok(meta) if !meta.is_dir() => {
                    return Err(format!("the path {rel:?} goes through a file"))
                }
                _ => {}
            }
        }
    }
    let out = tokio::process::Command::new(git)
        .args(["init", "--quiet", "--bare"])
        .arg(root.join(&name))
        .env_remove("GIT_DIR")
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .map_err(|err| format!("git does not start: {err}"))?;
    if !out.status.success() {
        return Err(format!(
            "git init {name:?}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    tracing::info!(repository = %name, "git: a new bare repository");
    repo_dir(&root, &name)
}

/// The paths of the repositories under `root`, sorted: each directory
/// with `HEAD` and `objects` (a bare repository), or with `.git` (a work
/// tree), at most [`MAX_DEPTH`] levels down. A name that starts with `.`
/// and a symlink are skipped. A repository holds no other repository.
pub fn repos(root: &Path) -> Vec<String> {
    let mut found = Vec::new();
    walk(root, "", 0, &mut found);
    found.sort();
    found
}

/// The most levels of [`repos`] under the root.
pub const MAX_DEPTH: usize = 4;

fn walk(dir: &Path, rel: &str, depth: usize, found: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if name.starts_with('.') || !entry.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let path = entry.path();
        let sub = if rel.is_empty() {
            name
        } else {
            format!("{rel}/{name}")
        };
        if (path.join("HEAD").is_file() && path.join("objects").is_dir())
            || path.join(".git").is_dir()
        {
            found.push(sub);
        } else if depth + 1 < MAX_DEPTH {
            walk(&path, &sub, depth + 1, found);
        }
    }
}

/// A value of `GIT_PROTOCOL` that git can send: `version=2` and other
/// `key=value` pairs with `:` between them.
pub(crate) fn protocol_ok(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"=:.,-_".contains(&b))
}
