//! `podnet offline capture`: one record format, merged by timestamp.

use std::path::PathBuf;

use anyhow::Result;
use clap::{Args, Subcommand};

use podnet::offline::capture;

use super::store_error;

#[derive(Args, Debug)]
pub(crate) struct CaptureArgs {
    #[command(subcommand)]
    pub(crate) command: CaptureCommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum CaptureCommand {
    /// Merge capture files into one, ordered by the timestamp, then by the
    /// place of the file in the list, then by the line. The output is
    /// redacted again with each secret.
    Merge(MergeArgs),
}

#[derive(Args, Debug)]
pub(crate) struct MergeArgs {
    /// The capture files, in the order that breaks an equal timestamp.
    #[arg(value_name = "FILE", required = true)]
    pub(crate) inputs: Vec<PathBuf>,
    /// The merged capture.
    #[arg(long, value_name = "FILE")]
    pub(crate) out: PathBuf,
    /// A secret to redact. Repeat it for more than one.
    #[arg(long, value_name = "SECRET")]
    pub(crate) secret: Vec<String>,
    /// A file with one secret on each line. Repeat it for more than one.
    #[arg(long, value_name = "FILE")]
    pub(crate) secret_file: Vec<PathBuf>,
}

pub(crate) fn capture(args: CaptureArgs) -> Result<()> {
    match args.command {
        CaptureCommand::Merge(args) => merge(args),
    }
}

fn merge(args: MergeArgs) -> Result<()> {
    let mut secrets: Vec<Vec<u8>> = args
        .secret
        .iter()
        .map(|secret| secret.as_bytes().to_vec())
        .collect();
    for path in &args.secret_file {
        let text = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
        for line in text.lines() {
            if !line.is_empty() {
                secrets.push(line.as_bytes().to_vec());
            }
        }
    }
    let records = capture::merge(&args.inputs, &args.out, secrets).map_err(store_error)?;
    let redacted = records.iter().filter(|record| record.redacted).count();
    println!(
        "merged {} record(s) into {}, {} marked redacted",
        records.len(),
        args.out.display(),
        redacted
    );
    Ok(())
}
