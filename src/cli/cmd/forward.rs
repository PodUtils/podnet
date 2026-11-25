//! `podnet forward` (T-099): port forwards over a network of the registry.

use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Args;
use podnet::exit::{Fault, ResultExt};
use podnet::network::{Addressing, Ctx, Dial, Network as _, Target};
use podnet_netns::listen::ProxyListen;

use crate::cli::signals::sleep_or_signal;

#[derive(Args, Debug)]
pub(crate) struct ForwardArgs {
    /// A forward: LISTEN=HOST:PORT (repeatable). LISTEN is PORT (on 127.0.0.1),
    /// ADDR:PORT, tcp://ADDR:PORT or unix:/PATH. Each connection to LISTEN goes
    /// through the network to HOST:PORT. With LISTEN=peer:FINGERPRINT/SERVICE
    /// each connection opens one stream of a link to that peer, for its
    /// service. A running `podnet link connect` or `link listen` holds the
    /// link; PODNET_LINK_CONTROL names its control socket. A peer that is not
    /// the fingerprint exits 77, and no link that runs exits 69. With udp:PORT
    /// or udp:ADDR:PORT the forward carries UDP: each client has its own flow,
    /// and a flow ends after 60 s with no datagram. A datagram for a full flow
    /// is dropped.
    #[arg(short = 'L', long = "local", value_name = "LISTEN=HOST:PORT")]
    local: Vec<String>,
    /// A name for a client that takes only HOST:PORT: NAME=HOST:PORT
    /// (repeatable, 254 at most). Each name gets the next address of
    /// 127.0.0.2 to 127.0.0.255, a listener there on PORT (or on
    /// --alias-port), and a forward to HOST:PORT. --dns answers each name
    /// with its address.
    #[arg(long = "name", value_name = "NAME=HOST:PORT")]
    names: Vec<String>,
    /// The port of the listeners of --name, for each name. The default is
    /// the PORT of each name.
    #[arg(long, value_name = "PORT", requires = "names")]
    alias_port: Option<u16>,
    /// A DNS door: answer DNS queries on ADDR:PORT (UDP and TCP) through
    /// the network, from the resolver at --dns-upstream. The names of
    /// --name get their address from the door itself. A query that cannot
    /// go to the upstream gets SERVFAIL. An address in use exits 69.
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
    /// 127.0.0.1; the token comes from PODNET_DEVTUNNEL_TOKEN_FILE or
    /// PODNET_DEVTUNNEL_ACCESS_TOKEN_FILE, or PODNET_DEVTUNNEL_ANONYMOUS=1
    /// sends none), or
    /// warp or warp:identity=FILE (TCP and UDP through a WARP session; names
    /// resolve inside the tunnel; the device is kept in FILE, else in the
    /// default identity file of podnet warp). A SOCKS5 or HTTP proxy carries no UDP. Default:
    /// the way out of the process (--proxy, the variables, else direct).
    #[arg(long, value_name = "SPEC")]
    network: Option<String>,
    /// Stop after this many seconds.
    #[arg(long, value_name = "SECONDS")]
    run_for: Option<u64>,
}

/// Run the forwards until a signal or --run-for.
pub(crate) async fn forward(args: ForwardArgs) -> Result<()> {
    if args.local.is_empty() && args.dns.is_none() && args.names.is_empty() {
        return Err(anyhow::anyhow!(
            "give -L LISTEN=HOST:PORT, --name NAME=HOST:PORT or --dns ADDR:PORT"
        ))
        .fault(Fault::Usage);
    }
    let aliases = podnet::forwards::aliases(&args.names).fault(Fault::Usage)?;
    let upstream: Option<Target> = args
        .dns
        .map(|_| {
            args.dns_upstream
                .parse::<Target>()
                .map_err(|_| anyhow::anyhow!("--dns-upstream: give HOST:PORT"))
        })
        .transpose()
        .fault(Fault::Usage)?;
    // Each name is a rule on its address (T-404).
    // With no --alias-port, each name listens on its own target port
    // (#625): each name has its own address, so the ports can differ.
    let alias_rules = aliases.iter().map(|a| {
        let port = args.alias_port.unwrap_or_else(|| {
            a.target
                .parse::<Target>()
                .map(|t| t.port())
                .unwrap_or_default()
        });
        format!("{}:{port}={}", a.ip, a.target)
    });
    let rules = args
        .local
        .iter()
        .cloned()
        .chain(alias_rules)
        .map(|spec| podnet::forwards::Rule::parse(&spec))
        .collect::<Result<Vec<_>>>()
        .fault(Fault::Usage)?;
    let dial: Arc<dyn Dial> = match &args.network {
        Some(spec) => {
            let network = podnet::networks::parse(spec)
                .map_err(anyhow::Error::from)
                .fault(Fault::Usage)?;
            // A usage fault of the network is exit 64, a refused key 77.
            let link = network
                .up(&Ctx::default())
                .await
                .map_err(podnet::networks::up_fault)?;
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
        // GitHub #638: the line comes after the bind; a bind that fails
        // prints no line.
        let (udp, tcp) = podnet::forwards::bind_dns(addr).await?;
        println!(
            "podnet: dns door  {}  ->  {upstream}  via {}",
            bound(udp.local_addr(), addr),
            dial.label()
        );
        let table =
            (!aliases.is_empty()).then(|| Arc::new(podnet::forwards::alias_table(&aliases)));
        servers.spawn(podnet::forwards::serve_dns_on(
            udp,
            tcp,
            dial.clone(),
            upstream,
            table,
        ));
    }
    for a in &aliases {
        // The line of a hosts file, for a client with no --dns.
        println!("podnet: name  {}  {}", a.ip, a.name);
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
            // egress-exempt: the UDP listener of -L
            let socket = tokio::net::UdpSocket::bind(addr)
                .await
                .with_context(|| format!("-L udp:{addr}"))
                .map_err(podnet::net::bind_fault_udp(addr.port()))?;
            println!(
                "podnet: forward  udp:{}  ->  {shown}  via {}",
                bound(socket.local_addr(), addr),
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
                    bound(listener.local_addr(), addr),
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

/// The address of a bound socket for the line of a listener. A host that
/// refuses getsockname(2) (the lab `getsockname-denied` of pod-lab) gets the
/// address of the command line and the reason, not the end of the command
/// (T-452): the listener works with no known address.
fn bound(addr: std::io::Result<std::net::SocketAddr>, asked: impl std::fmt::Display) -> String {
    match addr {
        Ok(addr) => addr.to_string(),
        Err(e) => format!("{asked} (the host refused getsockname: {e})"),
    }
}

#[cfg(test)]
mod tests {
    /// T-502: a refused getsockname(2) gives the address of the command line
    /// and the reason, not an error that ends the command.
    #[test]
    fn a_refused_getsockname_shows_the_asked_address() {
        let ok: std::io::Result<std::net::SocketAddr> = Ok("127.0.0.1:8080".parse().unwrap());
        assert_eq!(super::bound(ok, "127.0.0.1:0"), "127.0.0.1:8080");
        let refused = Err(std::io::Error::from_raw_os_error(libc::EOPNOTSUPP));
        let line = super::bound(refused, "127.0.0.1:17411");
        assert!(
            line.starts_with("127.0.0.1:17411 (the host refused getsockname: "),
            "{line}"
        );
    }
}
