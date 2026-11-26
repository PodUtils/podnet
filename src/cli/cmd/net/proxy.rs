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
    let names = podnet::vnet::dns::Resolver::from_maps(
        &args.maps,
        &args.map_files,
        &defaults.subnet.into(),
    )
    .fault(Fault::Usage)?;
    let names = Arc::new(names);
    let mut _link = None;
    let net = match &args.switch {
        None => Arc::new(podnet::vnet::NetStack::loopback_with(
            podnet::vnet::StackConfig {
                gateway_service: Some(names.clone()),
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
                gateway_service: Some(names.clone()),
                ..defaults
            };
            let net = Arc::new(podnet::vnet::NetStack::over_link(config));
            let faults = podnet::vnet::switch::Faults {
                drop: args.drop,
                duplicate: args.duplicate,
                reorder: args.reorder,
                seed: args.seed,
            };
            let switch = switch_of(switch).await?;
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

/// The switch of `--switch`: `unix:/PATH`, or `tcp://HOST:PORT` that this
/// stack dials. A name is resolved here (#764: it was read as a listen
/// address, which takes an IP only); a name that does not resolve exits 68.
async fn switch_of(text: &str) -> Result<podnet::vnet::proxy::ProxyListen> {
    use podnet::vnet::proxy::ProxyListen;
    let rest = ["tcp://", "tcp:"].iter().find_map(|p| text.strip_prefix(p));
    let Some(rest) = rest.filter(|r| r.parse::<std::net::SocketAddr>().is_err()) else {
        return ProxyListen::parse(text)
            .context("--switch")
            .fault(Fault::Usage);
    };
    let (host, port) = rest
        .rsplit_once(':')
        .and_then(|(h, p)| Some((h.trim_matches(['[', ']']), p.parse::<u16>().ok()?)))
        .filter(|(h, _)| !h.is_empty())
        .ok_or_else(|| anyhow::anyhow!("--switch {text}: give tcp://HOST:PORT"))
        .fault(Fault::Usage)?;
    let ips = podnet::resolve::resolve(host)
        .await
        .with_context(|| format!("--switch {text}"))
        .fault(Fault::NoHost)?;
    let ip = ips
        .first()
        .copied()
        .ok_or_else(|| anyhow::anyhow!("--switch {text}: {host} has no address"))
        .fault(Fault::NoHost)?;
    Ok(ProxyListen::Tcp(std::net::SocketAddr::new(ip, port)))
}

#[cfg(test)]
mod switch_tests {
    use super::switch_of;
    use podnet::vnet::proxy::ProxyListen;

    /// #764: `--switch tcp://HOST:PORT` takes a name, which it resolves; an
    /// IP and a unix path stay as before; a form with no port is a usage
    /// error.
    #[tokio::test]
    async fn a_switch_takes_a_name_an_ip_or_a_path() {
        let named = switch_of("tcp://localhost:9000").await.unwrap();
        assert!(
            matches!(named, ProxyListen::Tcp(a) if a.ip().is_loopback() && a.port() == 9000),
            "{named:?}"
        );
        let ip = switch_of("tcp://127.0.0.1:7000").await.unwrap();
        assert!(matches!(ip, ProxyListen::Tcp(a) if a.port() == 7000));
        let path = switch_of("unix:/run/s.sock").await.unwrap();
        assert!(matches!(path, ProxyListen::Unix(_)));
        for bad in ["tcp://localhost", "tcp://:9000", "tcp://localhost:x"] {
            let e = switch_of(bad).await.unwrap_err();
            assert_eq!(
                podnet::exit::code_of(&e),
                podnet::exit::USAGE,
                "{bad}: {e:#}"
            );
        }
    }
}
