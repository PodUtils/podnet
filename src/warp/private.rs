//! The check of a file of the WARP identity: its owner and its mode.

use anyhow::{Context as _, Result};

/// Refuse `path` when its owner is another user, or when its group or other
/// users may read or write it.
pub(crate) fn check_private(path: &std::path::Path) -> Result<()> {
    check_private_as(path, unsafe { libc::geteuid() })
}

/// [`check_private`] for the user `uid`. An owner that is another user is a
/// configuration error (T-173): it has the class `Config`, and no
/// registration follows it.
pub(super) fn check_private_as(path: &std::path::Path, uid: u32) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::metadata(path)
        .with_context(|| format!("reading the WARP identity from {}", path.display()))?;
    if meta.is_dir() {
        return Err(crate::exit::fault(
            crate::exit::Fault::Usage,
            anyhow::anyhow!("{} is a directory; give the identity file", path.display()),
        ));
    }
    if meta.uid() != uid {
        return Err(crate::exit::fault(
            crate::exit::Fault::Config,
            anyhow::anyhow!(
                "{} belongs to user {}, not to this user ({uid}); podnet does not use it. Run as that user, or give an identity file of this user (--identity PATH)",
                path.display(),
                meta.uid()
            ),
        ));
    }
    let mode = meta.mode() & 0o777;
    if mode & 0o077 != 0 {
        anyhow::bail!(
            "{} has mode {mode:04o}: other users may read the WARP key. Run: chmod 600 {}",
            path.display(),
            path.display()
        );
    }
    Ok(())
}
