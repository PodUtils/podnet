//! The facts of the build: the version of the workspace and the commit.
//! `build.rs` of this crate reads the commit from `PODNET_BUILD_COMMIT`
//! (the release workflow sets it; a local build gives "unknown").

/// The version of the workspace (`[workspace.package] version`).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The commit of the build, or "unknown".
pub const COMMIT: &str = env!("PODNET_BUILD_COMMIT");

/// The version line of podnet: `VERSION (COMMIT)`.
pub fn version() -> &'static str {
    concat!(
        env!("CARGO_PKG_VERSION"),
        " (",
        env!("PODNET_BUILD_COMMIT"),
        ")"
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_version_line_has_the_version_and_the_commit() {
        assert_eq!(
            super::version(),
            format!("{} ({})", super::VERSION, super::COMMIT)
        );
        assert!(!super::COMMIT.is_empty());
    }
}
