//! Where podnet keeps its files: the WARP identity, and the shims that it
//! builds.
//!
//! The state directory is the first of these that podnet can create and write:
//!
//! 1. `PODNET_STATE_DIR` (only this one when it is set; `none` keeps no file);
//! 2. `$XDG_STATE_HOME/podnet`;
//! 3. `$HOME/.local/state/podnet`;
//! 4. `$XDG_RUNTIME_DIR/podnet`;
//! 5. `podnet-<uid>` in the temporary directory.
//!
//! podnet never takes the current directory unless the user names it. Each
//! directory is made with mode 0700. A directory of the temporary directory
//! that belongs to another user, or that other users can write, is refused.
//! podnet reads the home only from `HOME`, never from the user database, so a
//! uid with no passwd entry works.

use std::ffi::OsString;
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context as _, Result};

/// The variable that names the state directory.
pub const STATE_DIR_VARIABLE: &str = "PODNET_STATE_DIR";

/// The candidates, with the name of their source, from the variables of
/// `env`.
pub fn candidates(env: &dyn Fn(&str) -> Option<OsString>) -> Option<Vec<(String, PathBuf)>> {
    let var = |key: &str| env(key).filter(|v| !v.is_empty());
    if let Some(dir) = var(STATE_DIR_VARIABLE) {
        if dir == "none" {
            return None;
        }
        return Some(vec![(STATE_DIR_VARIABLE.to_string(), PathBuf::from(dir))]);
    }
    let mut out = Vec::new();
    if let Some(dir) = var("XDG_STATE_HOME") {
        out.push((
            "$XDG_STATE_HOME".to_string(),
            PathBuf::from(dir).join("podnet"),
        ));
    }
    if let Some(home) = var("HOME") {
        out.push((
            "$HOME".to_string(),
            PathBuf::from(home).join(".local/state/podnet"),
        ));
    }
    if let Some(dir) = var("XDG_RUNTIME_DIR") {
        out.push((
            "$XDG_RUNTIME_DIR".to_string(),
            PathBuf::from(dir).join("podnet"),
        ));
    }
    let tmp = var("TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    let uid = unsafe { libc::geteuid() };
    out.push((
        "the temporary directory".to_string(),
        tmp.join(format!("podnet-{uid}")),
    ));
    Some(out)
}

/// `PODNET_FOREIGN_HOME=allow`: write in a directory of another user anyway.
pub const FOREIGN_HOME_VARIABLE: &str = "PODNET_FOREIGN_HOME";

/// The owner of `path`, or of its nearest directory that exists, with the
/// path that has that owner.
fn owner_of(path: &Path) -> Option<(u32, &Path)> {
    use std::os::unix::fs::MetadataExt as _;
    let mut at = Some(path);
    while let Some(p) = at {
        if let Ok(meta) = std::fs::metadata(p) {
            return Some((meta.uid(), p));
        }
        at = p.parent();
    }
    None
}

/// An error when `path`, a file or a directory that podnet is to write (or
/// its nearest directory that exists), belongs to another user (podssh
/// #118: root with the HOME of a user made root-owned files in that home,
/// and OpenSSH then refused the user's ~/.ssh/config).
/// `PODNET_FOREIGN_HOME=allow` writes anyway, with a warning.
pub fn check_own(path: &Path) -> Result<()> {
    check_own_with(
        path,
        unsafe { libc::geteuid() },
        std::env::var(FOREIGN_HOME_VARIABLE).ok().as_deref() == Some("allow"),
    )
}

fn check_own_with(path: &Path, me: u32, allow: bool) -> Result<()> {
    let Some((owner, found)) = owner_of(path) else {
        return Ok(());
    };
    if owner == me {
        return Ok(());
    }
    if allow {
        tracing::warn!(
            path = %path.display(),
            owner,
            "{FOREIGN_HOME_VARIABLE}=allow: podnet writes in a directory of another user"
        );
        return Ok(());
    }
    // T-529: name the path that has the other owner; `path` can be missing.
    let named = if found == path {
        path.display().to_string()
    } else {
        format!("{} (in it: {})", found.display(), path.display())
    };
    Err(crate::exit::fault(
        crate::exit::Fault::Config,
        anyhow::anyhow!(
            "{} belongs to the user {owner}, not to this user ({me}): podnet writes no file of another user (a file of root in the home of a user breaks it). Run as that user (sudo -u USER, or sudo -H), give another path, or set {FOREIGN_HOME_VARIABLE}=allow",
            named
        ),
    ))
}

/// The state directory of this process, or `None` with `PODNET_STATE_DIR=none`.
/// The error names each path that podnet tried, and why it failed.
pub fn state_dir() -> Result<Option<PathBuf>> {
    choose(&|key| std::env::var_os(key))
}

/// [`state_dir`] with the variables of `env`.
pub fn choose(env: &dyn Fn(&str) -> Option<OsString>) -> Result<Option<PathBuf>> {
    let (dir, skipped) = choose_with(env, &check_own)?;
    for line in skipped {
        if warn_first(&line) {
            tracing::warn!("{line}");
        }
    }
    Ok(dir)
}

/// True the first time that this process gives `line` (T-529: each caller
/// of [`state_dir`] chooses again, and `warp register` has three).
fn warn_first(line: &str) -> bool {
    static SEEN: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
    let mut seen = SEEN
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if seen.iter().any(|s| s == line) {
        return false;
    }
    seen.push(line.to_string());
    true
}

/// [`choose`] with the check of the owner in `check`: the directory, and a
/// warning for each directory of another user that it skips, which names
/// the directory that podnet uses.
fn choose_with(
    env: &dyn Fn(&str) -> Option<OsString>,
    check: &dyn Fn(&Path) -> Result<()>,
) -> Result<(Option<PathBuf>, Vec<String>)> {
    let Some(candidates) = candidates(env) else {
        return Ok((None, Vec::new()));
    };
    let mut failures = Vec::new();
    let mut skipped = Vec::new();
    for (source, dir) in candidates {
        // A HOME of another user is skipped, with a warning (podssh #118).
        if source == "$HOME" || source == "$XDG_STATE_HOME" {
            if let Err(err) = check(&dir) {
                skipped.push(format!("{err:#}"));
                failures.push(format!("{} ({source}): {err:#}", dir.display()));
                continue;
            }
        }
        match prepare(&dir) {
            Ok(()) => {
                let used = format!("; podnet uses {}", dir.display());
                let skipped = skipped.into_iter().map(|line| line + &used).collect();
                return Ok((Some(dir), skipped));
            }
            Err(err) => failures.push(format!("{} ({source}): {err:#}", dir.display())),
        }
    }
    bail!(
        "no state directory can be written; set {STATE_DIR_VARIABLE} to a directory, or to none to keep no file. Tried: {}",
        failures.join("; ")
    )
}

/// True when `path` exists, false when it does not. Any other error of the
/// stat (EACCES, ENOTDIR, ELOOP) is returned with the path: before (issue
/// 178), `Path::exists` gave false for it, and a new WARP device could be
/// written over a file that podnet could not read.
pub fn exists(path: &Path) -> Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(anyhow::Error::new(err).context(format!("reading {}", path.display()))),
    }
}

