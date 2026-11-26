//! `podnet offline spool`: a directory of entries, not a database.

use std::io::{Read, Write};
use std::path::PathBuf;

use anyhow::{anyhow, Result};
use clap::{Args, Subcommand};

use podnet::exit::{self, Fault};
use podnet::offline::spool::Spool;

use super::{hex, key, next_sequence, store_error};

#[derive(Args, Debug)]
pub(crate) struct SpoolArgs {
    #[command(subcommand)]
    pub(crate) command: SpoolCommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum SpoolCommand {
    /// Write one entry: a body, a sequence and the sender of this install.
    Add(AddArgs),
    /// List each entry that verifies, in the order of its sequence.
    List(ListArgs),
    /// Take the entry with the lowest sequence: write its body, then
    /// remove the file. A process that dies before the write keeps it.
    Take(TakeArgs),
    /// Check each file of the spool, and name each fault at its offset.
    Check(CheckArgs),
}

#[derive(Args, Debug)]
pub(crate) struct AddArgs {
    /// The directory of the spool. It is made when it does not exist.
    pub(crate) path: PathBuf,
    /// The key of the sender. Default: the state directory of podnet.
    #[arg(long, value_name = "DIR")]
    pub(crate) key: Option<PathBuf>,
    /// The sequence of the entry. Default: one above the highest entry.
    #[arg(long, value_name = "N")]
    pub(crate) sequence: Option<u64>,
    /// The body from this file. Default: stdin.
    #[arg(long, value_name = "FILE")]
    pub(crate) file: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub(crate) struct ListArgs {
    pub(crate) path: PathBuf,
    /// One JSON document: {"entries": [...]}.
    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Args, Debug)]
pub(crate) struct TakeArgs {
    pub(crate) path: PathBuf,
    /// Write the body to this new file (a file that exists exits 64, and the
    /// entry stays). Default: stdout.
    #[arg(long, value_name = "FILE")]
    pub(crate) out: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub(crate) struct CheckArgs {
    pub(crate) path: PathBuf,
}

pub(crate) fn spool(args: SpoolArgs) -> Result<()> {
    match args.command {
        SpoolCommand::Add(args) => add(args),
        SpoolCommand::List(args) => list(args),
        SpoolCommand::Take(args) => take(args),
        SpoolCommand::Check(args) => check(args),
    }
}

fn add(args: AddArgs) -> Result<()> {
    let spool = Spool::open(&args.path).map_err(store_error)?;
    let sequence = match args.sequence {
        Some(sequence) => sequence,
        None => next_sequence(&spool)?,
    };
    let body = match &args.file {
        Some(path) => std::fs::read(path)
            .map_err(|e| exit::fault(Fault::NoInput, anyhow!("{}: {e}", path.display())))?,
        None => {
            let mut body = Vec::new();
            std::io::stdin()
                .read_to_end(&mut body)
                .map_err(|e| anyhow!("stdin: {e}"))?;
            body
        }
    };
    let me = key(&args.key, true)?;
    let entry = spool.put(me.key(), sequence, &body).map_err(store_error)?;
    println!(
        "{} {} {} {}",
        entry.sequence,
        hex(&entry.digest),
        entry.body.len(),
        entry.path().display()
    );
    Ok(())
}

fn list(args: ListArgs) -> Result<()> {
    let spool = Spool::open(&args.path).map_err(store_error)?;
    let entries = spool.entries().map_err(store_error)?;
    if args.json {
        let rows: Vec<serde_json::Value> = entries
            .iter()
            .map(|entry| {
                serde_json::json!({
                    "sequence": entry.sequence,
                    "sender": entry.sender.hex(),
                    "length": entry.body.len(),
                    "checksum": hex(&entry.digest),
                    "file": entry.path().display().to_string(),
                })
            })
            .collect();
        println!(
            "{}",
            podnet::status::stamped(serde_json::json!({ "entries": rows }))
        );
    } else {
        for entry in &entries {
            println!(
                "{} {} {} {} {}",
                entry.sequence,
                entry.sender.hex(),
                entry.body.len(),
                hex(&entry.digest),
                entry.path().display()
            );
        }
    }
    Ok(())
}

fn take(args: TakeArgs) -> Result<()> {
    let spool = Spool::open(&args.path).map_err(store_error)?;
    let entry = spool.receive().map_err(store_error)?;
    let entry = entry.ok_or_else(|| exit::fault(Fault::NoInput, anyhow!("the spool is empty")))?;
    match &args.out {
        Some(path) => {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
                .map_err(|e| match e.kind() {
                    // T-613: take writes a new file; the entry stays in the
                    // spool, and nothing is done.
                    std::io::ErrorKind::AlreadyExists => exit::fault(
                        Fault::Usage,
                        anyhow!(
                            "--out {}: the file exists; give a new path (take does not write over a file)",
                            path.display()
                        ),
                    ),
                    _ => exit::fault(Fault::OsError, anyhow!("{}: {e}", path.display())),
                })?;
            file.write_all(&entry.body)
                .and_then(|()| file.sync_all())
                .map_err(|e| exit::fault(Fault::OsError, anyhow!("{}: {e}", path.display())))?;
        }
        None => {
            let mut out = std::io::stdout();
            out.write_all(&entry.body)
                .and_then(|()| out.flush())
                .map_err(|e| anyhow!("stdout: {e}"))?;
        }
    }
    // The entry leaves the spool only after its body is on its way.
    spool.consume(&entry).map_err(store_error)?;
    eprintln!(
        "took sequence {} of {} ({} bytes)",
        entry.sequence,
        entry.sender.hex(),
        entry.body.len()
    );
    Ok(())
}

fn check(args: CheckArgs) -> Result<()> {
    let spool = Spool::open(&args.path).map_err(store_error)?;
    let faults = spool.check().map_err(store_error)?;
    for fault in &faults {
        println!("{} {} {}", fault.file.display(), fault.offset, fault.why);
    }
    if faults.is_empty() {
        println!("ok: no fault");
        return Ok(());
    }
    let verb = if faults.len() == 1 {
        "entry does"
    } else {
        "entries do"
    };
    Err(exit::fault(
        Fault::DataError,
        anyhow!("{} {} not verify", faults.len(), verb),
    ))
}
