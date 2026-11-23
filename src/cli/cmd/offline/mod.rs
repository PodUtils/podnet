//! `podnet offline`: the stores and the discovery of a host with no network.
//!
//! One command for each part of PodUtils/podnet#499 that is not a carrier:
//! `spool` (store and forward on a directory), `discover` (a signed mDNS
//! announcement), `bundle` (a signed offline bundle and its delta),
//! `capture` (one record format and a merge), `pair` (an eight-word single-use
//! code bound to a key) and `replica` (an append-only log with a merge).
//!
//! The carriers that a session uses without a network are `--carrier
//! serial:PATH[,BAUD]` and `--carrier hf:PATH[,BPS]` of `podnet link`.

mod bundle;
mod capture;
mod discover;
mod pair;
mod replica;
mod spool;

use anyhow::Result;
use clap::{Args, Subcommand};

#[derive(Args, Debug)]
#[command(subcommand_required = true, arg_required_else_help = false)]
pub(crate) struct OfflineArgs {
    #[command(subcommand)]
    pub(crate) command: OfflineCommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum OfflineCommand {
    /// A store-and-forward spool: a directory of files, each with a
    /// checksum, a sequence and the sender's key.
    Spool(spool::SpoolArgs),
    /// A signed announcement over mDNS, and the peers that this host hears.
    Discover(discover::DiscoverArgs),
    /// An offline bundle with a signed manifest, and its delta update.
    Bundle(bundle::BundleArgs),
    /// One record format for captures, merged by timestamp with redaction
    /// at write time.
    Capture(capture::CaptureArgs),
    /// An eight-word single-use code, bound to the Ed25519 key of this
    /// install, exchanged through a file or a spool.
    Pair(pair::PairArgs),
    /// An append-only replication log: last-writer-wins for each key with a
    /// vector clock and a stated tie rule.
    Replica(replica::ReplicaArgs),
}

pub(crate) fn offline(args: OfflineArgs) -> Result<()> {
    match args.command {
        OfflineCommand::Spool(args) => spool::spool(args),
        OfflineCommand::Discover(args) => discover::discover(args),
        OfflineCommand::Bundle(args) => bundle::bundle(args),
        OfflineCommand::Capture(args) => capture::capture(args),
        OfflineCommand::Pair(args) => pair::pair(args),
        OfflineCommand::Replica(args) => replica::replica(args),
    }
}

/// The key of this install in `dir`, made when `make` is true. The default
/// directory is the state directory of podnet.
pub(crate) fn key(
    dir: &Option<std::path::PathBuf>,
    make: bool,
) -> Result<podnet::identity::Identity> {
    let dir = match dir {
        Some(dir) => dir.clone(),
        None => podnet::identity::default_dir()?,
    };
    if make {
        podnet::identity::create(&dir)
    } else {
        podnet::identity::open(&dir)
    }
}

/// One exit class for each failure of a store, so that a script can tell a
/// missing file from bad data. The codes follow `podnet man exit-status`.
pub(crate) fn store_error(e: pod_offline::Error) -> anyhow::Error {
    use pod_offline::Error;
    use podnet::exit::{self, Fault};
    let fault = match &e {
        Error::File { source, .. } if source.kind() == std::io::ErrorKind::NotFound => {
            Fault::NoInput
        }
        Error::File { .. } => Fault::OsError,
        Error::Address { .. }
        | Error::Baud { .. }
        | Error::Device { .. }
        | Error::Open { .. }
        | Error::Settings { .. }
        | Error::Window { .. }
        | Error::Speed { .. } => Fault::Usage,
        Error::CodeUsed | Error::CodeExpired => Fault::NoPermission,
        Error::Manifest { .. }
        | Error::WrongBase { .. }
        | Error::HashMismatch { .. }
        | Error::Spool { .. }
        | Error::SequenceTaken { .. }
        | Error::EntryCorrupt { .. }
        | Error::Txt { .. }
        | Error::Capture { .. }
        | Error::Code { .. }
        | Error::Proof
        | Error::Log { .. }
        | Error::Radio { .. } => Fault::DataError,
        Error::FrameTooLarge { .. }
        | Error::Frame { .. }
        | Error::KissEscape { .. }
        | Error::Ack { .. }
        | Error::Busy { .. }
        | Error::Receiving
        | Error::Modem { .. } => Fault::Unavailable,
    };
    exit::fault(fault, anyhow::Error::new(e))
}

/// The hex form of `bytes`.
pub(crate) fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// The next free sequence of a spool: one above its highest entry.
pub(crate) fn next_sequence(spool: &pod_offline::spool::Spool) -> Result<u64> {
    let entries = spool.entries().map_err(store_error)?;
    Ok(entries.last().map_or(0, |entry| entry.sequence + 1))
}
