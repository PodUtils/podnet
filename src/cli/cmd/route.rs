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
    #[arg(long, value_name = "FILE", required_unless_present = "status")]
    pub(crate) config: Option<std::path::PathBuf>,
    /// Print the state of a running router: each group and member (health,
    /// round-trip time, flows), from its control socket (the key `control`
    /// of its file). Exit 69 when no router answers
    #[arg(long, value_name = "unix:/PATH", conflicts_with_all = ["config", "check"])]
    pub(crate) status: Option<String>,
    /// With --status: print the JSON document
    #[arg(long, requires = "status")]
    pub(crate) json: bool,
    /// Read the file and bring up nothing: exit 0 when it is valid, 64 with
    /// the key that is wrong.
    #[arg(long)]
    pub(crate) check: bool,
    /// Print the kernel section of the file in another form, and run
    /// nothing. mwan-json: the JSON file of the mwan director (wans and
    /// clients). Exit 64 when the file has no kernel section
    #[arg(long, value_name = "FORMAT", value_parser = ["mwan-json"], conflicts_with_all = ["check", "status"])]
    pub(crate) export: Option<String>,
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
    if let Some(control) = &args.status {
        return print_status(control, args.json).await;
    }
    let Some(path) = &args.config else {
        return Err(anyhow::anyhow!("give --config FILE")).fault(Fault::Usage);
    };
    let config = podnet::router::load(path).fault(Fault::Usage)?;
    if args.export.is_some() {
        let Some(section) = &config.kernel else {
            return Err(anyhow::anyhow!(
                "--export mwan-json: the file has no kernel section"
            ))
            .fault(Fault::Usage);
        };
        let plan = podnet::router::kernel::Plan::new(section).fault(Fault::Usage)?;
        let target = config.health.as_ref().map(|h| h.target.as_str());
        println!("{:#}", plan.mwan_json(target));
        return Ok(());
    }
    if let Some(to) = &args.to {
        return check_target(&config, to, args.door);
    }
    if args.check {
        println!(
            "podnet: route  {}: {} networks, {} groups, {} doors",
            path.display(),
            config.networks.len(),
            config.groups.len(),
            config.doors.len()
        );
        if let Some(kernel) = &config.kernel {
            println!("podnet: route  kernel: {} WANs", kernel.wans.len());
        }
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
    for d in &router.doors {
        if let Some((addr, upstream)) = &d.dns {
            println!("podnet: route  dns {addr}  via {}  to {upstream}", d.label);
            servers.spawn(podnet::forwards::serve_dns(
                *addr,
                d.dial.clone(),
                upstream.clone(),
            ));
        }
    }
    if let Some(control) = &config.control {
        let Ok(ProxyListen::Unix(path)) = ProxyListen::parse(control) else {
            return Err(anyhow::anyhow!("control: give unix:/PATH")).fault(Fault::Usage);
        };
        podnet::net::claim_socket_path(&path, args.detach.replace)?;
        let listener = podnet_netns::listen::bind_private(&path)
            .with_context(|| format!("control unix:{}", path.display()))
            .map_err(podnet::net::bind_fault)?;
        println!("podnet: route  control unix:{}", path.display());
        servers.spawn(router.clone().serve_control(listener));
    }
    let kernel = apply_kernel(&config)?;
    podnet::detach::ready(router.status());
    let probing = router.clone();
    let ended = tokio::select! {
        _ = sleep_or_signal(args.run_for) => Ok(()),
        _ = probing.probe_forever() => Ok(()),
        Some(ended) = servers.join_next() => ended.context("a door stopped")?,
    };
    if let Some(plan) = kernel {
        if let Err(e) = podnet::router::kernel::remove(&plan) {
            eprintln!("podnet: route  kernel: {e:#}");
        }
    }
    ended
}

/// Write the kernel mode of the file (T-119), if it has one.
fn apply_kernel(config: &podnet::router::Config) -> Result<Option<podnet::router::kernel::Plan>> {
    use podnet::router::kernel;
    let Some(section) = &config.kernel else {
        return Ok(None);
    };
    let plan = kernel::Plan::new(section).fault(Fault::Config)?;
    kernel::apply(&plan, None).fault(Fault::Unavailable)?;
    println!(
        "podnet: route  kernel  {} WANs, aggregate table {}",
        plan.wans.len(),
        plan.aggregate
    );
    Ok(Some(plan))
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

/// `route --status unix:/PATH [--json]` (T-097): the state of a running
/// router.
async fn print_status(control: &str, json: bool) -> Result<()> {
    let Ok(ProxyListen::Unix(path)) = ProxyListen::parse(control) else {
        return Err(anyhow::anyhow!("--status {control:?}: give unix:/PATH")).fault(Fault::Usage);
    };
    let status = podnet::router::read_status(&path)
        .await
        .fault(Fault::Unavailable)?;
    if json {
        println!("{status:#}");
        return Ok(());
    }
    for door in status["doors"].as_array().into_iter().flatten() {
        println!(
            "door {}  via {}",
            door["listen"].as_str().unwrap_or("?"),
            door["via"].as_str().unwrap_or("?")
        );
    }
    for group in status["groups"].as_array().into_iter().flatten() {
        println!(
            "group {}  policy {}  head {}  refused {}",
            group["group"].as_str().unwrap_or("?"),
            group["policy"].as_str().unwrap_or("weighted"),
            group["head"].as_str().unwrap_or("-"),
            group["refused"]
        );
        for m in group["members"].as_array().into_iter().flatten() {
            let rtt = m["rtt_ms"]
                .as_f64()
                .map_or("-".to_string(), |ms| format!("{ms:.1} ms"));
            let floor = m["rtt_floor_ms"]
                .as_f64()
                .map_or("-".to_string(), |ms| format!("{ms:.1} ms"));
            println!(
                "  {}  {}  rtt {rtt}  floor {floor}  flows {}  weight {}",
                m["name"].as_str().unwrap_or("?"),
                m["health"].as_str().unwrap_or("?"),
                m["flows"],
                m["weight"]
            );
        }
    }
    Ok(())
}
