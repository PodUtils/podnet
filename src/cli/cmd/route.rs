//! `podnet route` (T-100): the router of a config file (`podnet::router`).

use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Args;
use podnet::exit::{Fault, ResultExt};
use podnet_netns::listen::ProxyListen;

use crate::cli::opts::DetachArgs;
use crate::cli::signals::sleep_or_signal;

#[derive(Args, Debug)]
#[command(group = clap::ArgGroup::new("change").args(["select", "rotate", "reconnect", "interval", "add_door", "remove_door", "add_network", "remove_network"]))]
pub(crate) struct RouteArgs {
    /// The config file of the router: networks, groups, health, doors and
    /// guest wires (YAML; podnet man route names the keys).
    #[arg(long, value_name = "FILE", required_unless_present_any = ["status", "control"])]
    pub(crate) config: Option<std::path::PathBuf>,
    /// Print the state of a running router: each group and member (health,
    /// round-trip time, flows), from its control socket (the key `control`
    /// of its file). Exit 69 when no router answers
    #[arg(long, value_name = "unix:/PATH", conflicts_with_all = ["config", "check"])]
    pub(crate) status: Option<String>,
    /// With --status or a control flag: print the JSON document
    #[arg(long)]
    pub(crate) json: bool,
    /// With --status: print the metrics rows of the router (one for each
    /// member: health, round-trip time, flows, bytes; the text format of
    /// Prometheus) in place of the state
    #[arg(long, requires = "status", conflicts_with = "json")]
    pub(crate) metrics: bool,
    /// Change a running router through its control socket (the key `control`
    /// of its file; mode 0600, the same user or root). Give one of --select,
    /// --rotate, --reconnect, --interval, --add-door, --remove-door,
    /// --add-network and --remove-network. Exit 69 when no router answers,
    /// 64 when it refuses the request, 75 when a rotate came too soon
    #[arg(long, value_name = "unix:/PATH", conflicts_with_all = ["config", "check", "status"])]
    pub(crate) control: Option<String>,
    /// With --control: pin a group to one member, GROUP=MEMBER; GROUP=auto
    /// gives the choice back to the policy. A pinned member that is down
    /// refuses the flows (no direct way)
    #[arg(long, value_name = "GROUP=MEMBER", requires = "control", conflicts_with_all = ["rotate", "reconnect", "interval"])]
    pub(crate) select: Option<String>,
    /// With --control: make a new identity of the network MEMBER, make the
    /// new link before the old one ends, and keep its state directory (the
    /// key `state_dir`). One rotate for each minute and network
    #[arg(long, value_name = "MEMBER", requires = "control", conflicts_with_all = ["select", "reconnect", "interval"])]
    pub(crate) rotate: Option<String>,
    /// With --control: make a new link of the network MEMBER with the same
    /// identity, make before break
    #[arg(long, value_name = "MEMBER", requires = "control", conflicts_with_all = ["select", "rotate", "interval"])]
    pub(crate) reconnect: Option<String>,
    /// With --control: the seconds between two probes (1 to 86400). The
    /// router needs a health section
    #[arg(long, value_name = "SECONDS", requires = "control", conflicts_with_all = ["select", "rotate", "reconnect"])]
    pub(crate) interval: Option<u64>,
    /// With --control: add a door while the router runs. DOOR holds its keys as
    /// YAML (listen, via, rules, allow_shadowed_rules, dns, dns_upstream) or
    /// as a JSON object. The door listens at once. Exit 64 when the router
    /// refuses it
    #[arg(long, value_name = "DOOR", requires = "control")]
    pub(crate) add_door: Option<String>,
    /// With --control: stop the door that listens on LISTEN (as the file
    /// writes it). Its open flows end at their own end; the other doors are
    /// not cut
    #[arg(long, value_name = "LISTEN", requires = "control")]
    pub(crate) remove_door: Option<String>,
    /// With --control: add a network, NAME=SPEC (a spec of docs/egress.md).
    /// A door can name it in `via` at once
    #[arg(long, value_name = "NAME=SPEC", requires = "control")]
    pub(crate) add_network: Option<String>,
    /// With --control: remove a network. Exit 64 when a door, a guest or a
    /// group uses it
    #[arg(long, value_name = "NAME", requires = "control")]
    pub(crate) remove_network: Option<String>,
    /// Read the file and bring up nothing: exit 0 when it is valid, 64 with
    /// the key that is wrong. For each tailscale:/PATH network, print the
    /// login state of tailscaled (one read of its socket).
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
        return print_status(control, args.json, args.metrics).await;
    }
    if let Some(control) = &args.control {
        return control_write(control, &args).await;
    }
    if args.json {
        return Err(anyhow::anyhow!("--json needs --status or --control")).fault(Fault::Usage);
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
        for (i, guest) in config.guests.iter().enumerate() {
            println!(
                "podnet: route  guest {i}: wire {} net {} via {}",
                guest.wire, guest.net, guest.via
            );
        }
        if let Some(kernel) = &config.kernel {
            println!("podnet: route  kernel: {} WANs", kernel.wans.len());
        }
        // T-115: the login state of each tailnet network.
        for row in podnet::networks::tailscale_rows(&config.networks).await {
            println!("podnet: route  tailnet {row}");
        }
        return Ok(());
    }
    let router = Arc::new(podnet::router::Router::up(&config).await?);
    let mut servers = tokio::task::JoinSet::new();
    router.open_doors(args.detach.replace).await?;
    for guest in &router.guests {
        let opened = guest
            .open(args.detach.replace)
            .map_err(podnet::net::bind_fault)?;
        println!(
            "podnet: route  guest {}  net {}/{}  via {}",
            guest.wire,
            guest.net.network,
            guest.net.prefix,
            guest.label()
        );
        servers.spawn(guest.clone().serve(opened));
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
        Some(ended) = servers.join_next() => ended.context("a server stopped")?,
        failed = router.door_failure() => Err(failed),
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
    let g = plan.guard;
    let on = |b: bool| if b { "on" } else { "off" };
    println!(
        "podnet: route  kernel  masquerade {}, mss clamp {}, kill switch {}, rp_filter 0 {}",
        on(g.masquerade),
        on(g.mss_clamp),
        on(g.kill_switch),
        on(g.rp_filter)
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

/// `route --control unix:/PATH --select|--rotate|--reconnect|--interval`
/// (T-180): one write to a running router, then its new state.
async fn control_write(control: &str, args: &RouteArgs) -> Result<()> {
    use serde_json::json;
    let Ok(ProxyListen::Unix(path)) = ProxyListen::parse(control) else {
        return Err(anyhow::anyhow!("--control {control:?}: give unix:/PATH")).fault(Fault::Usage);
    };
    let (method, route, body) = if let Some(spec) = &args.select {
        let Some((group, member)) = spec
            .split_once('=')
            .filter(|(g, m)| !g.is_empty() && !m.is_empty())
        else {
            return Err(anyhow::anyhow!(
                "--select {spec:?}: give GROUP=MEMBER or GROUP=auto"
            ))
            .fault(Fault::Usage);
        };
        (
            "POST",
            "/route/select",
            json!({"group": group, "member": member}),
        )
    } else if let Some(member) = &args.rotate {
        ("POST", "/route/rotate", json!({"member": member}))
    } else if let Some(member) = &args.reconnect {
        ("POST", "/route/reconnect", json!({"member": member}))
    } else if let Some(seconds) = args.interval {
        ("PATCH", "/route/interval", json!({"seconds": seconds}))
    } else if let Some(door) = &args.add_door {
        ("POST", "/route/add-door", door_keys(door)?)
    } else if let Some(listen) = &args.remove_door {
        ("POST", "/route/remove-door", json!({"listen": listen}))
    } else if let Some(spec) = &args.add_network {
        let Some((name, spec)) = spec
            .split_once('=')
            .filter(|(n, s)| !n.is_empty() && !s.is_empty())
        else {
            return Err(anyhow::anyhow!("--add-network {spec:?}: give NAME=SPEC"))
                .fault(Fault::Usage);
        };
        (
            "POST",
            "/route/add-network",
            json!({"name": name, "spec": spec}),
        )
    } else if let Some(name) = &args.remove_network {
        ("POST", "/route/remove-network", json!({"name": name}))
    } else {
        return Err(anyhow::anyhow!(
            "--control: give one of --select, --rotate, --reconnect, --interval, --add-door, --remove-door, --add-network and --remove-network"
        ))
        .fault(Fault::Usage);
    };
    let status = match podnet::router::control::request(&path, method, route, Some(&body)).await {
        Ok(status) => status,
        Err(e) => {
            let fault = match e.downcast_ref::<podnet::router::control::Answer>() {
                Some(a) if a.code == 429 => Fault::TempFail,
                Some(a) if a.code < 500 => Fault::Usage,
                _ => Fault::Unavailable,
            };
            return Err(e).fault(fault);
        }
    };
    print_state(&status, args.json);
    Ok(())
}

/// The keys of a door from `--add-door`: a JSON object, or YAML.
fn door_keys(text: &str) -> Result<serde_json::Value> {
    let value = match serde_json::from_str::<serde_json::Value>(text) {
        Ok(v) => v,
        Err(_) => podnet_core::yaml::parse(text)
            .map_err(|e| anyhow::anyhow!("--add-door: {e:#}"))
            .fault(Fault::Usage)?,
    };
    if !value.is_object() {
        return Err(anyhow::anyhow!(
            "--add-door: give the keys of a door (listen, via, ...)"
        ))
        .fault(Fault::Usage);
    }
    Ok(value)
}

/// `route --status unix:/PATH [--json]` (T-097): the state of a running
/// router.
async fn print_status(control: &str, json: bool, metrics: bool) -> Result<()> {
    let Ok(ProxyListen::Unix(path)) = ProxyListen::parse(control) else {
        return Err(anyhow::anyhow!("--status {control:?}: give unix:/PATH")).fault(Fault::Usage);
    };
    if metrics {
        let rows = podnet::router::read_metrics(&path)
            .await
            .fault(Fault::Unavailable)?;
        print!("{rows}");
        return Ok(());
    }
    let status = podnet::router::read_status(&path)
        .await
        .fault(Fault::Unavailable)?;
    print_state(&status, json);
    Ok(())
}

/// Print the status document of a router: the JSON, or the lines.
fn print_state(status: &serde_json::Value, json: bool) {
    if json {
        println!("{status:#}");
        return;
    }
    if let Some(seconds) = status["interval"].as_u64() {
        println!("probe every {seconds} s");
    }
    for door in status["doors"].as_array().into_iter().flatten() {
        println!(
            "door {}  via {}  flows {}  open {}  bytes {}",
            door["listen"].as_str().unwrap_or("?"),
            door["via"].as_str().unwrap_or("?"),
            door["flows"],
            door["open"],
            door["bytes"]
        );
    }
    for group in status["groups"].as_array().into_iter().flatten() {
        println!(
            "group {}  policy {}  pinned {}  split {}  head {}  refused {}",
            group["group"].as_str().unwrap_or("?"),
            group["policy"].as_str().unwrap_or("weighted"),
            group["pinned"].as_str().unwrap_or("-"),
            split_text(&group["split"]),
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
                "  {}  {}  rtt {rtt}  floor {floor}  flows {}  bytes {}  weight {}",
                m["name"].as_str().unwrap_or("?"),
                m["health"].as_str().unwrap_or("?"),
                m["flows"],
                m["bytes"],
                m["weight"]
            );
        }
    }
}

/// The state of the split of a group: `off`, or `udp(up A, down B)` with
/// the count of its sockets and of its fallbacks (T-179).
fn split_text(split: &serde_json::Value) -> String {
    match split["mode"].as_str() {
        Some("udp") => format!(
            "udp(up {}, down {}; sockets {}, fallbacks {})",
            split["up"].as_str().unwrap_or("?"),
            split["down"].as_str().unwrap_or("?"),
            split["sockets"],
            split["fallbacks"]
        ),
        _ => "off".to_string(),
    }
}
