//! `podnet offline discover`: a signed mDNS announcement.

use std::time::Duration;

use anyhow::Result;
use clap::{Args, Subcommand};

use podnet::offline::mdns;

use super::key;

#[derive(Args, Debug)]
pub(crate) struct DiscoverArgs {
    #[command(subcommand)]
    pub(crate) command: DiscoverCommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum DiscoverCommand {
    /// Sign an announcement and send it to the mDNS group, or to one
    /// address.
    Announce(AnnounceArgs),
    /// Hear announcements for a time and list the peers whose signature
    /// verified. A bad signature is counted and ignored.
    Listen(ListenArgs),
}

#[derive(Args, Debug)]
pub(crate) struct AnnounceArgs {
    /// The name of this peer: one line, with no control byte.
    #[arg(long, value_name = "NAME")]
    pub(crate) name: String,
    /// The address that a peer dials: a host, a path or a serial path.
    #[arg(long, value_name = "ADDRESS")]
    pub(crate) address: String,
    /// The port that a peer dials.
    #[arg(long, value_name = "PORT")]
    pub(crate) port: u16,
    /// The key that signs the announcement. Default: the state directory.
    #[arg(long, value_name = "DIR")]
    pub(crate) key: Option<std::path::PathBuf>,
    /// The UDP destination: the mDNS group by default.
    #[arg(long, value_name = "IP:PORT", default_value = mdns::GROUP)]
    pub(crate) to: String,
    /// Send the announcement this many times. Default 1.
    #[arg(long, value_name = "N", default_value_t = 1)]
    pub(crate) repeat: u32,
}

#[derive(Args, Debug)]
pub(crate) struct ListenArgs {
    /// The UDP address to bind. The wildcard and the group join 224.0.0.251.
    #[arg(long, value_name = "IP:PORT", default_value = "0.0.0.0:5353")]
    pub(crate) bind: String,
    /// Seconds to listen.
    #[arg(long, value_name = "SECONDS", default_value_t = 5)]
    pub(crate) timeout: u64,
    /// One JSON document: {"peers": [...], "ignored": N, "records": N}.
    #[arg(long)]
    pub(crate) json: bool,
}

pub(crate) fn discover(args: DiscoverArgs) -> Result<()> {
    match args.command {
        DiscoverCommand::Announce(args) => announce(args),
        DiscoverCommand::Listen(args) => listen(args),
    }
}

fn announce(args: AnnounceArgs) -> Result<()> {
    let me = key(&args.key, true)?;
    let announcement = mdns::announce(
        me.key(),
        &args.name,
        &args.address,
        args.port,
        &args.to,
        args.repeat,
    )?;
    println!(
        "announced {} {}:{} {} to {}",
        announcement.name,
        announcement.address,
        announcement.port,
        announcement.key.hex(),
        args.to
    );
    Ok(())
}

fn listen(args: ListenArgs) -> Result<()> {
    let heard = mdns::listen(&args.bind, Duration::from_secs(args.timeout))?;
    let peers = heard.discovery.peers();
    if args.json {
        let rows: Vec<serde_json::Value> = peers
            .iter()
            .map(|peer| {
                serde_json::json!({
                    "name": peer.name,
                    "address": peer.address,
                    "port": peer.port,
                    "key": peer.key.hex(),
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::json!({
                "peers": rows,
                "records": heard.records,
                "ignored": heard.ignored,
            })
        );
    } else {
        for peer in &peers {
            println!(
                "{} {}:{} {}",
                peer.name,
                peer.address,
                peer.port,
                peer.key.hex()
            );
        }
        eprintln!(
            "{} peer(s), {} record(s), {} ignored",
            peers.len(),
            heard.records,
            heard.ignored
        );
    }
    Ok(())
}
