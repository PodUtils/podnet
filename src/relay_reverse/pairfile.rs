//! The pair file: `pair-NAME.json` of the store of `pod-relay` in the
//! `relay` directory of the state directory. It holds the three tokens of a
//! pair, so it is a private file: mode 0600, a regular file of this user,
//! no symlink, in a directory that other users cannot write.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use pod_relay::cache::Cache;

use super::Pair;
use crate::exit::{fault, Fault};
use crate::ws_relay::Relay;

/// The store of the pairs in `dir`.
fn store(dir: &Path) -> Cache {
    Cache::in_dirs(vec![dir.to_path_buf()])
}

/// A refusal of the directory of the pairs that other users can write.
fn check_dir(dir: &Path) -> Result<()> {
    if dir.is_dir() {
        crate::state::check_private_dir(dir)
            .with_context(|| format!("{}", dir.display()))
            .map_err(|e| fault(Fault::Config, e))?;
    }
    Ok(())
}

/// The pair file of the pair `name` in `dir`.
pub fn file_of(dir: &Path, name: &str) -> Result<PathBuf> {
    let file = pod_relay::pair::file_name(name)
        .map_err(|e| fault(Fault::Usage, anyhow::anyhow!("{e}")))?;
    Ok(dir.join(file))
}

/// Write `pair` to `dir/pair-NAME.json` with mode 0600 (the directory
/// 0700). The data goes to a new file and a rename puts it in place. A
/// directory that other users can write is refused.
pub fn save(pair: &Pair, dir: &Path) -> Result<PathBuf> {
    check_dir(dir)?;
    pod_relay::pair::store(&store(dir), &pair.name, &pair.inner)
        .map_err(|e| fault(Fault::Config, anyhow::anyhow!("{e}")))
}

/// Remove the pair file of `name` from `dir`.
pub fn remove(dir: &Path, name: &str) -> Result<()> {
    pod_relay::pair::remove(&store(dir), name).map_err(|e| anyhow::anyhow!("{e}"))
}

/// Read the pair file `path`: the checks of a private file. A file that is
/// missing is 69, a file that is not safe, cannot be read or does not parse
/// is 78.
fn read(path: &Path) -> Result<Pair> {
    let shown = path.display();
    match std::fs::symlink_metadata(path) {
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Err(fault(
                Fault::Unavailable,
                anyhow::anyhow!("no pair at {shown} (podnet relay pair)"),
            ))
        }
        Err(err) => {
            return Err(fault(
                Fault::Config,
                anyhow::anyhow!("reading {shown}: {err}"),
            ))
        }
        Ok(meta) if !meta.file_type().is_file() => {
            return Err(fault(
                Fault::Config,
                anyhow::anyhow!("{shown} is not used: it is not a regular file"),
            ))
        }
        Ok(_) => {}
    }
    let unusable = |why: String| {
        fault(
            Fault::Config,
            anyhow::anyhow!("{why} (the file holds tokens: chmod 600 it, or make a new pair)"),
        )
    };
    let inner = pod_relay::pair::read_file(path).map_err(|e| unusable(e.to_string()))?;
    // The store keeps the relay with no scheme; a pair file of a test names
    // a plain relay as `http://` (pod-relay call C103).
    let text = pod_relay::cache::read_private(path).map_err(unusable)?;
    let doc: serde_json::Value =
        serde_json::from_str(&text).map_err(|_| unusable(format!("{shown} is not a pair")))?;
    let relay = doc
        .get("relay")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let url = Relay::single(relay, None)
        .map_err(|e| unusable(format!("the relay of {shown}: {e:#}")))?
        .url();
    Ok(Pair::of(inner, url))
}

/// Read the pair `name` from `dir`; with no name, the pair that has the
/// most time left.
/// The class of each error (issue 249): a bad name is 64, no pair is 69, a
/// pair file that cannot be read, is not safe or does not parse is 78.
pub fn load(dir: &Path, name: Option<&str>) -> Result<Pair> {
    check_dir(dir)?;
    let label = match name {
        Some(name) => name.to_string(),
        None => match pod_relay::pair::newest(&store(dir), None) {
            Some((label, _)) => label,
            None => {
                return Err(fault(
                    Fault::Unavailable,
                    anyhow::anyhow!("no pair in {} (podnet relay pair)", dir.display()),
                ))
            }
        },
    };
    read(&file_of(dir, &label)?)
}