/// The file `name` in the first candidate directory that has it, with no
/// directory made (issue 178: `warp status` made the state directory).
/// `Ok(None)` when no candidate has it, or with `PODNET_STATE_DIR=none`.
pub fn find_existing(name: &str) -> Result<Option<PathBuf>> {
    find_existing_in(&|key| std::env::var_os(key), name)
}

/// [`find_existing`] with the variables of `env`.
pub fn find_existing_in(
    env: &dyn Fn(&str) -> Option<OsString>,
    name: &str,
) -> Result<Option<PathBuf>> {
    let Some(candidates) = candidates(env) else {
        return Ok(None);
    };
    for (_, dir) in candidates {
        let path = dir.join(name);
        if exists(&path)? {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

/// Make `dir` (mode 0700), check its owner and its mode, and write a file
/// in it.
pub fn prepare(dir: &Path) -> Result<()> {
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .context("making the directory")?;
    let meta = std::fs::metadata(dir).context("reading the directory")?;
    let uid = unsafe { libc::geteuid() };
    if meta.uid() != uid {
        bail!(
            "the directory belongs to user {}, not to this user ({uid})",
            meta.uid()
        );
    }
    if meta.mode() & 0o022 != 0 {
        bail!(
            "other users can write the directory (mode {:04o})",
            meta.mode() & 0o7777
        );
    }
    writable(dir)
}

/// Write and remove a file in `dir`.
pub fn writable(dir: &Path) -> Result<()> {
    let probe = dir.join(format!(".podnet-probe-{}", std::process::id()));
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
        .context("writing a file")?;
    let _ = std::fs::remove_file(&probe);
    Ok(())
}

/// Check that podnet can write the file `path` later: its directory can be
/// made and written. Call it before work that the file must keep (a WARP
/// registration), so that a failure costs nothing.
pub fn check_file_writable(path: &Path) -> Result<()> {
    let dir = match path.parent().filter(|p| !p.as_os_str().is_empty()) {
        Some(dir) => dir,
        None => Path::new("."),
    };
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .and_then(|_| writable(dir).map_err(std::io::Error::other))
        .with_context(|| format!("{} cannot be written", dir.display()))
}

/// True when the file system of `dir` lets podnet run or map code from it
/// (no `noexec` mount).
pub fn allows_exec(dir: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = std::ffi::CString::new(dir.as_os_str().as_bytes()) else {
        return false;
    };
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return false;
    }
    st.f_flag & libc::ST_NOEXEC == 0
}

/// T-525: the directory of a shim: `out` when the user gives it, else
/// [`exec_dir`] of `name`. A shim cannot load from a `noexec` file system:
/// an explicit `out` there is an error (exit 78), with no silent fallback,
/// because the user writes that path into `LD_PRELOAD`.
pub fn shim_dir(out: Option<PathBuf>, name: &str) -> Result<PathBuf> {
    shim_dir_with(out, name, allows_exec)
}

/// [`shim_dir`] with the check of the file system in `allows`.
fn shim_dir_with(
    out: Option<PathBuf>,
    name: &str,
    allows: impl Fn(&Path) -> bool,
) -> Result<PathBuf> {
    let Some(out) = out else {
        return exec_dir(name);
    };
    // The directory can be made later: check the nearest one that exists.
    let there = out
        .ancestors()
        .find(|dir| dir.exists())
        .unwrap_or(Path::new("."));
    if allows(there) {
        return Ok(out);
    }
    let other = match exec_dir(name) {
        Ok(dir) => format!(" (podnet uses {})", dir.display()),
        Err(_) => String::new(),
    };
    Err(crate::exit::fault(
        crate::exit::Fault::Config,
        anyhow::anyhow!(
            "{} is on a noexec file system: a shim cannot load from it; give another --out, or none{other}",
            out.display()
        ),
    ))
}

/// A directory for a file that must run or be loaded (a shim), named
/// `name`: the first of the temporary directory, the state directory and
/// `$XDG_RUNTIME_DIR` that is private, writable and not `noexec`.
pub fn exec_dir(name: &str) -> Result<PathBuf> {
    let mut candidates = vec![std::env::temp_dir().join(name)];
    if let Ok(Some(state)) = state_dir() {
        candidates.push(state.join(name));
    }
    if let Some(run) = std::env::var_os("XDG_RUNTIME_DIR").filter(|v| !v.is_empty()) {
        candidates.push(PathBuf::from(run).join(name));
    }
    exec_dir_from(candidates)
}

/// [`exec_dir`] with the candidates in `candidates`. Issue 247: a
/// directory that exists already must be a directory of this user (not a
/// symlink) that other users cannot write, because podnet loads the code in
/// it.
fn exec_dir_from(candidates: Vec<PathBuf>) -> Result<PathBuf> {
    let mut tried = Vec::new();
    for dir in candidates {
        if let Err(err) = std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&dir)
        {
            tried.push(format!("{}: {err}", dir.display()));
            continue;
        }
        if let Err(err) = check_private_dir(&dir) {
            tried.push(format!("{}: {err:#}", dir.display()));
            continue;
        }
        if !allows_exec(&dir) {
            tried.push(format!("{}: the file system is noexec", dir.display()));
            continue;
        }
        if let Err(err) = writable(&dir) {
            tried.push(format!("{}: {err:#}", dir.display()));
            continue;
        }
        return Ok(dir);
    }
    bail!(
        "no directory can hold a shim: {}; use --out",
        tried.join("; ")
    )
}

/// Check that `dir` is a directory of this user, not a symlink, and that
/// other users cannot write it.
pub fn check_private_dir(dir: &Path) -> Result<()> {
    let meta = std::fs::symlink_metadata(dir).context("reading the directory")?;
    if meta.file_type().is_symlink() {
        bail!("the path is a symlink");
    }
    if !meta.is_dir() {
        bail!("the path is not a directory");
    }
    let uid = unsafe { libc::geteuid() };
    if meta.uid() != uid {
        bail!(
            "the directory belongs to user {}, not to this user ({uid})",
            meta.uid()
        );
    }
    if meta.mode() & 0o022 != 0 {
        bail!(
            "other users can write the directory (mode {:04o})",
            meta.mode() & 0o7777
        );
    }
    Ok(())
}

/// True when `path` is a regular file of this user (not a symlink) that
/// other users cannot write.
pub fn own_file(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|meta| {
        meta.file_type().is_file()
            && meta.uid() == unsafe { libc::geteuid() }
            && meta.mode() & 0o022 == 0
    })
}

