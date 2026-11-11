//! `podnet forward` (T-099): port forwards over a network of the registry.

use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Args;
use podnet::exit::{Fault, ResultExt};
use podnet::network::{Ctx, Dial, Network as _};
use podnet_netns::listen::ProxyListen;

use crate::cli::signals::sleep_or_signal;

#[derive(Args, Debug)]
pub(crate) struct ForwardArgs {
    /// A forward: LISTEN=HOST:PORT (repeatable). LISTEN is PORT (on
    /// 127.0.0.1), ADDR:PORT, tcp://ADDR:PORT or unix:/PATH. Each
    /// connection to LISTEN goes through the network to HOST:PORT.
    #[arg(
        short = 'L',
        long = "local",
        value_name = "LISTEN=HOST:PORT",
        required = true
    )]
    local: Vec<String>,
    /// The network of the forwards, as a spec: direct, a proxy URL,
    /// relay or relay:URL, tailscale:/PATH, unix:/PATH. Default: the way
    /// out of the process (--proxy, the variables, else direct).
    #[arg(long, value_name = "SPEC")]
    network: Option<String>,
    /// Stop after this many seconds.
    #[arg(long, value_name = "SECONDS")]
    run_for: Option<u64>,
}

/// Run the forwards until a signal or --run-for.
pub(crate) async fn forward(args: ForwardArgs) -> Result<()> {
    let rules = args
        .local
        .iter()
        .map(|spec| podnet::forwards::Rule::parse(spec))
        .collect::<Result<Vec<_>>>()
        .fault(Fault::Usage)?;
    let dial: Arc<dyn Dial> = match &args.network {
        Some(spec) => {
            let network = podnet::networks::parse(spec)
                .map_err(anyhow::Error::from)
                .fault(Fault::Usage)?;
            let link = network.up(&Ctx::default()).await?;
            link.way.context("the network gives no way out")?.dial
        }
        None => podnet::dialers::process(),
    };
    let mut servers = tokio::task::JoinSet::new();
    for rule in rules {
        let target = rule.target.clone();
        match &rule.listen {
            ProxyListen::Tcp(addr) => {
                let listener = tokio::net::TcpListener::bind(addr)
                    .await
                    .with_context(|| format!("-L {addr}"))
                    .map_err(podnet::net::bind_fault)?;
                println!(
                    "podnet: forward  {}  ->  {target}  via {}",
                    listener.local_addr()?,
                    dial.label()
                );
                servers.spawn(podnet::forwards::serve_tcp(listener, dial.clone(), target));
            }
            ProxyListen::Unix(path) => {
                podnet::net::claim_socket_path(path, false)?;
                let listener = tokio::net::UnixListener::bind(path)
                    .with_context(|| format!("-L unix:{}", path.display()))
                    .map_err(podnet::net::bind_fault)?;
                {
                    use std::os::unix::fs::PermissionsExt as _;
                    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
                }
                println!(
                    "podnet: forward  unix:{}  ->  {target}  via {}",
                    path.display(),
                    dial.label()
                );
                servers.spawn(podnet::forwards::serve_unix(listener, dial.clone(), target));
            }
        }
    }
    tokio::select! {
        _ = sleep_or_signal(args.run_for) => Ok(()),
        Some(ended) = servers.join_next() => ended.context("a forward stopped")?,
    }
}