/// The stored pair that a node takes at the expiry of `ending`: one that
/// expires later and has time left.
pub(super) fn next(dir: &Path, ending: &pod_relay::pair::Pair) -> Option<Pair> {
    let (label, _) = pod_relay::pair::newest(&store(dir), Some(ending))?;
    load(dir, Some(&label)).ok()
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt as _;

    /// Write the pair `name` to `dir` as the store writes it, and read it.
    pub(in crate::relay_reverse) fn written(dir: &Path, name: &str, expires: u64) -> Pair {
        write(dir, name, expires);
        load(dir, Some(name)).unwrap()
    }

    fn write(dir: &Path, name: &str, expires: u64) -> PathBuf {
        pod_relay::cache::private_dir(dir).unwrap();
        let doc = serde_json::json!({
            "relay": "relay.example", "name": name, "node_token": "testonly-n",
            "connect_token": "testonly-c", "stop_token": "testonly-s", "expires": expires,
        });
        let path = file_of(dir, name).unwrap();
        std::fs::write(&path, doc.to_string()).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        path
    }

    #[test]
    fn load_refuses_a_file_that_is_open_a_symlink_or_large() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "a", 5);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let err = load(dir.path(), Some("a")).unwrap_err();
        assert_eq!(crate::exit::code_of(&err), 78, "{err:#}");
        assert!(format!("{err:#}").contains("others can read it"), "{err:#}");
        // A symlink to a good file.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::os::unix::fs::symlink(&path, dir.path().join("pair-b.json")).unwrap();
        let err = load(dir.path(), Some("b")).unwrap_err();
        assert_eq!(crate::exit::code_of(&err), 78, "{err:#}");
        assert!(format!("{err:#}").contains("not a regular file"), "{err:#}");
        // A file over the limit.
        let big = dir.path().join("pair-c.json");
        std::fs::write(&big, vec![b' '; 70 * 1024]).unwrap();
        std::fs::set_permissions(&big, std::fs::Permissions::from_mode(0o600)).unwrap();
        let err = load(dir.path(), Some("c")).unwrap_err();
        assert_eq!(crate::exit::code_of(&err), 78, "{err:#}");
        assert!(format!("{err:#}").contains("larger"), "{err:#}");
        // No name: the bad files do not hide the good one.
        assert_eq!(load(dir.path(), None).unwrap().name, "a");
        // No pair of the name, and a name that is no name.
        let err = load(dir.path(), Some("d")).unwrap_err();
        assert_eq!(crate::exit::code_of(&err), 69, "{err:#}");
        let err = load(dir.path(), Some("../x")).unwrap_err();
        assert_eq!(crate::exit::code_of(&err), 64, "{err:#}");
    }

    #[test]
    fn a_directory_that_others_can_write_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let relay = dir.path().join("relay");
        write(&relay, "a", 5);
        std::fs::set_permissions(&relay, std::fs::Permissions::from_mode(0o777)).unwrap();
        let err = load(&relay, Some("a")).unwrap_err();
        assert_eq!(crate::exit::code_of(&err), 78, "{err:#}");
        let err = load(&relay, None).unwrap_err();
        assert_eq!(crate::exit::code_of(&err), 78, "{err:#}");
    }

    #[test]
    fn the_newest_pair_with_no_name_and_its_relay_url() {
        let dir = tempfile::tempdir().unwrap();
        written(dir.path(), "old", 1);
        let new = written(dir.path(), "new", 2);
        assert_eq!(
            (new.relay.as_str(), new.node_token()),
            ("https://relay.example", "testonly-n")
        );
        assert_eq!(load(dir.path(), None).unwrap().name, "new");
        assert!(!format!("{new:?}").contains("testonly"));
    }
}
