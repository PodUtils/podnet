//! The pair file: `NAME.json` in the `relay` directory of the state
//! directory. It holds the three tokens of a pair, so it is a private file:
//! mode 0600, a regular file of this user, no symlink.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};

use super::{check_name, Pair};
use crate::exit::{fault, Fault};
use crate::relay_pool::{read_private_checked, Unusable};

/// Write `pair` to `dir/NAME.json` with mode 0600 (the directory 0700). The
/// data goes to a new file and a rename puts it in place, so a file of an
/// older mode or a symlink at the path is replaced. A directory that other
/// users can write is refused.
pub fn save(pair: &Pair, dir: &Path) -> Result<PathBuf> {
    use std::os::unix::fs::DirBuilderExt as _;
    check_name(&pair.name)?;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .with_context(|| format!("creating {}", dir.display()))?;
    crate::state::check_private_dir(dir).with_context(|| format!("{}", dir.display()))?;
    let path = dir.join(format!("{}.json", pair.name));
    crate::state::replace_file(&path, serde_json::to_string_pretty(pair)?.as_bytes(), 0o600)?;
    Ok(path)
}

/// Read the pair file `path`: the checks of a private file. A file that is
/// missing is 69, a file that is not safe, cannot be read or does not parse
/// is 78.
fn read(path: &Path) -> Result<Pair> {
    let text = match read_private_checked(path) {
        Ok(text) => text,
        Err(Unusable::Missing) => {
            return Err(fault(
                Fault::Unavailable,
                anyhow::anyhow!("no pair at {} (podnet relay pair)", path.display()),
            ))
        }
        Err(Unusable::Unsafe(why)) => {
            return Err(fault(
                Fault::Config,
                anyhow::anyhow!(
                    "{} is not used: {why} (it holds tokens: chmod 600 it, or make a new pair)",
                    path.display()
                ),
            ))
        }
        Err(Unusable::Io(err)) => {
            return Err(fault(
                Fault::Config,
                anyhow::anyhow!("reading {}: {err}", path.display()),
            ))
        }
    };
    serde_json::from_str(&text)
        .with_context(|| format!("{} is not a pair", path.display()))
        .map_err(|e| fault(Fault::Config, e))
}

/// Read the pair `name` from `dir`; with no name, the pair that has the
/// most time left.
/// The class of each error (issue 249): a bad name is 64, no pair is 69, a
/// pair file that cannot be read, is not safe or does not parse is 78.
pub fn load(dir: &Path, name: Option<&str>) -> Result<Pair> {
    if dir.is_dir() {
        crate::state::check_private_dir(dir)
            .with_context(|| format!("{}", dir.display()))
            .map_err(|e| fault(Fault::Config, e))?;
    }
    if let Some(name) = name {
        check_name(name).map_err(|e| fault(Fault::Usage, e))?;
        return read(&dir.join(format!("{name}.json")));
    }
    let mut best: Option<Pair> = None;
    for entry in std::fs::read_dir(dir)
        .with_context(|| format!("no pair in {} (podnet relay pair)", dir.display()))
        .map_err(|e| fault(Fault::Unavailable, e))?
        .flatten()
    {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        match read(&path) {
            Ok(pair) => {
                if best.as_ref().is_none_or(|b| pair.expires > b.expires) {
                    best = Some(pair);
                }
            }
            Err(err) => tracing::warn!("{err:#}"),
        }
    }
    best.ok_or_else(|| {
        fault(
            Fault::Unavailable,
            anyhow::anyhow!("no pair in {} (podnet relay pair)", dir.display()),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt as _;

    fn pair(name: &str) -> Pair {
        Pair {
            name: name.into(),
            node_token: "n".into(),
            connect_token: "c".into(),
            stop_token: "s".into(),
            expires: 5,
            relay: "https://relay.example".into(),
        }
    }

    fn mode(path: &Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn a_file_of_mode_0644_becomes_0600_after_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.json");
        std::fs::write(&path, "old").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        save(&pair("a"), dir.path()).unwrap();
        assert_eq!(mode(&path), 0o600);
        assert_eq!(load(dir.path(), Some("a")).unwrap().name, "a");
    }

    #[test]
    fn save_replaces_a_symlink_and_does_not_write_through_it() {
        let dir = tempfile::tempdir().unwrap();
        let other = dir.path().join("other");
        std::fs::write(&other, "keep").unwrap();
        let link = dir.path().join("a.json");
        std::os::unix::fs::symlink(&other, &link).unwrap();
        save(&pair("a"), dir.path()).unwrap();
        assert_eq!(std::fs::read_to_string(&other).unwrap(), "keep");
        assert!(std::fs::symlink_metadata(&link).unwrap().is_file());
    }

    #[test]
    fn load_refuses_a_file_that_is_open_a_symlink_or_large() {
        let dir = tempfile::tempdir().unwrap();
        let path = save(&pair("a"), dir.path()).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let err = load(dir.path(), Some("a")).unwrap_err();
        assert_eq!(crate::exit::code_of(&err), 78, "{err:#}");
        assert!(format!("{err:#}").contains("other users"), "{err:#}");
        // A symlink to a good file.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::os::unix::fs::symlink(&path, dir.path().join("b.json")).unwrap();
        let err = load(dir.path(), Some("b")).unwrap_err();
        assert_eq!(crate::exit::code_of(&err), 78, "{err:#}");
        assert!(format!("{err:#}").contains("not a regular file"), "{err:#}");
        // A file over the limit.
        let big = dir.path().join("c.json");
        std::fs::write(&big, vec![b' '; 70 * 1024]).unwrap();
        std::fs::set_permissions(&big, std::fs::Permissions::from_mode(0o600)).unwrap();
        let err = load(dir.path(), Some("c")).unwrap_err();
        assert_eq!(crate::exit::code_of(&err), 78, "{err:#}");
        assert!(format!("{err:#}").contains("too large"), "{err:#}");
        // No name: the bad files do not hide the good one.
        assert_eq!(load(dir.path(), None).unwrap().name, "a");
    }

    #[test]
    fn a_directory_that_others_can_write_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let relay = dir.path().join("relay");
        save(&pair("a"), &relay).unwrap();
        std::fs::set_permissions(&relay, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(save(&pair("b"), &relay).is_err());
        let err = load(&relay, Some("a")).unwrap_err();
        assert_eq!(crate::exit::code_of(&err), 78, "{err:#}");
    }
}
