//! `podnet wg` (T-125): WireGuard. `podnet wg serve` is a userspace
//! responder for a list of peers (`podnet::networks::wg_serve`).

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use podnet::exit::{Fault, ResultExt};
use podnet::network::{Ctx, Network as _, Way};

use crate::cli::signals::sleep_or_signal;

#[derive(Args, Debug)]
#[command(subcommand_required = true, arg_required_else_help = false)]
pub(crate) struct WgArgs {
    #[command(subcommand)]
    pub(crate) command: WgCommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum WgCommand {
    /// Be a WireGuard peer for a list of peers, with no TUN device and no
    /// capability. The UDP port is ListenPort of the file. A userspace
    /// stack ends the TCP and UDP flows of each peer, and dials each target
    /// through the way of --egress SPEC (a network spec: direct, a proxy
    /// URL, wg:FILE, warp, a chain; default: the way out of the process).
    /// The file is a wg-quick file: [Interface] with PrivateKey, ListenPort,
    /// Address (A.B.C.D/N: the net of the peers; the server has its first
    /// address) and MTU; one [Peer] for each peer with PublicKey, AllowedIPs
    /// (inside the net) and PresharedKey. A key that the file does not list
    /// gets no answer. A packet with a source outside the AllowedIPs of its
    /// peer is dropped. IPv4 only inside the tunnel.
    Serve(ServeArgs),
}

#[derive(Args, Debug)]
pub(crate) struct ServeArgs {
    /// The wg-quick file of the server. It holds the private key: keep it
    /// at mode 0600.
    #[arg(long, value_name = "FILE")]
    config: PathBuf,
    /// Stop after this many seconds.
    #[arg(long, value_name = "SECONDS")]
    run_for: Option<u64>,
}

pub(crate) async fn wg(args: WgArgs, egress: Vec<String>) -> Result<()> {
    match args.command {
        WgCommand::Serve(args) => serve(args, egress).await,
    }
}

/// Run the responder until a signal or --run-for.
async fn serve(args: ServeArgs, egress: Vec<String>) -> Result<()> {
    let file = podnet::networks::wg_serve::read(&args.config)
        .map_err(anyhow::Error::from)
        .fault(Fault::Usage)?;
    // The link of the way stays open while the server runs.
    let (way, _link) = match egress.as_slice() {
        [] => (Way::destination(podnet::dialers::process()), None),
        [spec] => {
            let network = podnet::networks::parse(spec)
                .map_err(|e| anyhow::anyhow!("--egress: {}", e.text()))
                .fault(Fault::Usage)?;
            let link = network
                .up(&Ctx::default())
                .await
                .map_err(podnet::networks::up_fault)?;
            let way = link.way.clone().context("the network gives no way out")?;
            (way, Some(link))
        }
        _ => {
            return Err(anyhow::anyhow!(
                "give one --egress SPEC (a chain is one spec: 'TOP over BELOW')"
            ))
            .fault(Fault::Usage)
        }
    };
    let label = args.config.display().to_string();
    let peers = file.peers();
    let mut server = podnet::networks::wg_serve::serve(file, &label, way).await?;
    let status = server.status();
    println!(
        "podnet: wg serve  udp:{}  {peers} peers  net {}  via {}",
        server.local_addr(),
        status["net"].as_str().unwrap_or("?"),
        status["via"].as_str().unwrap_or("?"),
    );
    tokio::select! {
        _ = sleep_or_signal(args.run_for) => Ok(()),
        why = server.wait() => Err(anyhow::anyhow!("podnet wg serve stopped: {why}")),
    }
}
