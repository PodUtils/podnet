//! `podnet net proxy`: a SOCKS5 and HTTP CONNECT proxy into the virtual
//! network, and the real targets of its forwards.

use std::sync::Arc;

use anyhow::{Context, Result};

use podnet::exit::{Fault, ResultExt};
use podnet::vnet::VirtAddr;

use super::real::RealTarget;
use super::ProxyArgs;
use crate::cli::cmd::run::listen_doc;
use crate::cli::signals::sleep_or_signal;

/// A probability from 0.0 to 1.0 (issue 196).
pub(crate) fn probability(value: &str) -> std::result::Result<f64, String> {
    let p: f64 = value
        .parse()
        .map_err(|_| format!("{value} is not a number"))?;
    if !(0.0..=1.0).contains(&p) {
        return Err(format!("{value} is not from 0.0 to 1.0"));
    }
    Ok(p)
}

pub(crate) async fn net_proxy(args: ProxyArgs) -> Result<()> {
    use podnet::vnet::proxy::{ProxyListen, Resolver};

    // T-193: one resolver for the door, the gateway (10.66.0.1:53) and the
    // stack; a bad entry is a usage error before anything starts.
    let defaults = podnet::vnet::StackConfig::default();
    let names =
        podnet::vnet::dns::Resolver::from_maps(&args.maps, &args.map_files, &defaults.subnet)
            .fault(Fault::Usage)?;
    let names = Arc::new(names);
    let mut _link = None;
    let net = match &args.switch {
        None => Arc::new(podnet::vnet::NetStack::loopback_with(
            podnet::vnet::StackConfig {
                dns: Some(names.clone()),
                ..defaults
            },
        )),
        Some(switch) => {
            let address = args
                .address
                .context("--switch needs --address, the address of this stack")
                .fault(Fault::Usage)?;
            let config = podnet::vnet::StackConfig {
                local_v4: address,
                dns: Some(names.clone()),
                ..defaults
            };
            let net = Arc::new(podnet::vnet::NetStack::over_link(config));
            let faults = podnet::vnet::switch::Faults {
                drop: args.drop,
                duplicate: args.duplicate,
                reorder: args.reorder,
                seed: args.seed,
            };
            let switch = ProxyListen::parse(switch)
                .context("--switch")
                .fault(Fault::Usage)?;
            // Issue 309: a TCP switch needs its shared token.
            let token = podnet::vnet::switch::token(args.switch_token_file.as_deref())
                .fault(Fault::Usage)?;
            if let Some(token) = &token {
                podnet::report::hide(token);
            }
            if matches!(switch, ProxyListen::Tcp(_)) && token.is_none() {
                return Err(anyhow::anyhow!(
                    "a TCP switch needs its token: set {}, or give --switch-token-file",
                    podnet::vnet::switch::TOKEN_VARIABLE
                ))
                .fault(Fault::Usage);
            }
            _link = Some(
                podnet::vnet::switch::attach(&net, &switch, faults, token.as_deref())
                    .await
                    .fault(Fault::Unavailable)?,
            );
            #[cfg(target_os = "linux")]
            {
                if let Some(name) = &args.control {
                    podnet::vnet::switch::serve_control(net.clone(), name)?;
                    println!("podnet: control  \\0{name}  (PODNET_VNET_CONTROL={name})");
                }
                for spec in &args.imports {
                    let virt: VirtAddr = spec.trim().parse()?;
                    podnet::vnet::switch::import(net.clone(), virt)?;
                    println!("podnet: import   {virt}");
                }
            }
            match &switch {
                ProxyListen::Unix(path) => {
                    println!("podnet: switch   unix:{} as {address}", path.display())
                }
                ProxyListen::Tcp(addr) => println!("podnet: switch   tcp://{addr} as {address}"),
            }
            net
        }
    };
    let resolver: Resolver = names.hosts_pairs().into_iter().collect();
    let listen = ProxyListen::parse(&args.listen)?;
    let forwards = parse_forwards(&args.forwards)?;
    let mut proxy = podnet::vnet::proxy::serve(net.clone(), listen, resolver, args.port).await?;
    for (virt, real) in forwards {
        let dial_real = real.clone();
        proxy
            .spawn_forwards(net.clone(), virt, move || {
                let real = dial_real.clone();
                async move { real.dial().await }
            })
            .await
            .map_err(|err| forward_fault(virt, err))?;
        println!("podnet: forward  {virt} -> {}", real.describe());
    }
    // Issue 300: the bound address (a TCP port 0 gives the port).
    println!("podnet: proxy    {}", proxy.listen);
    podnet::detach::ready(listen_doc(&proxy.listen));
    sleep_or_signal(args.run_for).await;
    proxy.abort();
    Ok(())
}

/// Parse each `--forward VIRT=REAL`. A repeated `VIRT` is a usage error.
fn parse_forwards(specs: &[String]) -> Result<Vec<(VirtAddr, RealTarget)>> {
    let mut out: Vec<(VirtAddr, RealTarget)> = Vec::new();
    for spec in specs {
        let (virt, real) = spec
            .split_once('=')
            .with_context(|| format!("--forward wants VIRT=REAL, got {spec:?}"))
            .fault(Fault::Usage)?;
        let virt: VirtAddr = virt.trim().parse().fault(Fault::Usage)?;
        if out.iter().any(|(seen, _)| *seen == virt) {
            return Err(anyhow::anyhow!("--forward {virt} is given more than once"))
                .fault(Fault::Usage);
        }
        out.push((virt, RealTarget::parse(real.trim()).fault(Fault::Usage)?));
    }
    Ok(out)
}

/// The class of a refused bind of a forward: an address that the subnet does
/// not hold is a usage error; a taken address or an ACL refusal is 69.
fn forward_fault(virt: VirtAddr, err: std::io::Error) -> anyhow::Error {
    let class = match err.kind() {
        std::io::ErrorKind::AddrNotAvailable => Fault::Usage,
        _ => Fault::Unavailable,
    };
    podnet::exit::fault(
        class,
        anyhow::Error::new(err).context(format!("--forward {virt} could not bind")),
    )
}
