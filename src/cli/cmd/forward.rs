//! `podnet forward` (T-099): port forwards over a network of the registry.

use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Args;
use podnet::exit::{Fault, ResultExt};
use podnet::network::{Addressing, Class, Ctx, Dial, Failure, Network as _, Target};
use podnet_netns::listen::ProxyListen;

use crate::cli::signals::sleep_or_signal;

#[derive(Args, Debug)]
pub(crate) struct ForwardArgs {
    /// A forward: LISTEN=HOST:PORT (repeatable). LISTEN is PORT (on
    /// 127.0.0.1), ADDR:PORT, tcp://ADDR:PORT or unix:/PATH. Each
    /// connection to LISTEN goes through the network to HOST:PORT. With
    /// LISTEN=peer:FINGERPRINT/SERVICE each connection opens one stream of
    /// a link to that peer, for its service (the link is held by a running
    /// `podnet link connect` or `link listen`; PODNET_LINK_CONTROL names its
    /// control socket; a peer that is not the fingerprint exits 77, and no
    /// link that runs exits 69). With
    /// udp:PORT or udp:ADDR:PORT the forward carries UDP: each client has
    /// its own flow, and a flow ends after 60 s with no datagram. A
    /// datagram for a full flow is dropped.
    #[arg(short = 'L', long = "local", value_name = "LISTEN=HOST:PORT")]
    local: Vec<String>,
    /// A DNS door: answer DNS queries on ADDR:PORT (UDP and TCP) through
    /// the network, from the resolver at --dns-upstream.
    #[arg(long, value_name = "ADDR:PORT")]
    dns: Option<std::net::SocketAddr>,
    /// The resolver of --dns, an address at the far end of the network.
    #[arg(long, value_name = "HOST:PORT", default_value = "1.1.1.1:53")]
    dns_upstream: String,
    /// The network of the forwards, as a spec: direct, a proxy URL,
    /// relay or relay:URL, tailscale:/PATH, netbird:[PORT], wg:FILE,
    /// unix:/PATH, turn://HOST[:PORT] or turns://HOST[:PORT] (UDP flows
    /// through an allocation on a TURN server; the user and the password
    /// come from PODNET_TURN_USER and PODNET_TURN_PASSWORD_FILE; a TCP
    /// forward is refused), or devtunnel://TUNNEL (TCP to a port of the host
    /// of a Dev Tunnels tunnel through its relay; the target host is
    /// 127.0.0.1; the token comes from PODNET_DEVTUNNEL_TOKEN_FILE). A SOCKS5 or HTTP proxy carries no UDP. Default:
    /// the way out of the process (--proxy, the variables, else direct).
    #[arg(long, value_name = "SPEC")]
    network: Option<String>,
    /// Stop after this many seconds.
    #[arg(long, value_name = "SECONDS")]
    run_for: Option<u64>,
}

/// Run the forwards until a signal or --run-for.
pub(crate) async fn forward(args: ForwardArgs) -> Result<()> {
    if args.local.is_empty() && args.dns.is_none() {
        return Err(anyhow::anyhow!(
            "give -L LISTEN=HOST:PORT or --dns ADDR:PORT"
        ))
        .fault(Fault::Usage);
    }
    let upstream: Option<Target> = args
        .dns
        .map(|_| {
            args.dns_upstream
                .parse::<Target>()
                .map_err(|_| anyhow::anyhow!("--dns-upstream: give HOST:PORT"))
        })
        .transpose()
        .fault(Fault::Usage)?;
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
            // A usage fault of the network (a missing TURN account) is exit 64.
            let link = network.up(&Ctx::default()).await.map_err(|e| {
                match e.downcast_ref::<Failure>() {
                    Some(f) if f.class == Class::Usage => podnet::exit::fault(Fault::Usage, e),
                    _ => e,
                }
            })?;
            let way = link.way.context("the network gives no way out")?;
            // A peer way meets a peer: it takes no HOST:PORT.
            Addressing::Destination
                .admit(std::slice::from_ref(&way))
                .map_err(|e| anyhow::anyhow!("{e}; give -L LISTEN=peer:FINGERPRINT/SERVICE"))
                .fault(Fault::Usage)?;
            way.dial
        }
        None => podnet::dialers::process(),
    };
    let mut servers = tokio::task::JoinSet::new();
    if let (Some(addr), Some(upstream)) = (args.dns, upstream) {
        println!(
            "podnet: dns door  {addr}  ->  {upstream}  via {}",
            dial.label()
        );
        servers.spawn(podnet::forwards::serve_dns(addr, dial.clone(), upstream));
    }
    for rule in rules {
        let target = rule.target.clone();
        // A rule `peer:FINGERPRINT/SERVICE` has its own network. The check
        // of the link runs here, before the listener: a peer that is not
        // the fingerprint exits 77.
        let dial = match &rule.via {
            Some(spec) => peer_dial(spec).await?,
            None => dial.clone(),
        };
        let shown = rule.via.clone().unwrap_or_else(|| target.to_string());
        if rule.udp {
            let ProxyListen::Tcp(addr) = &rule.listen else {
                unreachable!("Rule::parse admits TCP addresses for UDP")
            };
            let socket = tokio::net::UdpSocket::bind(addr)
                .await
                .with_context(|| format!("-L udp:{addr}"))
                .map_err(podnet::net::bind_fault)?;
            println!(
                "podnet: forward  udp:{}  ->  {shown}  via {}",
                socket.local_addr()?,
                dial.label()
            );
            servers.spawn(podnet::forwards::serve_udp(
                Arc::new(socket),
                dial.clone(),
                target,
            ));
            continue;
        }
        match &rule.listen {
            ProxyListen::Tcp(addr) => {
                let listener = tokio::net::TcpListener::bind(addr)
                    .await
                    .with_context(|| format!("-L {addr}"))
                    .map_err(podnet::net::bind_fault)?;
                println!(
                    "podnet: forward  {}  ->  {shown}  via {}",
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
                    "podnet: forward  unix:{}  ->  {shown}  via {}",
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

/// The dialer of the network `peer:FINGERPRINT/SERVICE`. The network checks
/// the link first (77: a peer that is not the fingerprint; 69: none runs).
async fn peer_dial(spec: &str) -> Result<Arc<dyn Dial>> {
    let network = podnet::networks::parse(spec)
        .map_err(anyhow::Error::from)
        .fault(Fault::Usage)?;
    let link = network.up(&Ctx::default()).await?;
    Ok(link.way.context("the network gives no way out")?.dial)
}
