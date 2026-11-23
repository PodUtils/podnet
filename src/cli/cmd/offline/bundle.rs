//! `podnet offline bundle`: a signed manifest, and a delta from a base.

use std::path::PathBuf;

use anyhow::{anyhow, Result};
use clap::{Args, Subcommand};

use podnet::exit::{self, Fault};
use podnet::offline::bundle::{self, Op};

use super::{key, store_error};

#[derive(Args, Debug)]
pub(crate) struct BundleArgs {
    #[command(subcommand)]
    pub(crate) command: BundleCommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum BundleCommand {
    /// Write a bundle, or a delta from --base, into a directory.
    Build(BuildArgs),
    /// Apply a delta to the bundle that it names.
    Apply(ApplyArgs),
    /// Read a bundle and check the signature of its manifest and each file.
    Verify(VerifyArgs),
}

#[derive(Args, Debug)]
pub(crate) struct BuildArgs {
    /// The directory of the bundle.
    #[arg(long, value_name = "DIR")]
    pub(crate) out: PathBuf,
    /// The name of the bundle, one line.
    #[arg(long, value_name = "NAME")]
    pub(crate) name: String,
    /// A file of the bundle: SRC:DEST, with a relative DEST that has no
    /// `..`. Repeat it for each file.
    #[arg(long = "file", value_name = "SRC:DEST")]
    pub(crate) files: Vec<String>,
    /// The bundle of the previous release. With it, the output is a delta
    /// from that bundle, not a full bundle.
    #[arg(long, value_name = "DIR")]
    pub(crate) base: Option<PathBuf>,
    /// The key that signs the manifest. Default: the state directory.
    #[arg(long, value_name = "DIR")]
    pub(crate) key: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub(crate) struct ApplyArgs {
    /// The bundle that the delta names.
    #[arg(long, value_name = "DIR")]
    pub(crate) base: PathBuf,
    /// The directory of the delta.
    #[arg(long, value_name = "DIR")]
    pub(crate) delta: PathBuf,
}

#[derive(Args, Debug)]
pub(crate) struct VerifyArgs {
    /// The directory of the bundle.
    pub(crate) dir: PathBuf,
}

pub(crate) fn bundle(args: BundleArgs) -> Result<()> {
    match args.command {
        BundleCommand::Build(args) => build(args),
        BundleCommand::Apply(args) => apply(args),
        BundleCommand::Verify(args) => verify(args),
    }
}

/// The files of `--file SRC:DEST`, in the order of the flags.
fn files_of(flags: &[String]) -> Result<Vec<(String, Vec<u8>)>> {
    let mut out = Vec::new();
    for flag in flags {
        let (source, dest) = flag
            .split_once(':')
            .ok_or_else(|| exit::fault(Fault::Usage, anyhow!("--file {flag:?}: give SRC:DEST")))?;
        if dest.is_empty() {
            return Err(exit::fault(
                Fault::Usage,
                anyhow!("--file {flag:?}: the DEST is empty"),
            ));
        }
        let body = std::fs::read(source)
            .map_err(|e| exit::fault(Fault::NoInput, anyhow!("{source}: {e}")))?;
        out.push((dest.to_string(), body));
    }
    Ok(out)
}

fn build(args: BuildArgs) -> Result<()> {
    let me = key(&args.key, true)?;
    let files = files_of(&args.files)?;
    match &args.base {
        Some(base) => {
            let previous = bundle::read_bundle(base).map_err(store_error)?;
            let delta = bundle::write_delta(&args.out, me.key(), &previous, &args.name, &files)
                .map_err(store_error)?;
            let mut ops = [0usize; 3];
            for op in delta.ops.values() {
                match op {
                    Op::Add => ops[0] += 1,
                    Op::Change => ops[1] += 1,
                    Op::Delete => ops[2] += 1,
                }
            }
            println!(
                "delta {} {} base {} add {} change {} delete {}",
                delta.manifest.name, delta.manifest.id, delta.base, ops[0], ops[1], ops[2]
            );
        }
        None => {
            let manifest = bundle::write_bundle(&args.out, me.key(), &args.name, &files)
                .map_err(store_error)?;
            println!(
                "bundle {} {} files {}",
                manifest.name,
                manifest.id,
                manifest.files.len()
            );
        }
    }
    Ok(())
}

fn apply(args: ApplyArgs) -> Result<()> {
    let manifest = bundle::apply_delta(&args.base, &args.delta).map_err(store_error)?;
    println!(
        "applied {} {} files {}",
        manifest.name,
        manifest.id,
        manifest.files.len()
    );
    Ok(())
}

fn verify(args: VerifyArgs) -> Result<()> {
    let manifest = bundle::read_bundle(&args.dir).map_err(store_error)?;
    println!(
        "bundle {} {} files {} key {}",
        manifest.name,
        manifest.id,
        manifest.files.len(),
        manifest.key.hex()
    );
    Ok(())
}
