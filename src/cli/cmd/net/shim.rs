//! The shim commands of `podnet net`: `shim`, `tailscale`, `socksify`,
//! `passwd-shim` and `ts-shims`.

use anyhow::{Context, Result};

use podnet::exit::{Fault, ResultExt};

use super::{ShimArgs, SocksifyArgs, TailscaleArgs, TsShimsArgs};
use crate::cli::signals::sleep_or_signal;

pub(crate) fn net_shim(args: ShimArgs) -> Result<()> {
    let subnet = podnet::vnet::VirtualSubnet::from(podnet::vnet::StackConfig::default().subnet);
    let hosts = podnet::vnet::dns::Resolver::from_maps(&args.maps, &args.map_files, &subnet)
        .fault(Fault::Usage)?
        .hosts_pairs();
    // T-167: a bad route is a usage error before the shim is built.
    let routes = podnet::vnet::shim::socks_route_list(&args.socks_routes).fault(Fault::Usage)?;
    // T-357: a bad forward is a usage error too.
    let forwards = podnet::vnet::shim::forward_list(&args.forwards).fault(Fault::Usage)?;
    // H-05: resolve and check PODNET_VNET_PREFIX before building the shim so
    // an invalid or non-UTF-8 value fails fast with exit 64.
    let prefix = match std::env::var("PODNET_VNET_PREFIX") {
        Ok(value) if podnet::vnet::addr::valid_prefix(&value) => value,
        Ok(value) => {
            return Err(anyhow::anyhow!(
                "PODNET_VNET_PREFIX {value:?} is not 1 to {} bytes of letters, digits, '.', '-' or '_'",
                podnet::vnet::addr::MAX_PREFIX
            ))
            .fault(Fault::Usage);
        }
        Err(std::env::VarError::NotPresent) => podnet::vnet::shim::unique_prefix(),
        Err(std::env::VarError::NotUnicode(_)) => {
            return Err(anyhow::anyhow!("PODNET_VNET_PREFIX is not valid UTF-8"))
                .fault(Fault::Usage);
        }
    };
    // T-525: an explicit --out on a noexec file system is refused.
    let directory = podnet::state::shim_dir(args.out.clone(), "podnet-vnet")?;
    let shim = match podnet::vnet::shim::locate() {
        Some(shim) if args.out.is_none() => shim,
        _ => podnet::vnet::shim::build(&directory)?,
    };
    let options = podnet::vnet::shim::ShimOptions {
        prefix,
        log: args.log,
        map_loopback: args.map_loopback,
        hosts,
        socks_routes: if routes.is_empty() {
            Vec::new()
        } else {
            vec![routes]
        },
        socks_socket: args.socks_socket.clone(),
        forwards: if forwards.is_empty() {
            Vec::new()
        } else {
            vec![forwards]
        },
        ..podnet::vnet::shim::ShimOptions::default()
    };
    let env = options.environment(&shim.path).fault(Fault::Usage)?;
    if args.json {
        let object: serde_json::Map<String, serde_json::Value> = env
            .into_iter()
            .map(|(key, value)| (key, serde_json::Value::String(value)))
            .collect();
        println!("{}", serde_json::to_string_pretty(&object)?);
    } else {
        // The notes go to stderr, so `eval "$(podnet net shim)"` runs only
        // the exports on stdout.
        eprintln!("podnet: shim     {}", shim.path.display());
        eprintln!("podnet: origin   {:?}", shim.origin);
        for (key, value) in &env {
            println!("  export {key}={value}");
        }
    }
    Ok(())
}

pub(crate) async fn net_tailscale(args: TailscaleArgs) -> Result<()> {
    use podnet::vnet::proxy::ProxyListen;
    use podnet::vnet::tailscale::{self, Dialer};

    let listen = ProxyListen::parse(&args.listen)?;
    let kind = match &listen {
        ProxyListen::Unix(_) => podnet::net::Listener::Unix,
        ProxyListen::Tcp(_) => podnet::net::Listener::Ip,
    };
    // A live socket is exit 69, a file that is no socket is 64.
    if let ProxyListen::Unix(path) = &listen {
        podnet::net::claim_socket_path(path, false)?;
    }
    // The LocalAPI socket of tailscaled must answer before the front door
    // starts: before, a missing tailscaled was found only at the first
    // SOCKS request, and the command exited 0.
    tokio::net::UnixStream::connect(&args.socket)
        .await
        .with_context(|| {
            format!(
                "the LocalAPI socket {} of tailscaled does not answer: start tailscaled, or give --socket",
                args.socket
            )
        })
        .fault(Fault::Unavailable)?;
    let tailnet = args.tailnet.clone().or_else(|| {
        std::env::var("PODNET_TAILNET")
            .ok()
            .filter(|value| !value.trim().is_empty())
    });
    let guard = tailnet.map(tailscale::GuardConfig::new);
    let proxy =
        match tailscale::serve_guarded(Dialer::new(&args.socket), listen, args.port, guard).await {
            Ok(proxy) => proxy,
            // T-243: another tailnet is a configuration error, a daemon that is
            // not ready is an unavailable service. Nothing was bound.
            Err(err) => match err.downcast_ref::<tailscale::GuardError>() {
                Some(tailscale::GuardError::Mismatch { .. }) => {
                    return Err(err).fault(Fault::Config);
                }
                Some(tailscale::GuardError::NotReady(_)) => {
                    return Err(err).fault(Fault::Unavailable);
                }
                None => return Err(podnet::net::bind_fault_of(kind, err)),
            },
        };
    // Issue 300: the bound address (a TCP port 0 gives the port).
    println!("podnet: proxy    {}", proxy.listen);
    println!("podnet: localapi {}", args.socket);
    println!("podnet: note     run tailscaled with --tun=userspace-networking");
    println!("podnet: shim     eval \"$(podnet net socksify --socket <listen-path>)\"");
    sleep_or_signal(args.run_for).await;
    proxy.abort();
    Ok(())
}