/// Write `bytes` to `path` with the mode `mode`. The data goes to a new
/// file with a unique name in the same directory, and a rename puts it at
/// `path`. A symlink or a file of another user at `path` is replaced, not
/// followed.
pub fn replace_file(path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    // T-417: pod-core's one durable replace (PodUtils/podnet#437): the file
    // and its directory are synced, so a power cut leaves the old file or
    // the new one, not an empty one.
    use pod_core::files::{replace_private, Commit};
    match replace_private(path, bytes, mode, false)
        .with_context(|| format!("writing {}", path.display()))?
    {
        Commit::Durable => {}
        Commit::InstalledUnconfirmed(err) | Commit::InPlace(err) => tracing::warn!(
            path = %path.display(),
            "the file is written, but the sync of its directory failed ({err}): a power cut can lose it"
        ),
    }
    Ok(())
}

/// A new name beside `path` for a temporary file: `.NAME.PID.N.tmp`.
pub fn temp_name(path: &Path) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let name = path.file_name().map_or_else(
        || std::ffi::OsString::from("file"),
        |name| name.to_os_string(),
    );
    let mut temp = std::ffi::OsString::from(".");
    temp.push(name);
    temp.push(format!(".{}.{n}.tmp", std::process::id()));
    path.with_file_name(temp)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T-525 (GitHub #699): an explicit --out on a noexec file system gives
    /// exit 78 and one line; an exec file system or no --out passes.
    #[test]
    fn state_shim_dir_refuses_an_explicit_noexec_out() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("podnet-vnet");
        for (given, exec, refused) in [
            (Some(out.clone()), false, true),
            (Some(out.clone()), true, false),
            (None, false, false),
        ] {
            let got = shim_dir_with(given.clone(), "podnet-vnet", |_| exec);
            match (refused, got) {
                (true, Err(err)) => {
                    assert_eq!(crate::exit::code_of(&err), 78, "{err:#}");
                    let line = format!("{err:#}");
                    assert!(line.contains(&out.display().to_string()), "{line}");
                    assert!(line.contains("noexec") && !line.contains('\n'), "{line}");
                }
                (false, Ok(path)) => assert!(given.is_none() || path == out, "{path:?}"),
                (refused, got) => panic!("{given:?} exec={exec}: refused={refused}, {got:?}"),
            }
        }
    }

    /// T-529 (GitHub #704): the warning names the directory that has the
    /// other owner and the directory that podnet uses, and comes once.
    #[test]
    fn state_a_foreign_home_is_named_once_with_the_used_directory() {
        let root = tempfile::tempdir().unwrap();
        let (home, tmp) = (root.path().join("home"), root.path().join("tmp"));
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&tmp).unwrap();
        let me = unsafe { libc::geteuid() };
        let env = |key: &str| match key {
            "HOME" => Some(home.clone().into_os_string()),
            "TMPDIR" => Some(tmp.clone().into_os_string()),
            _ => None,
        };
        let used = tmp.join(format!("podnet-{me}"));
        let state = home.join(".local/state/podnet");
        let other = |p: &Path| check_own_with(p, me + 1, false);
        let mine = |p: &Path| check_own_with(p, me, false);
        // HOME of another user, .local missing; then .local of another user.
        for (layout, owner_path) in [
            ("no .local", home.clone()),
            (".local made", home.join(".local")),
        ] {
            if layout == ".local made" {
                std::fs::create_dir_all(home.join(".local")).unwrap();
            }
            let (dir, warnings) = choose_with(&env, &other).unwrap();
            assert_eq!(dir.as_deref(), Some(used.as_path()), "{layout}");
            assert_eq!(warnings.len(), 1, "{layout}: {warnings:?}");
            let want = format!(
                "{} (in it: {}) belongs to the user {me}, not to this user ({})",
                owner_path.display(),
                state.display(),
                me + 1
            );
            assert!(warnings[0].starts_with(&want), "{layout}: {}", warnings[0]);
            assert!(
                warnings[0].ends_with(&format!("; podnet uses {}", used.display())),
                "{layout}: {}",
                warnings[0]
            );
        }
        // HOME of this user: no warning.
        let (dir, warnings) = choose_with(&env, &mine).unwrap();
        assert_eq!(dir.as_deref(), Some(state.as_path()));
        assert!(warnings.is_empty(), "{warnings:?}");
        // Two calls in one process: one warning.
        let line = format!("T-529 test {}", root.path().display());
        assert!(warn_first(&line));
        assert!(!warn_first(&line));
    }

    // podssh #118: a directory of another user is refused; allow writes.
    #[test]
    fn a_directory_of_another_user_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let me = unsafe { libc::geteuid() };
        let path = dir.path().join("a/b/c.conf");
        assert!(check_own_with(&path, me, false).is_ok());
        let err = check_own_with(&path, me + 1, false).unwrap_err();
        assert_eq!(crate::exit::code_of(&err), 78);
        assert!(
            format!("{err:#}").contains("PODNET_FOREIGN_HOME=allow"),
            "{err:#}"
        );
        assert!(check_own_with(&path, me + 1, true).is_ok());
    }

    // Issue 178: a stat error is not a missing file.
    #[test]
    fn a_stat_error_is_not_a_missing_file() {
        let dir = std::env::temp_dir().join(format!("podnet-exists-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("plain");
        std::fs::write(&file, "x").unwrap();
        // FILE/warp.json: ENOTDIR (also as root, where mode 000 is no test).
        assert!(exists(&file.join("warp.json")).is_err());
        assert!(!exists(&dir.join("missing.json")).unwrap());
        assert!(exists(&file).unwrap());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // Issue 178: a search makes no directory.
    #[test]
    fn find_existing_creates_nothing() {
        let root = std::env::temp_dir().join(format!("podnet-find-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let env = |key: &str| match key {
            "XDG_STATE_HOME" => Some(root.join("state").into_os_string()),
            "HOME" => Some(root.join("home").into_os_string()),
            "TMPDIR" => Some(root.join("tmp").into_os_string()),
            _ => None,
        };
        assert!(find_existing_in(&env, "warp.json").unwrap().is_none());
        assert!(!root.exists());
        // A file in the second candidate is found.
        let second = root.join("home/.local/state/podnet");
        std::fs::create_dir_all(&second).unwrap();
        std::fs::write(second.join("warp.json"), "{}").unwrap();
        assert_eq!(
            find_existing_in(&env, "warp.json").unwrap(),
            Some(second.join("warp.json"))
        );
        assert!(!root.join("state").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }
    use std::collections::HashMap;
    use std::os::unix::fs::PermissionsExt;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let map: HashMap<String, OsString> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), OsString::from(v)))
            .collect();
        move |key| map.get(key).cloned()
    }

    #[test]
    fn with_no_home_the_chain_uses_the_temporary_directory() {
        let dir = tempfile::tempdir().unwrap();
        let tmp = dir.path().to_str().unwrap();
        let chosen = choose(&env(&[("TMPDIR", tmp)])).unwrap().unwrap();
        assert!(chosen.starts_with(tmp), "{}", chosen.display());
        assert!(chosen
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("podnet-"));
        let cwd = std::env::current_dir().unwrap();
        assert!(!chosen.starts_with(&cwd) || cwd.starts_with(tmp));
        let mode = std::fs::metadata(&chosen).unwrap().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }

    #[test]
    fn the_order_of_the_chain() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().to_str().unwrap();
        let state = format!("{base}/state");
        let home = format!("{base}/home");
        let chosen = choose(&env(&[("XDG_STATE_HOME", &state), ("HOME", &home)]))
            .unwrap()
            .unwrap();
        assert_eq!(chosen, PathBuf::from(&state).join("podnet"));
        let chosen = choose(&env(&[("HOME", &home)])).unwrap().unwrap();
        assert_eq!(chosen, PathBuf::from(&home).join(".local/state/podnet"));
        let own = format!("{base}/own");
        let chosen = choose(&env(&[(STATE_DIR_VARIABLE, &own), ("HOME", &home)]))
            .unwrap()
            .unwrap();
        assert_eq!(chosen, PathBuf::from(&own));
    }

    #[test]
    fn none_keeps_no_file() {
        assert_eq!(
            choose(&env(&[(STATE_DIR_VARIABLE, "none"), ("HOME", "/")])).unwrap(),
            None
        );
    }

    /// A path under a file cannot be a directory, also for root. The error
    /// names each path.
    #[test]
    fn with_no_writable_candidate_the_error_names_each_path() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a-file");
        std::fs::write(&file, b"").unwrap();
        let f = file.to_str().unwrap();
        let err = choose(&env(&[
            ("XDG_STATE_HOME", f),
            ("HOME", f),
            ("XDG_RUNTIME_DIR", f),
            ("TMPDIR", f),
        ]))
        .unwrap_err()
        .to_string();
        for want in [
            "$XDG_STATE_HOME",
            "$HOME",
            "$XDG_RUNTIME_DIR",
            "the temporary directory",
            "PODNET_STATE_DIR",
        ] {
            assert!(err.contains(want), "{want} is not in: {err}");
        }
    }

    #[test]
    fn a_directory_that_others_can_write_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let open = dir.path().join("open");
        std::fs::create_dir(&open).unwrap();
        std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o777)).unwrap();
        let err = prepare(&open).unwrap_err().to_string();
        assert!(err.contains("other users can write"), "{err}");
    }

    #[test]
    fn a_file_that_must_be_written_later_is_checked_first() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a-file");
        std::fs::write(&file, b"").unwrap();
        assert!(check_file_writable(&file.join("warp.json")).is_err());
        assert!(check_file_writable(&dir.path().join("new/warp.json")).is_ok());
    }

    // Issue 247: an exec directory that exists and that other users can
    // write, or that is a symlink, is not used.
    #[test]
    fn exec_dir_refuses_a_directory_open_to_others() {
        let dir = tempfile::tempdir().unwrap();
        let open = dir.path().join("open");
        std::fs::create_dir(&open).unwrap();
        std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o777)).unwrap();
        let link = dir.path().join("link");
        let target = dir.path().join("target");
        std::fs::create_dir(&target).unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let good = dir.path().join("good");
        let chosen = exec_dir_from(vec![open.clone(), link.clone(), good.clone()]).unwrap();
        assert_eq!(chosen, good);
        assert_eq!(std::fs::metadata(&good).unwrap().mode() & 0o777, 0o700);
        let err = exec_dir_from(vec![open, link]).unwrap_err().to_string();
        assert!(err.contains("other users can write"), "{err}");
        assert!(err.contains("symlink"), "{err}");
    }

    // Issue 247: a write replaces a symlink and does not follow it.
    #[test]
    fn replace_file_does_not_follow_a_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let victim = dir.path().join("victim");
        std::fs::write(&victim, b"keep").unwrap();
        let path = dir.path().join("passwd");
        std::os::unix::fs::symlink(&victim, &path).unwrap();
        replace_file(&path, b"new", 0o644).unwrap();
        assert_eq!(std::fs::read(&victim).unwrap(), b"keep");
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        assert!(own_file(&path));
        let meta = std::fs::symlink_metadata(&path).unwrap();
        assert_eq!(meta.mode() & 0o777, 0o644);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    }

    #[test]
    fn the_temporary_directory_allows_exec_here() {
        // The test only reads the flag; a noexec /tmp makes exec_dir go on.
        let _ = allows_exec(&std::env::temp_dir());
        assert!(!allows_exec(Path::new("/no/such/dir")));
    }
}
