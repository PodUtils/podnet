//! The fish file of `podnet warp up --persist` (issue 115, T-211): a file of
//! its own in `~/.config/fish/conf.d`. podnet changes or removes only a file
//! that it wrote: its first line is the marker of the shell blocks.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context as _, Result};

use super::{quote, BEGIN, END, SHELL_VARIABLE};

/// The file of fish: a file of its own in conf.d, when fish is the shell or
/// its config directory exists.
pub(super) fn fish_file() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    crate::state::check_own(&home).ok()?;
    let dir = home.join(".config/fish");
    let shell = std::env::var("SHELL").unwrap_or_default();
    (shell.ends_with("fish") || dir.exists()).then(|| dir.join("conf.d/podnet-warp.fish"))
}

/// The fish form of the shell block. fish has `export`, so the env file works
/// with `source`.
pub(super) fn fish_block(env: &Path, socket: &Path, exe: &Path) -> String {
    let env = quote(&env.display().to_string());
    let socket = quote(&socket.display().to_string());
    let exe = quote(&exe.display().to_string());
    format!(
        "{BEGIN}\n\
if test -f {env}; source {env}; end\n\
if status is-interactive; and not set -q {SHELL_VARIABLE}; and test -S {socket}; and test -x {exe}; exec {exe} warp shell; end\n\
{END}\n"
    )
}

/// True when `text` is a fish file of podnet: it starts with the marker.
pub(super) fn fish_file_is_ours(text: &str) -> bool {
    text.lines().next() == Some(BEGIN)
}

/// An error when `file` exists and podnet did not write it.
pub(super) fn check(file: &Path) -> Result<()> {
    match std::fs::read_to_string(file) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Ok(text) if fish_file_is_ours(&text) => Ok(()),
        _ => bail!(
            "{} exists and podnet did not write it; remove it, or do not give --persist",
            file.display()
        ),
    }
}

/// Write `block` to `file`. A file that podnet did not write is an error,
/// and stays.
pub(super) fn write(file: &Path, block: &str) -> Result<()> {
    check(file)?;
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("making {}", dir.display()))?;
    }
    std::fs::write(file, block).with_context(|| format!("writing {}", file.display()))
}

/// Remove `file` when podnet wrote it. Print what happened.
pub(super) fn remove(file: &Path) {
    let Ok(text) = std::fs::read_to_string(file) else {
        if file.exists() {
            println!(
                "podnet: warp down: {} is not a file of podnet; it stays",
                file.display()
            );
        }
        return;
    };
    if !fish_file_is_ours(&text) {
        println!(
            "podnet: warp down: {} is not a file of podnet; it stays",
            file.display()
        );
    } else if std::fs::remove_file(file).is_ok() {
        println!("podnet: warp down: removed {}", file.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block() -> String {
        fish_block(
            Path::new("/r/warp.env"),
            Path::new("/r/warp.sock"),
            Path::new("/b/podnet"),
        )
    }

    #[test]
    fn the_fish_block_loads_the_env_file() {
        let fish = block();
        assert!(
            fish.contains("if test -f '/r/warp.env'; source '/r/warp.env'; end\n"),
            "{fish}"
        );
        assert!(fish.contains("exec '/b/podnet' warp shell; end"), "{fish}");
    }

    #[test]
    fn fish_file_is_ours_by_its_first_line() {
        assert!(fish_file_is_ours(&block()));
        assert!(!fish_file_is_ours(""));
        assert!(!fish_file_is_ours("set -x EDITOR vi\n"));
        assert!(!fish_file_is_ours(&format!("set -x A 1\n{}", block())));
    }

    #[test]
    fn a_user_fish_file_stays() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("conf.d/podnet-warp.fish");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "set -x EDITOR vi\n").unwrap();
        let err = write(&file, &block()).unwrap_err().to_string();
        assert!(err.contains(&file.display().to_string()), "{err}");
        remove(&file);
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "set -x EDITOR vi\n"
        );
        // A file of podnet is replaced and removed.
        std::fs::write(&file, block()).unwrap();
        write(&file, &block()).unwrap();
        remove(&file);
        assert!(!file.exists());
        // No file: written.
        write(&file, &block()).unwrap();
        assert!(fish_file_is_ours(&std::fs::read_to_string(&file).unwrap()));
    }
}
