//! The resolver file of `podnet warp tun --default-route`.
//!
//! Before the first change, `set_file` writes a record of the original file
//! to the state directory (mode 0600, synced). A restart or `warp down`
//! restores the original when the process was killed. The record is applied
//! only when the file still starts with the marker line.

use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context as _, Result};
use serde::{Deserialize, Serialize};

/// The first line of the file that podnet writes.
pub const MARKER: &str = "# podnet warp tun --default-route: DNS through WARP";

const RESOLV: &str = "/etc/resolv.conf";
const VERSION: u32 = 1;
/// The mode of a new resolver file when the original has no mode.
const MODE: u32 = 0o644;

/// What `Routes` changed in the resolver.
#[derive(Debug)]
pub enum Dns {
    Resolved,
    File {
        paths: Paths,
        /// The lock of the record; held until `Routes` drops.
        _lock: crate::instances::Held,
    },
}

/// The files of the resolver change.
#[derive(Debug, Clone)]
pub struct Paths {
    pub resolv: PathBuf,
    pub record: PathBuf,
    pub lock: PathBuf,
}

impl Paths {
    /// `/etc/resolv.conf` and the files in the state directory.
    pub fn system() -> Result<Self> {
        let dir = match crate::state::state_dir()? {
            Some(dir) => dir,
            None => crate::instances::run_dir()?,
        };
        Ok(Self::in_dir(RESOLV.into(), &dir))
    }

