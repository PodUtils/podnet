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
    /// With --check: a dry run for the target HOST:PORT. Print the rule, the
    /// network or the group, and the member of each door, with no dial. Exit
    /// 1 when a door refuses the target or its group has no member up
    #[arg(long, value_name = "HOST:PORT", requires = "check")]
    pub(crate) to: Option<String>,
    /// With --to: only the door at this position of the file (from 0)
    #[arg(long, value_name = "INDEX", requires = "to")]
    pub(crate) door: Option<usize>,
    /// Stop after this many seconds.
    #[arg(long, value_name = "SECONDS")]
    pub(crate) run_for: Option<u64>,
    #[command(flatten)]
    pub(crate) detach: DetachArgs,
}

/// Run the router until a signal or --run-for.
pub(crate) async fn route(args: RouteArgs) -> Result<()> {
    let config = podnet::router::load(&args.config).fault(Fault::Usage)?;
    if let Some(to) = &args.to {
        return check_target(&config, to, args.door);
    }
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
    for podnet::router::RouterDoor {
        listen,
        door,
        label,
        ..
    } in &router.doors
    {
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

/// `route --check --to HOST:PORT` (T-177): the choice of each door, with the
/// same function as the router and no dial.
fn check_target(config: &podnet::router::Config, to: &str, only: Option<usize>) -> Result<()> {
    let target: podnet::network::Target = to
        .parse()
        .map_err(|_| anyhow::anyhow!("--to {to:?}: give HOST:PORT"))
        .fault(Fault::Usage)?;
    let router = podnet::router::Router::plan(config).fault(Fault::Config)?;
    let doors: Vec<usize> = match only {
        Some(i) if i >= router.doors.len() => {
            return Err(anyhow::anyhow!(
                "--door {i}: the file has {} doors (0 to {})",
                router.doors.len(),
                router.doors.len() - 1
            ))
            .fault(Fault::Usage);
        }
        Some(i) => vec![i],
        None => (0..router.doors.len()).collect(),
    };
    let mut refused = Vec::new();
    println!("door\tlisten\trule\tway\tmember\thealth");
    for i in doors {
        let decision = router.decide(i, &target);
        let (member, health) = decision
            .member
            .clone()
            .unwrap_or_else(|| ("-".to_string(), "-".to_string()));
        println!(
            "{i}\t{}\t{}\t{}\t{member}\t{health}",
            router.doors[i].listen,
            decision.rule.as_deref().unwrap_or("default"),
            decision.way.as_deref().unwrap_or("reject"),
        );
        if !decision.passes(&router.groups) {
            refused.push(i.to_string());
        }
    }
    match refused.as_slice() {
        [] => Ok(()),
        [one] => Err(anyhow::anyhow!("the door {one} refuses {target}")),
        many => Err(anyhow::anyhow!(
            "the doors {} refuse {target}",
            many.join(", ")
        )),
    }
}
