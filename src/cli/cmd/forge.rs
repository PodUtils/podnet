//! `podnet forge`: a forge without one host (FORGE-1 and FORGE-2).
//!
//! `clone` reads a local content-addressed seed, clones it with no forge
//! account and no network, and verifies the commit hash of the checkout. A
//! seed whose content is not the key that `--expect` names is refused by
//! name.

use std::path::PathBuf;

use anyhow::Result;
use clap::{Args, Subcommand};

/// `podnet forge`.
#[derive(Args, Debug)]
pub(crate) struct ForgeArgs {
    #[command(subcommand)]
    pub(crate) command: ForgeCommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum ForgeCommand {
    /// Clone a local seed and verify that the checkout is HASH. It reads no
    /// forge account and allows the `file` protocol only.
    Clone {
        /// The local git repository that is the seed.
        #[arg(long, value_name = "PATH")]
        seed: PathBuf,
        /// The commit hash that the checkout must have (the 40 hex digits,
        /// or a 64-digit hash of a SHA-256 repository).
        #[arg(long, value_name = "HASH")]
        expect: String,
        /// The directory to make; default: the seed's name in this
        /// directory.
        #[arg(value_name = "DEST")]
        dest: Option<PathBuf>,
    },
}

pub(crate) fn forge(args: ForgeArgs) -> Result<()> {
    match args.command {
        ForgeCommand::Clone { seed, expect, dest } => clone(seed, expect, dest),
    }
}

fn clone(seed: PathBuf, expect: String, dest: Option<PathBuf>) -> Result<()> {
    let dest = dest.unwrap_or_else(|| default_dest(&seed));
    let cloned = podnet::forge::clone_seed(&seed, &expect, &dest)?;
    println!(
        "cloned {} to {} at {}",
        seed.display(),
        cloned.path.display(),
        cloned.hash
    );
    Ok(())
}

/// The seed's last path component, without a `.git` suffix.
fn default_dest(seed: &std::path::Path) -> PathBuf {
    let name = seed
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty() && name != "." && name != "..")
        .unwrap_or_else(|| "seed".to_string());
    let name = name.strip_suffix(".git").unwrap_or(&name);
    PathBuf::from(name)
}