    pub fn in_dir(resolv: PathBuf, dir: &Path) -> Self {
        Self {
            resolv,
            record: dir.join("resolv.record.json"),
            lock: dir.join("resolv.lock"),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Record {
    version: u32,
    path: PathBuf,
    /// `file`, `symlink` or `absent`.
    kind: String,
    text: Option<String>,
    link_target: Option<PathBuf>,
    mode: Option<u32>,
}

/// Change the resolver file to `text`, after a record of the original.
pub fn set_file(paths: &Paths, text: &str) -> Result<Dns> {
    let Some(lock) = crate::instances::try_lock(&paths.lock)? else {
        bail!("another warp tun runs (it holds {})", paths.lock.display());
    };
    // The lock is ours: a record on disk is from a process that ended.
    reconcile_held(paths)?;
    let record = read_original(&paths.resolv)?;
    write_record(&paths.record, &record)?;
    replace_file(&paths.resolv, text, record.mode.unwrap_or(MODE))?;
    Ok(Dns::File {
        paths: paths.clone(),
        _lock: lock,
    })
}

fn read_original(path: &Path) -> Result<Record> {
    let mut record = Record {
        version: VERSION,
        path: path.to_path_buf(),
        kind: "absent".into(),
        text: None,
        link_target: None,
        mode: None,
    };
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            record.kind = "symlink".into();
            record.link_target = Some(std::fs::read_link(path)?);
        }
        Ok(meta) => {
            record.kind = "file".into();
            record.text = Some(std::fs::read_to_string(path).unwrap_or_default());
            record.mode = Some(meta.permissions().mode() & 0o7777);
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(err).with_context(|| format!("reading {}", path.display())),
    }
    Ok(record)
}

fn write_record(path: &Path, record: &Record) -> Result<()> {
    use std::io::Write as _;
    let temp = path.with_extension("tmp");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&temp)
        .with_context(|| format!("writing {}", temp.display()))?;
    file.write_all(serde_json::to_string(record)?.as_bytes())?;
    file.sync_all()?;
    std::fs::rename(&temp, path).with_context(|| format!("writing {}", path.display()))?;
    if let Some(dir) = path.parent() {
        let _ = std::fs::File::open(dir).and_then(|d| d.sync_all());
    }
    Ok(())
}

/// Restore a record of a process that ended, when no live tun holds the lock.
pub fn reconcile(paths: &Paths) -> Result<()> {
    match crate::instances::try_lock(&paths.lock)? {
        Some(_held) => reconcile_held(paths),
        None => Ok(()),
    }
}

/// [`reconcile`] for `/etc/resolv.conf`; a failure is a message.
pub fn reconcile_system() {
    if let Err(err) = Paths::system().and_then(|paths| reconcile(&paths)) {
        eprintln!("podnet: the resolver file is not restored: {err:#}");
    }
}

/// [`reconcile`] for the caller that holds the lock.
fn reconcile_held(paths: &Paths) -> Result<()> {
    let Ok(text) = std::fs::read_to_string(&paths.record) else {
        return Ok(());
    };
    let record: Record = serde_json::from_str(&text)
        .with_context(|| format!("reading {}", paths.record.display()))?;
    if record.version != VERSION {
        bail!(
            "{} has the version {}; this podnet knows the version {VERSION}. The record is left in place",
            paths.record.display(),
            record.version
        );
    }
    let ours = std::fs::read_to_string(&paths.resolv).is_ok_and(|t| t.starts_with(MARKER));
    if ours {
        restore(&record)?;
        tracing::info!(path = %record.path.display(), "the resolver file is restored");
    } else {
        tracing::warn!(path = %record.path.display(), "resolv.conf was changed by someone else; left as found");
    }
    let _ = std::fs::remove_file(&paths.record);
    Ok(())
}

fn restore(record: &Record) -> Result<()> {
    let path = &record.path;
    match record.kind.as_str() {
        "symlink" => {
            let target = record.link_target.clone().unwrap_or_default();
            let _ = std::fs::remove_file(path);
            std::os::unix::fs::symlink(&target, path)
                .with_context(|| format!("restoring {}", path.display()))
        }
        "file" => replace_file(
            path,
            record.text.as_deref().unwrap_or_default(),
            record.mode.unwrap_or(MODE),
        ),
        _ => {
            let _ = std::fs::remove_file(path);
            Ok(())
        }
    }
}

/// Put the resolver back when `Routes` drops.
pub fn undo(paths: &Paths) {
    if let Err(err) = reconcile_held(paths) {
        tracing::warn!(error = %format!("{err:#}"), "the resolver file is not restored");
    }
}

/// Write `text` as the file `path` with the mode `mode` (#278). A new file
/// in the same directory is renamed over the old one: a failure keeps the
/// old file, and a symbolic link is replaced, not written through. Where a
/// rename over the file is not possible (a bind mount: EBUSY, EXDEV), the
/// file is written in place.
pub fn replace_file(path: &Path, text: &str, mode: u32) -> Result<()> {
    let temp = path.with_extension(format!("podnet-{}", std::process::id()));
    std::fs::write(&temp, text).with_context(|| format!("writing {}", temp.display()))?;
    // The umask does not change the mode.
    std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(mode))
        .with_context(|| format!("setting the mode of {}", temp.display()))?;
    match std::fs::rename(&temp, path) {
        Ok(()) => Ok(()),
        Err(err) => {
            let _ = std::fs::remove_file(&temp);
            tracing::debug!(error = %err, path = %path.display(), "no rename over the file; it is written in place");
            std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORIGINAL: &str = "nameserver 10.0.0.2\nsearch corp\n";

    fn setup(dir: &Path) -> Paths {
        let paths = Paths::in_dir(dir.join("resolv.conf"), dir);
        std::fs::write(&paths.resolv, ORIGINAL).unwrap();
        paths
    }

    fn ours() -> String {
        format!("{MARKER}\nnameserver 1.1.1.1\n")
    }

    /// #278: the file is replaced by a rename: a link is replaced, not
    /// written through, and a failed write keeps the old file.
    #[test]
    fn the_resolv_conf_is_replaced_and_never_removed_first() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("stub.conf");
        std::fs::write(&target, "nameserver 127.0.0.53\n").unwrap();
        let path = dir.path().join("resolv.conf");
        std::os::unix::fs::symlink(&target, &path).unwrap();
        replace_file(&path, "nameserver 1.1.1.1\n", MODE).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "nameserver 1.1.1.1\n"
        );
        assert!(!std::fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            "nameserver 127.0.0.53\n"
        );
        let missing = dir.path().join("no-such-dir").join("resolv.conf");
        assert!(replace_file(&missing, "x", MODE).is_err());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "nameserver 1.1.1.1\n"
        );
    }

    #[test]
    fn replace_file_sets_the_mode() {
        let dir = tempfile::tempdir().unwrap();
        for umask in [0o077, 0o000] {
            let path = dir.path().join(format!("resolv-{umask}"));
            // SAFETY: umask only changes the mask of this process.
            let old = unsafe { libc::umask(umask) };
            let done = replace_file(&path, "x\n", 0o644);
            // SAFETY: as above.
            unsafe { libc::umask(old) };
            done.unwrap();
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o7777;
            assert_eq!(mode, 0o644, "umask {umask:o}");
        }
    }

    #[test]
    fn resolv_backup_survives_a_kill() {
        let dir = tempfile::tempdir().unwrap();
        let paths = setup(dir.path());
        // A kill: the value goes away with no restore; the lock is free.
        drop(set_file(&paths, &ours()).unwrap());
        assert!(std::fs::read_to_string(&paths.resolv)
            .unwrap()
            .starts_with(MARKER));
        let mode = std::fs::metadata(&paths.record)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        // The second start does not record the marker file as the original.
        let dns = set_file(&paths, &ours()).unwrap();
        let record: Record =
            serde_json::from_str(&std::fs::read_to_string(&paths.record).unwrap()).unwrap();
        assert_eq!(record.text.as_deref(), Some(ORIGINAL));
        drop(dns);
        reconcile(&paths).unwrap();
        assert_eq!(std::fs::read_to_string(&paths.resolv).unwrap(), ORIGINAL);
        assert!(!paths.record.exists());
    }

    #[test]
    fn resolv_backup_restores_a_symlink_and_an_absent_file() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::in_dir(dir.path().join("resolv.conf"), dir.path());
        std::os::unix::fs::symlink("stub.conf", &paths.resolv).unwrap();
        drop(set_file(&paths, &ours()).unwrap());
        reconcile(&paths).unwrap();
        assert_eq!(
            std::fs::read_link(&paths.resolv).unwrap(),
            PathBuf::from("stub.conf")
        );
        std::fs::remove_file(&paths.resolv).unwrap();
        drop(set_file(&paths, &ours()).unwrap());
        reconcile(&paths).unwrap();
        assert!(std::fs::symlink_metadata(&paths.resolv).is_err());
    }

    #[test]
    fn resolv_marker_guards_the_restore() {
        let dir = tempfile::tempdir().unwrap();
        let paths = setup(dir.path());
        drop(set_file(&paths, &ours()).unwrap());
        std::fs::write(&paths.resolv, "nameserver 9.9.9.9\n").unwrap();
        reconcile(&paths).unwrap();
        assert_eq!(
            std::fs::read_to_string(&paths.resolv).unwrap(),
            "nameserver 9.9.9.9\n"
        );
        assert!(!paths.record.exists());
    }

    #[test]
    fn resolv_record_of_another_version_is_not_applied() {
        let dir = tempfile::tempdir().unwrap();
        let paths = setup(dir.path());
        drop(set_file(&paths, &ours()).unwrap());
        let text = std::fs::read_to_string(&paths.record).unwrap();
        std::fs::write(
            &paths.record,
            text.replace("\"version\":1", "\"version\":2"),
        )
        .unwrap();
        let err = reconcile(&paths).unwrap_err().to_string();
        assert!(err.contains("version 2"), "{err}");
        assert!(paths.record.exists());
        assert!(std::fs::read_to_string(&paths.resolv)
            .unwrap()
            .starts_with(MARKER));
    }

    #[test]
    fn a_live_tun_blocks_a_second_one() {
        let dir = tempfile::tempdir().unwrap();
        let paths = setup(dir.path());
        let _live = set_file(&paths, &ours()).unwrap();
        let err = set_file(&paths, &ours()).unwrap_err().to_string();
        assert!(err.contains("another warp tun runs"), "{err}");
        reconcile(&paths).unwrap();
        assert!(std::fs::read_to_string(&paths.resolv)
            .unwrap()
            .starts_with(MARKER));
    }
}
