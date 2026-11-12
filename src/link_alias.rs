//! Names for live links (T-308, GitHub #476 row E10-3).
//!
//! `--name NAME` of `link listen` and `link connect` is the name of the
//! background record. For a link, the name has 1 to 32 characters from
//! `a-z 0-9 -`, and it is unique among the live links. At most 64 link
//! instances are live. A name never replaces the fingerprint: the first dial
//! needs `--peer` or `--allow`. A dialer binds its name to the fingerprint of
//! the peer that it proved. The file `NAME.peer` next to the record holds it,
//! and it lives as long as the record. A dial under the same name to another
//! peer exits 77.

use std::path::Path;

use anyhow::{anyhow, Result};

use crate::exit::{self, Fault};
use crate::instances;

/// The most live link instances.
pub const MAX_NAMES: usize = 64;

/// The most characters of a name.
pub const NAME_LIMIT: usize = 32;

/// The word that starts the command of a link instance.
const COMMAND: &str = "link ";

fn usage(text: String) -> anyhow::Error {
    exit::fault(Fault::Usage, anyhow!(text))
}

/// Check the characters and the length of a name. A bad name exits 64.
pub fn check(name: &str) -> Result<()> {
    let good = !name.is_empty()
        && name.len() <= NAME_LIMIT
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if good {
        return Ok(());
    }
    Err(usage(format!(
        "--name {name:?}: a link name has 1 to {NAME_LIMIT} characters from a-z 0-9 -"
    )))
}

/// Check a name before a link command starts. A bad name, a name that a live
/// instance holds (unless `replace`) and a 65th live link exit 64.
pub fn precheck(dir: &Path, name: &str, replace: bool) -> Result<()> {
    check(name)?;
    if !replace && instances::is_alive(dir, name) {
        return Err(usage(format!(
            "--name {name}: a live instance has this name (podnet ps); use another name"
        )));
    }
    let live = instances::list(dir)
        .iter()
        .filter(|e| e.record.command.starts_with(COMMAND) && e.record.name != name)
        .count();
    if live >= MAX_NAMES {
        return Err(usage(format!(
            "--name {name}: {MAX_NAMES} link instances are live: the limit"
        )));
    }
    Ok(())
}

/// Bind the instance `name` to the fingerprint `peer`. The first call stores
/// it. A later call with the same fingerprint is a reconnect and passes. A
/// call with another fingerprint exits 77 and changes nothing.
pub fn bind(dir: &Path, name: &str, peer: &str) -> Result<()> {
    let path = instances::peer_path(dir, name);
    if let Ok(stored) = std::fs::read_to_string(&path) {
        let stored = stored.trim();
        if stored == peer {
            return Ok(());
        }
        instances::KEEP_PEER.store(true, std::sync::atomic::Ordering::SeqCst);
        return Err(exit::fault(
            Fault::NoPermission,
            anyhow!("the name {name} is bound to the peer {stored}, and the peer proved {peer}: the key changed"),
        ));
    }
    write_private(&path, peer)
        .map_err(|e| exit::fault(Fault::OsError, anyhow!("{}: {e}", path.display())))
}

fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    writeln!(file, "{text}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_has_1_to_32_characters_of_a_z_0_9_dash() {
        for good in ["a", "web-1", &"a".repeat(32)] {
            assert!(check(good).is_ok(), "{good}");
        }
        for bad in ["", "Web", "a_b", "a.b", "a b", &"a".repeat(33)] {
            let err = check(bad).unwrap_err();
            assert_eq!(exit::code_of(&err), exit::USAGE, "{bad}");
        }
    }

    #[test]
    fn the_65th_live_link_exits_64() {
        let dir = tempfile::tempdir().unwrap();
        let mut held = Vec::new();
        for n in 0..MAX_NAMES {
            let name = format!("l-{n}");
            let record = instances::Record {
                name: name.clone(),
                command: "link listen".into(),
                state: "ready".into(),
                ..instances::Record::default()
            };
            instances::write(dir.path(), &record).unwrap();
            let lock = instances::paths(dir.path(), &name).unwrap().lock;
            held.push(instances::try_lock(&lock).unwrap().unwrap());
        }
        let err = precheck(dir.path(), "one-more", false).unwrap_err();
        assert_eq!(exit::code_of(&err), exit::USAGE);
        // A name that a live instance holds exits 64 too, and a free name
        // passes below the limit.
        let err = precheck(dir.path(), "l-3", false).unwrap_err();
        assert_eq!(exit::code_of(&err), exit::USAGE);
        held.pop();
        precheck(dir.path(), "one-more", false).unwrap();
    }

    #[test]
    fn a_binding_keeps_its_fingerprint() {
        let dir = tempfile::tempdir().unwrap();
        bind(dir.path(), "web", "SHA256:aaa").unwrap();
        bind(dir.path(), "web", "SHA256:aaa").unwrap();
        let err = bind(dir.path(), "web", "SHA256:bbb").unwrap_err();
        instances::KEEP_PEER.store(false, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(exit::code_of(&err), 77);
    }
}
