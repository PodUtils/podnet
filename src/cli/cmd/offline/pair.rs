//! `podnet offline pair`: an eight-word code, single use, bound to a key.

use std::io::{Read, Write};
use std::path::PathBuf;

use anyhow::{anyhow, Result};
use clap::{ArgGroup, Args, Subcommand};

use podnet::exit::{self, Fault};
use podnet::offline::pairing::{take, Code, Offer, UsedCodes, WORDS_COUNT};
use podnet::offline::spool::Spool;

use super::{key, next_sequence, store_error};

#[derive(Args, Debug)]
pub(crate) struct PairArgs {
    #[command(subcommand)]
    pub(crate) command: PairCommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum PairCommand {
    /// Make a code, sign the offer of it, and print the eight words.
    Offer(OfferArgs),
    /// Take an offer with a code, use the code once, and write the record
    /// that binds the peer key to this key.
    Accept(AcceptArgs),
}

#[derive(Args, Debug)]
pub(crate) struct OfferArgs {
    /// The key that signs the offer. Default: the state directory.
    #[arg(long, value_name = "DIR")]
    pub(crate) key: Option<PathBuf>,
    /// Write the offer here. Without --spool, stdout holds it.
    #[arg(long, value_name = "FILE")]
    pub(crate) out: Option<PathBuf>,
    /// Put the offer in this spool, for a peer that comes later.
    #[arg(long, value_name = "DIR")]
    pub(crate) spool: Option<PathBuf>,
    /// The sequence of the spool entry. Default: one above the highest.
    #[arg(long, value_name = "N")]
    pub(crate) sequence: Option<u64>,
}

#[derive(Args, Debug)]
#[command(group = ArgGroup::new("source").args(["offer", "spool"]).required(true))]
pub(crate) struct AcceptArgs {
    /// The key of this install. Default: the state directory.
    #[arg(long, value_name = "DIR")]
    pub(crate) key: Option<PathBuf>,
    /// The file of the offer.
    #[arg(long, value_name = "FILE")]
    pub(crate) offer: Option<PathBuf>,
    /// The spool that holds the offer.
    #[arg(long, value_name = "DIR")]
    pub(crate) spool: Option<PathBuf>,
    /// The directory of the used codes. Each code is one file in it.
    #[arg(long, value_name = "DIR")]
    pub(crate) used: PathBuf,
    /// Write the paired record here. Default: stdout.
    #[arg(long, value_name = "FILE")]
    pub(crate) out: Option<PathBuf>,
}

pub(crate) fn pair(args: PairArgs) -> Result<()> {
    match args.command {
        PairCommand::Offer(args) => offer(args),
        PairCommand::Accept(args) => accept(args),
    }
}

fn offer(args: OfferArgs) -> Result<()> {
    let me = key(&args.key, true)?;
    let mut entropy = [0u8; WORDS_COUNT];
    getrandom::fill(&mut entropy).map_err(|e| anyhow!("the entropy of the code: {e}"))?;
    let code = Code::from_entropy(&entropy);
    let offer = Offer::make(me.key(), &code);
    let encoded = offer.encode();
    if let Some(path) = &args.out {
        write_new(path, &encoded)?;
    }
    if let Some(dir) = &args.spool {
        let spool = Spool::open(dir).map_err(store_error)?;
        let sequence = match args.sequence {
            Some(sequence) => sequence,
            None => next_sequence(&spool)?,
        };
        spool
            .put(me.key(), sequence, &encoded)
            .map_err(store_error)?;
    }
    if args.out.is_none() && args.spool.is_none() {
        // The offer goes to stdout, and the code, which is the credential,
        // goes to stderr.
        let mut out = std::io::stdout();
        out.write_all(&encoded)
            .and_then(|()| out.flush())
            .map_err(|e| anyhow!("stdout: {e}"))?;
        eprintln!("{}", code.text());
    } else {
        println!("{}", code.text());
    }
    eprintln!("the offer is for {}", me.public().hex());
    Ok(())
}

fn accept(args: AcceptArgs) -> Result<()> {
    let me = key(&args.key, true)?;
    let mut text = String::new();
    std::io::stdin()
        .read_to_string(&mut text)
        .map_err(|e| anyhow!("stdin: {e}"))?;
    let code = Code::parse(text.trim()).map_err(store_error)?;
    let spool = match &args.spool {
        Some(dir) => Some(Spool::open(dir).map_err(store_error)?),
        None => None,
    };
    let (offer, entry) = match (&args.offer, &spool) {
        (Some(path), _) => {
            let bytes = std::fs::read(path)
                .map_err(|e| exit::fault(Fault::NoInput, anyhow!("{}: {e}", path.display())))?;
            (Offer::decode(&bytes).map_err(store_error)?, None)
        }
        (None, Some(spool)) => {
            let entry = spool
                .receive()
                .map_err(store_error)?
                .ok_or_else(|| exit::fault(Fault::NoInput, anyhow!("the spool is empty")))?;
            let offer = Offer::decode(&entry.body).map_err(store_error)?;
            (offer, Some((spool, entry)))
        }
        (None, None) => {
            return Err(exit::fault(
                Fault::Usage,
                anyhow!("give --offer FILE or --spool DIR"),
            ));
        }
    };
    let codes = UsedCodes::open(&args.used).map_err(store_error)?;
    let paired = take(me.key(), &codes, &offer, &code).map_err(store_error)?;
    let bytes = paired.encode();
    match &args.out {
        Some(path) => write_new(path, &bytes)?,
        None => {
            let mut out = std::io::stdout();
            out.write_all(&bytes)
                .and_then(|()| out.flush())
                .map_err(|e| anyhow!("stdout: {e}"))?;
        }
    }
    if let Some((spool, entry)) = entry {
        spool.consume(&entry).map_err(store_error)?;
    }
    eprintln!("paired with {}", paired.peer.hex());
    Ok(())
}

fn write_new(path: &PathBuf, bytes: &[u8]) -> Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| {
            let fault = match e.kind() {
                std::io::ErrorKind::AlreadyExists => Fault::DataError,
                _ => Fault::OsError,
            };
            exit::fault(fault, anyhow!("{}: {e}", path.display()))
        })?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|e| exit::fault(Fault::OsError, anyhow!("{}: {e}", path.display())))?;
    Ok(())
}
