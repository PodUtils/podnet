//! `podnet offline replica`: an append-only log, merged by the tie rule.

use std::path::PathBuf;

use anyhow::{anyhow, Result};
use clap::{Args, Subcommand};

use podnet::exit::{self, Fault};
use podnet::offline::replication::{Record, Replica};

use super::{hex, key, store_error};

#[derive(Args, Debug)]
pub(crate) struct ReplicaArgs {
    #[command(subcommand)]
    pub(crate) command: ReplicaCommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum ReplicaCommand {
    /// Merge replication logs into one, and print the state of each key
    /// and each conflict that the rule resolved.
    Merge(MergeArgs),
}

#[derive(Args, Debug)]
pub(crate) struct MergeArgs {
    /// The logs, one record on each line.
    #[arg(value_name = "LOG", required = true)]
    pub(crate) logs: Vec<PathBuf>,
    /// Write the merged log here.
    #[arg(long, value_name = "FILE")]
    pub(crate) out: PathBuf,
    /// The key of this install. Default: the state directory.
    #[arg(long, value_name = "DIR")]
    pub(crate) key: Option<PathBuf>,
    /// One JSON document: {"records": N, "state": {...}, "conflicts": [...]}.
    #[arg(long)]
    pub(crate) json: bool,
}

pub(crate) fn replica(args: ReplicaArgs) -> Result<()> {
    match args.command {
        ReplicaCommand::Merge(args) => merge(args),
    }
}

fn records_of(path: &PathBuf) -> Result<Vec<Record>> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| exit::fault(Fault::NoInput, anyhow!("{}: {e}", path.display())))?;
    let mut out = Vec::new();
    for line in text.lines().filter(|line| !line.is_empty()) {
        out.push(Record::decode(line).map_err(store_error)?);
    }
    Ok(out)
}

fn merge(args: MergeArgs) -> Result<()> {
    let me = key(&args.key, true)?;
    let mut merged = Replica::new(me.key());
    for path in &args.logs {
        let records = records_of(path)?;
        merged.merge(&records);
    }
    // One order of the lines, whatever the order of the inputs: a merge is
    // the same file on each host.
    let mut lines: Vec<String> = merged.log().iter().map(Record::encode).collect();
    lines.sort();
    let mut text = String::new();
    for line in &lines {
        text.push_str(line);
        text.push('\n');
    }
    std::fs::write(&args.out, text)
        .map_err(|e| exit::fault(Fault::OsError, anyhow!("{}: {e}", args.out.display())))?;

    let state = merged.state();
    if args.json {
        let rows: Vec<serde_json::Value> = merged
            .conflicts()
            .iter()
            .map(|conflict| {
                serde_json::json!({
                    "key": conflict.key,
                    "winner": conflict.winner.hex(),
                    "winner_hash": conflict.winner_hash,
                    "losers": conflict.losers.iter().map(|key| key.hex()).collect::<Vec<_>>(),
                    "loser_hashes": conflict.loser_hashes,
                })
            })
            .collect();
        let values: serde_json::Map<String, serde_json::Value> = state
            .iter()
            .map(|(key, value)| (key.clone(), serde_json::Value::String(hex(value))))
            .collect();
        println!(
            "{}",
            podnet::status::stamped(serde_json::json!({
                "records": lines.len(),
                "state": values,
                "conflicts": rows,
            }))
        );
    } else {
        println!("records {}", lines.len());
        for (key, value) in &state {
            println!("{key} {}", hex(value));
        }
        for conflict in merged.conflicts() {
            println!(
                "conflict {} winner={} losers={} winner_key={}",
                conflict.key,
                conflict.winner_hash,
                conflict.loser_hashes.join(","),
                conflict.winner.hex()
            );
        }
    }
    Ok(())
}
