//! `podnet route` (T-100): the router of a config file (`podnet::router`).

use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Args;
use podnet::exit::{Fault, ResultExt};
use podnet_netns::listen::ProxyListen;

use crate::cli::opts::DetachArgs;
use crate::cli::signals::sleep_or_signal;

#[derive(Args, Debug)]
pub(crate) struct RouteArgs {
    /// The config file of the router: networks, groups, health and doors
    /// (YAML; podnet man route names the keys).
    #[arg(long, value_name = "FILE")]
    pub(crate) config: std::path::PathBuf,
    /// Read the file and bring up nothing: exit 0 when it is valid, 64 with
    /// the key that is wrong.
    #[arg(long)]
    pub(crate) check: bool,
    /// Stop after this many seconds.
    #[arg(long, value_name = "SECONDS")]
    pub(crate) run_for: Option<u64>,
    #[command(flatten)]
    pub(crate) detach: DetachArgs,
}

/// Run the router until a signal or --run-for.
pub(crate) async fn route(args: RouteArgs) -> Result<()> {
    let config = podnet::router::load(&args.config).fault(Fault::Usage)?;
    if args.check {
        println!(
            "podnet: route  {}: {} networks, {} groups, {} doors",
            args.config.display(),
            config.networks.len(),
            config.groups.len(),
            config.doors.len()
        );
        return Ok(());
    }
    let router = Arc::new(podnet::router::Router::up(&config).await?);
    let mut servers = tokio::task::JoinSet::new();
    for (listen, door, label) in &router.doors {
        match listen {
            ProxyListen::Tcp(addr) => {
                let listener = tokio::net::TcpListener::bind(addr)
                    .await
                    .with_context(|| format!("door {addr}"))
                    .map_err(podnet::net::bind_fault)?;
                println!(
                    "podnet: route  door {}  via {label}",
                    listener.local_addr()?
                );
                servers.spawn(door.clone().serve_tcp(listener));
            }
            ProxyListen::Unix(path) => {
                podnet::net::claim_socket_path(path, args.detach.replace)?;
                let listener = podnet_netns::listen::bind_private(path)
                    .with_context(|| format!("door unix:{}", path.display()))
                    .map_err(podnet::net::bind_fault)?;
                println!("podnet: route  door unix:{}  via {label}", path.display());
                servers.spawn(door.clone().serve_unix(listener));
            }
        }
    }
    podnet::detach::ready(router.status());
    let probing = router.clone();
    tokio::select! {
        _ = sleep_or_signal(args.run_for) => Ok(()),
        _ = probing.probe_forever() => Ok(()),
        Some(ended) = servers.join_next() => ended.context("a door stopped")?,
    }
}
