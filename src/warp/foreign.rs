//! An identity file of another user, read with the consent of
//! `PODNET_FOREIGN_IDENTITY=allow` (T-173). `pod-warp` checks the owner and
//! the mode; this module gives it the consent and names the variable.

use std::path::Path;

use super::WarpClient;

/// The consent variable of an identity file of another user.
pub const FOREIGN_IDENTITY_VARIABLE: &str = "PODNET_FOREIGN_IDENTITY";

fn consent() -> bool {
    std::env::var(FOREIGN_IDENTITY_VARIABLE).ok().as_deref() == Some("allow")
}

/// The identity file `path` for `pod-warp`. `PODNET_FOREIGN_IDENTITY=allow`
/// lets a file of another user be read, for a path that the user named:
/// never for the default path of the state directory.
pub(super) fn identity_file(path: &Path) -> pod_warp::IdentityFile {
    identity_file_with(path, consent())
}

fn identity_file_with(path: &Path, allow: bool) -> pod_warp::IdentityFile {
    let mut file = pod_warp::IdentityFile::new(path);
    let default = WarpClient::default_identity_path().ok().flatten();
    if allow && default.as_deref() != Some(path) {
        file.foreign = pod_warp::Foreign::Allow;
    }
    file
}

/// The owner of `path` when it is not this user.
#[cfg(unix)]
pub(super) fn owner(path: &Path) -> Option<u32> {
    use std::os::unix::fs::MetadataExt;
    let uid = std::fs::metadata(path).ok()?.uid();
    (uid != pod_warp::files::this_uid()).then_some(uid)
}

#[cfg(not(unix))]
pub(super) fn owner(_path: &Path) -> Option<u32> {
    None
}

/// A refused identity file of another user names the consent variable.
pub(super) fn hint(err: anyhow::Error, path: &Path) -> anyhow::Error {
    if owner(path).is_none() || consent() || crate::exit::code_of(&err) != crate::exit::CONFIG {
        return err;
    }
    let text = format!(
        "to use the identity of that user, name the file with --identity (--warp-identity) and set {FOREIGN_IDENTITY_VARIABLE}=allow; podnet then reads the file and writes nothing to it"
    );
    crate::exit::fault(crate::exit::Fault::Config, err.context(text))
}

/// The warning for an identity file of another user that podnet reads.
pub(super) fn warn(path: &Path) {
    if let Some(uid) = owner(path) {
        tracing::warn!(
            "{FOREIGN_IDENTITY_VARIABLE}=allow: {} belongs to user {uid}; podnet reads it and writes nothing to it",
            path.display()
        );
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use pod_warp::Foreign;

    #[test]
    fn the_consent_is_for_a_named_path_only() {
        let named = Path::new("/srv/other/warp.json");
        assert_eq!(identity_file_with(named, false).foreign, Foreign::Refuse);
        assert_eq!(identity_file_with(named, true).foreign, Foreign::Allow);
    }

    /// A saved identity file, owned by another user (as root only).
    fn foreign_file(dir: &Path) -> Option<std::path::PathBuf> {
        let device: super::super::Device = serde_json::from_value(serde_json::json!({
            "id": "dev", "tunnel_type": "masque", "account": {"id": "acc"},
            "config": {"client_id": "AQID"},
        }))
        .unwrap();
        let identity = super::super::Identity::generate().unwrap();
        let path = dir.join("warp.json");
        WarpClient::from_parts(device, identity)
            .save(&path)
            .unwrap();
        std::os::unix::fs::chown(&path, Some(65534), None).ok()?;
        Some(path)
    }

    // T-173: a file of another user is refused with exit 78 and the name
    // of the variable, read with the variable, and never written.
    #[tokio::test]
    async fn a_foreign_identity_needs_the_consent_variable() {
        if pod_warp::files::this_uid() != 0 {
            eprintln!("skipped: a file of another user needs root to make");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let Some(path) = foreign_file(dir.path()) else {
            eprintln!("skipped: chown is refused");
            return;
        };
        let before = std::fs::read(&path).unwrap();
        std::env::remove_var(FOREIGN_IDENTITY_VARIABLE);
        let err = WarpClient::register_or_load(&path)
            .await
            .err()
            .expect("refused");
        assert_eq!(crate::exit::code_of(&err), crate::exit::CONFIG, "{err:#}");
        assert!(
            format!("{err:#}").contains(FOREIGN_IDENTITY_VARIABLE),
            "{err:#}"
        );
        let err = WarpClient::load(&path).err().expect("refused");
        assert_eq!(crate::exit::code_of(&err), crate::exit::CONFIG, "{err:#}");
        // The refusal of `register_or_load` leaves the lock file of
        // `pod-warp`: it is an ask for podcrates (the check before the lock).
        let _ = std::fs::remove_file(dir.path().join("warp.json.lock"));
        std::env::set_var(FOREIGN_IDENTITY_VARIABLE, "allow");
        let client = WarpClient::register_or_load(&path).await;
        let loaded = WarpClient::load(&path);
        std::env::remove_var(FOREIGN_IDENTITY_VARIABLE);
        assert_eq!(client.expect("read with consent").device().id, "dev");
        assert_eq!(loaded.expect("load with consent").device().id, "dev");
        assert_eq!(std::fs::read(&path).unwrap(), before, "nothing is written");
        let others: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(others.len(), 1, "no lock, door or leg file: {others:?}");
    }

    // The mode stays checked with the consent.
    #[tokio::test]
    async fn the_mode_is_checked_with_the_consent() {
        use std::os::unix::fs::PermissionsExt;
        if pod_warp::files::this_uid() != 0 {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let Some(path) = foreign_file(dir.path()) else {
            return;
        };
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let file = identity_file_with(&path, true);
        let err = pod_warp::store::load(&file).expect_err("loose mode");
        assert!(err.chain().contains("chmod 600"), "{}", err.chain());
    }
}