pub(crate) fn print_environment(
    environment: Vec<(String, String)>,
    json: bool,
    export: &str,
) -> Result<()> {
    if json {
        let object: serde_json::Map<String, serde_json::Value> = environment
            .into_iter()
            .map(|(key, value)| (key, serde_json::Value::String(value)))
            .collect();
        println!("{}", serde_json::to_string_pretty(&object)?);
    } else {
        for (key, value) in &environment {
            println!("{export}{key}={value}");
        }
    }
    Ok(())
}

pub(crate) fn net_socksify(args: SocksifyArgs) -> Result<()> {
    // T-167: a bad route is a usage error before the shim is built.
    let cidrs = podnet::vnet::shim::socks_route_list(&args.cidrs).fault(Fault::Usage)?;
    // T-525: an explicit --out on a noexec file system is refused.
    let directory = podnet::state::shim_dir(args.out.clone(), "podnet-socks")?;
    let shim = match podnet::vnet::shim::locate_socks() {
        Some(shim) if args.out.is_none() => shim,
        _ => podnet::vnet::shim::build_socks(&directory)?,
    };
    let preload = match std::env::var("LD_PRELOAD") {
        Ok(existing) if !existing.is_empty() => format!("{}:{existing}", shim.path.display()),
        _ => shim.path.display().to_string(),
    };
    let mut environment = vec![
        ("LD_PRELOAD".to_string(), preload),
        ("PODNET_SOCKS_PROXY".to_string(), args.socket.clone()),
    ];
    if !cidrs.is_empty() {
        environment.push(("PODNET_SOCKS_CIDRS".to_string(), cidrs));
    }
    if args.log {
        environment.push(("PODNET_SOCKS_LOG".to_string(), "1".to_string()));
    }
    if !args.json {
        eprintln!("podnet: shim     {}", shim.path.display());
        eprintln!("podnet: origin   {:?}", shim.origin);
    }
    print_environment(environment, args.json, "export ")
}

/// Write the shims of `args`: the directory, the user and the shims.
fn write_user_shims(
    args: TsShimsArgs,
) -> Result<(
    podnet::vnet::tailscale_ssh::LocalUser,
    podnet::vnet::tailscale_ssh::Shims,
)> {
    use podnet::vnet::tailscale_ssh;

    // T-525: an explicit --out on a noexec file system is refused.
    let directory = podnet::state::shim_dir(args.out.clone(), "podnet-ts-shims")?;
    let mut user = tailscale_ssh::current_user();
    // The home of the entry is $HOME: ssh and ssh-keygen find ~/.ssh by
    // the entry, not by $HOME (T-363).
    if let Some(home) = std::env::var_os("HOME").filter(|h| std::path::Path::new(h).is_absolute()) {
        user.home = home.to_string_lossy().into_owned();
    }
    if let Some(name) = args.user {
        user.name = name;
    }
    if let Some(uid) = args.uid {
        user.uid = uid;
    }
    if let Some(gid) = args.gid {
        user.gid = gid;
    }
    if let Some(shell) = args.shell {
        user.shell = shell;
    }
    let shims = tailscale_ssh::install(&directory, std::slice::from_ref(&user))?;
    Ok((user, shims))
}

/// `net passwd-shim` (T-168): the user shims for any client. The shell form
/// is for eval: the notes go to stderr, and stdout has only the exports.
pub(crate) fn net_passwd_shim(args: TsShimsArgs) -> Result<()> {
    let json = args.json;
    let (user, shims) = write_user_shims(args)?;
    let environment = podnet::vnet::tailscale_ssh::client_environment(&shims);
    if !json {
        eprintln!("podnet: passwd   {}", shims.passwd.display());
        eprintln!(
            "podnet: user     {} uid={} gid={}",
            user.name, user.uid, user.gid
        );
        eprintln!("podnet: fakepwd  {}", shims.fakepwd.display());
    }
    print_environment(environment, json, "export ")
}

pub(crate) fn net_ts_shims(args: TsShimsArgs) -> Result<()> {
    let json = args.json;
    let (user, shims) = write_user_shims(args)?;
    let environment = podnet::vnet::tailscale_ssh::client_environment(&shims);
    if !json {
        println!("podnet: shims    {}", shims.dir.display());
        println!(
            "podnet: user     {} uid={} gid={}",
            user.name, user.uid, user.gid
        );
        println!("podnet: passwd   {}", shims.passwd.display());
        println!("podnet: fakepwd  {}", shims.fakepwd.display());
        println!(
            "podnet: daemon   PATH={}:$PATH tailscaled ... --statedir {}",
            shims.dir.display(),
            shims.dir.display()
        );
        println!("podnet: note     --statedir lets the daemon keep its SSH host keys");
    }
    print_environment(environment, json, "  export ")
}
