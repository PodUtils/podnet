//! `podnet tunnel`: the arguments, the origin and the `--check` report.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::Args;

use podnet::config::Config;
use podnet::exit::{Fault, ResultExt};
use podnet::ingress::{ForwardTarget, Ingress, IngressRule, OriginOptions, Service};

use crate::cli::cmd::run::run_tunnel;
use crate::cli::cmd::tunnel_inspect::{self, TunnelAction};
use crate::cli::opts::{Show, TunnelOpts};

#[derive(Args, Debug)]
#[command(args_conflicts_with_subcommands = true)]
pub(crate) struct TunnelArgs {
    #[command(subcommand)]
    pub(crate) action: Option<TunnelAction>,
    #[command(flatten)]
    pub(crate) common: TunnelOpts,
    /// HTTP or HTTPS origin.
    /// Not with --config.
    #[arg(long, value_parser = clap::builder::NonEmptyStringValueParser::new())]
    url: Option<String>,
    /// HTTP origin on a unix socket.
    /// Not with --config.
    #[arg(long)]
    unix: Option<PathBuf>,
    /// HTTP origin on 127.0.0.1:<PORT>.
    /// Not with --config.
    #[arg(long, value_parser = clap::value_parser!(u16).range(1..))]
    port: Option<u16>,
    /// Raw TCP origin.
    /// Not with --config.
    #[arg(long, value_parser = clap::builder::NonEmptyStringValueParser::new())]
    tcp: Option<String>,
    /// Expose a raw stream over a WebSocket endpoint (tcp://host:port or unix:/path).
    /// Not with --config.
    #[arg(long, value_parser = clap::builder::NonEmptyStringValueParser::new())]
    forward: Option<String>,
    /// cloudflared's built-in hello world.
    /// Not with --config.
    #[arg(long)]
    hello_world: bool,
    /// cloudflared-style YAML config with ingress rules.
    #[arg(long)]
    config: Option<PathBuf>,
    /// Named-tunnel token. Prefer the variable TUNNEL_TOKEN or
    /// --token-file: other users of the host can read a command line.
    #[arg(long, env = "TUNNEL_TOKEN", hide_env_values = true)]
    token: Option<String>,
    /// Read the named-tunnel token from this file (for example a secret
    /// of a container). The token is not on the command line.
    #[arg(
        long,
        value_name = "PATH",
        env = "TUNNEL_TOKEN_FILE",
        hide_env_values = true
    )]
    token_file: Option<PathBuf>,
    /// cloudflared credentials JSON file.
    #[arg(long)]
    credentials_file: Option<PathBuf>,
    /// Skip TLS verification of the origin.
    /// Not with --config.
    #[arg(long)]
    no_tls_verify: bool,
    /// Override the Host header sent to the origin.
    /// Not with --config.
    #[arg(long)]
    http_host_header: Option<String>,
    /// Drop Accept-Encoding so the origin cannot compress a streaming response.
    /// Not with --config.
    #[arg(long)]
    strip_accept_encoding: bool,
    /// Run the session end to end: a Noise XX handshake on
    /// the data stream before any application byte, the destination in a
    /// sealed envelope, and record keys from the handshake. The relay
    /// forwards and holds no key.
    #[arg(long)]
    pub(crate) e2e: bool,
    /// The near end's data address (IP:PORT), where the relay forwards.
    #[arg(long, value_name = "IP:PORT")]
    pub(crate) connect: Option<String>,
    /// The near end's control address (IP:PORT): the sealed destination
    /// goes here, out of band.
    #[arg(long, value_name = "IP:PORT")]
    pub(crate) control: Option<String>,
    /// The far end's data listen address (IP:PORT).
    #[arg(long, value_name = "IP:PORT")]
    pub(crate) listen: Option<String>,
    /// The far end's control listen address (IP:PORT).
    #[arg(long = "control-listen", value_name = "IP:PORT")]
    pub(crate) control_listen: Option<String>,
    /// The pair that the relay advertises: RELAY/PAIR.
    #[arg(long, value_name = "RELAY/PAIR")]
    pub(crate) pair: Option<String>,
    /// A suite of the records: chacha20-poly1305 (default) or
    /// aes-256-gcm. Give it twice to offer both.
    #[arg(long, value_name = "NAME")]
    pub(crate) suite: Vec<String>,
    /// The destination, HOST:PORT. The far end reads it from the sealed
    /// envelope; the near end puts it there.
    #[arg(long, value_name = "HOST:PORT")]
    pub(crate) destination: Option<String>,
    /// The service name at the destination.
    #[arg(long, default_value = "tcp", value_name = "NAME")]
    pub(crate) service: String,
    /// A peer that this end admits: 64 hex digits of its Ed25519 key. Give
    /// it once for each peer.
    #[arg(long, value_name = "KEY")]
    pub(crate) peer: Vec<String>,
    /// The compatibility path: the destination in clear on the control
    /// stream, as /connect/HOST/PORT. It leaks the destination, and the
    /// command prints that warning.
    #[arg(long)]
    pub(crate) compat_destination: bool,
    /// Print the hop knowledge table of --e2e and the exchange as a hop
    /// sees it.
    #[arg(long)]
    pub(crate) e2e_inspect: bool,
    /// Keep the last HTTP exchanges of the origin in memory (off by
    /// default): the method, the target, the status, the heads with each
    /// secret value redacted, and the bodies up to --inspect-body. podnet
    /// tunnel inspect list, show and replay read them over a unix socket
    /// of mode 0600.
    #[arg(long)]
    inspect: bool,
    /// With --inspect: the count of exchanges in the ring (default 100).
    /// A full ring drops the oldest.
    #[arg(long, value_name = "N", requires = "inspect", value_parser = clap::value_parser!(u64).range(1..=10_000))]
    inspect_keep: Option<u64>,
    /// With --inspect: the bytes of each body that the ring keeps (default
    /// 65536, at most 16777216). The rest is counted, not kept.
    #[arg(long, value_name = "BYTES", requires = "inspect", value_parser = clap::value_parser!(u64).range(1..=16_777_216))]
    inspect_body: Option<u64>,
    /// With --inspect: keep the values of Authorization, Cookie and each
    /// header or query parameter that names a token or a key. podnet
    /// prints a warning.
    #[arg(long, requires = "inspect")]
    inspect_raw: bool,
    /// A token of one relay and one service (LEARN-2). It is refused by
    /// another relay or another service.
    #[arg(long, value_name = "TOKEN", hide_env_values = true)]
    pub(crate) e2e_token: Option<String>,
    /// The file of the 32-byte seed that minted the token, as 64 hex
    /// digits on its first line.
    #[arg(long, value_name = "FILE")]
    pub(crate) e2e_token_seed_file: Option<PathBuf>,
}

/// The origin flags of `args`, by name. `--config` has the origin in its
/// `ingress:` rules, so these flags have no effect with it (T-204).
fn origin_flags(args: &TunnelArgs) -> Vec<&'static str> {
    [
        ("--url", args.url.is_some()),
        ("--unix", args.unix.is_some()),
        ("--port", args.port.is_some()),
        ("--tcp", args.tcp.is_some()),
        ("--forward", args.forward.is_some()),
        ("--hello-world", args.hello_world),
        ("--no-tls-verify", args.no_tls_verify),
        ("--http-host-header", args.http_host_header.is_some()),
        ("--strip-accept-encoding", args.strip_accept_encoding),
    ]
    .into_iter()
    .filter_map(|(name, given)| given.then_some(name))
    .collect()
}

pub(crate) async fn tunnel(args: TunnelArgs) -> Result<()> {
    if let Some(TunnelAction::Inspect { command }) = &args.action {
        return tunnel_inspect::run(command, args.common.json).await;
    }
    if args.e2e || args.e2e_inspect {
        return super::e2e::tunnel_e2e(&args).await;
    }
    let given = origin_flags(&args);
    if args.config.is_some() && !given.is_empty() {
        return Err(podnet::exit::fault(
            Fault::Usage,
            anyhow::anyhow!(
                "tunnel --config: {} has no effect with a config file; put the origin in `ingress:` of the file",
                given.join(", ")
            ),
        ));
    }
    let mut config = if let Some(path) = &args.config {
        podnet::config::load(path).fault(Fault::Config)?
    } else {
        Config::default()
    };
    // Issue 172: a token of the command line and --credentials-file are a
    // conflict; a token of TUNNEL_TOKEN gives way to the flag.
    let token = match (&args.token, &args.credentials_file) {
        (Some(token), Some(path)) => {
            if std::env::var("TUNNEL_TOKEN").ok().as_deref() != Some(token.as_str()) {
                return Err(podnet::exit::fault(
                    Fault::Usage,
                    anyhow::anyhow!("give --token or --credentials-file, not both"),
                ));
            }
            tracing::warn!(
                "TUNNEL_TOKEN is set; --credentials-file {} is used",
                path.display()
            );
            None
        }
        (token, _) => token.as_ref(),
    };
    if let Some(token) = token {
        podnet::report::hide(token);
        config.credentials = podnet::config::parse_token(token).fault(Fault::Config)?;
    } else if let Some(path) = &args.credentials_file {
        config.credentials = podnet::config::parse_credentials_file(path).fault(Fault::Config)?;
    }

    if args.config.is_none() {
        let mut ingress = Ingress::new();
        if let Some(forward) = &args.forward {
            let target = ForwardTarget::parse(forward).fault(Fault::Usage)?;
            ingress = ingress.rule_with(IngressRule {
                hostname: None,
                path: Some(podnet::ingress::PathPattern::prefix(
                    podnet::ws::DEFAULT_PATH,
                )),
                service: Service::Forward(target),
                origin: OriginOptions::default(),
            });
        }
        if let Some(service) = pick_optional_service(&args).fault(Fault::Usage)? {
            let origin = OriginOptions {
                no_tls_verify: args.no_tls_verify,
                http_host_header: args.http_host_header.clone(),
                strip_accept_encoding: args.strip_accept_encoding,
                ..OriginOptions::default()
            };
            ingress = ingress.rule_with(IngressRule {
                hostname: None,
                path: None,
                service,
                origin,
            });
        }
        if ingress.is_empty() {
            // T-244: --inspect was the hop table of --e2e before.
            let hint = if args.inspect {
                "; the hop table of --e2e is --e2e-inspect"
            } else {
                ""
            };
            return Err(podnet::exit::fault(
                Fault::Usage,
                anyhow::anyhow!(
                    "choose an origin (--url, --unix, --port, --tcp, --hello-world) or --forward{hint}"
                ),
            ));
        }
        config.ingress = ingress;
    }
    config = args.common.apply(config)?;
    // A forward-only tunnel has no HTTP origin to verify; the root is expected
    // to answer 503. A raw TCP origin answers HTTP with 501 (issue 199).
    if !config
        .ingress
        .rules
        .iter()
        .any(|rule| !matches!(rule.service, Service::Forward(_) | Service::Tcp(_)))
    {
        if config
            .ingress
            .rules
            .iter()
            .any(|rule| matches!(rule.service, Service::Tcp(_)))
            && config.verify
        {
            tracing::info!("a raw TCP origin does not answer HTTP: no verify of the public URL");
        }
        config.verify = false;
    }

    if args.common.check {
        return preflight(&config, args.common.show()).await;
    }
    // The socket of the inspector lives as long as the tunnel.
    let _inspector = match args.inspect {
        true => Some(tunnel_inspect::start(
            &tunnel_inspect::Flags {
                keep: args.inspect_keep.map(|n| n as usize),
                body: args.inspect_body.map(|n| n as usize),
                raw: args.inspect_raw,
            },
            &config,
        )?),
        false => None,
    };
    run_tunnel(config, args.common.show(), args.common.run_for, vec![]).await
}

/// `--check`: print the report of the check, and fail when a check failed.
pub(crate) async fn preflight(config: &Config, show: Show) -> Result<()> {
    print_report(podnet::preflight::check(config).await, show)
}

/// Print the report of a check; a failed check gives its exit code.
pub(crate) fn print_report(report: podnet::preflight::Report, show: Show) -> Result<()> {
    if show.json {
        println!("{}", podnet::status::stamped(report.to_json()));
    } else {
        print!("{}", report.text());
    }
    let failed = report
        .checks
        .iter()
        .filter(|c| c.status == podnet::preflight::Status::Fail)
        .count();
    if failed > 0 {
        let err = anyhow::anyhow!(
            "the check failed ({failed} of {}); nothing was started",
            report.checks.len()
        );
        // Issue 173: a failed check of the configuration gives its code
        // (78), as a run does. A failed origin or network check gives 1.
        return Err(match report.fault() {
            Some(class) => podnet::exit::fault(class, err),
            None => err,
        });
    }
    Ok(())
}

/// Split `host:port`, `[v6]:port` or `ip:port` into the host and the port.
pub(crate) fn split_host_port(spec: &str) -> Result<(String, u16)> {
    let (host, port) = spec
        .rsplit_once(':')
        .with_context(|| format!("{spec}: give host:port"))?;
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let port: u16 = port
        .parse()
        .ok()
        .filter(|p| *p != 0)
        .with_context(|| format!("{spec}: the port must be 1 to 65535"))?;
    if host.is_empty() || host.contains(['/', ' ']) {
        bail!("{spec}: give host:port");
    }
    Ok((host.to_string(), port))
}

fn pick_optional_service(args: &TunnelArgs) -> Result<Option<Service>> {
    let mut choices = 0;
    let mut service = None;
    if let Some(url) = &args.url {
        choices += 1;
        if url
            .split("://")
            .nth(1)
            .and_then(|r| r.split('/').next())
            .is_some_and(|a| a.ends_with(":0"))
        {
            bail!("--url {url}: the port 0 is not a port");
        }
        service = Some(Service::parse(url).with_context(|| format!("--url {url}"))?);
    }
    if let Some(path) = &args.unix {
        choices += 1;
        use std::os::unix::fs::FileTypeExt;
        match std::fs::metadata(path) {
            Ok(meta) if meta.file_type().is_socket() => {}
            Ok(_) => bail!("--unix {}: not a Unix socket", path.display()),
            Err(err) => bail!("--unix {}: {err}", path.display()),
        }
        service = Some(Service::Unix(path.clone()));
    }
    if let Some(port) = args.port {
        choices += 1;
        service = Some(Service::Http(format!("http://127.0.0.1:{port}")));
    }
    if let Some(addr) = &args.tcp {
        choices += 1;
        // Issue 199: tcp://host:port is accepted; another scheme is not.
        let addr = &addr.strip_prefix("tcp://").unwrap_or(addr).to_string();
        if addr.contains("://") {
            bail!("--tcp {addr}: give host:port or tcp://host:port");
        }
        let (host, port) = addr
            .rsplit_once(':')
            .ok_or_else(|| anyhow::anyhow!("--tcp {addr}: give host:port"))?;
        if host.is_empty() || port.parse::<u16>().map_or(true, |p| p == 0) {
            bail!("--tcp {addr}: give host:port with a port from 1 to 65535");
        }
        service = Some(Service::Tcp(addr.clone()));
    }
    if args.hello_world {
        choices += 1;
        service = Some(Service::HelloWorld);
    }
    if choices > 1 {
        bail!("choose at most one origin: --url, --unix, --port, --tcp, --hello-world");
    }
    Ok(service)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    struct Probe {
        #[command(flatten)]
        args: TunnelArgs,
    }

    #[test]
    fn origin_flags_are_named() {
        let parse = |a: &[&str]| Probe::try_parse_from([&["p"], a].concat()).unwrap().args;
        assert!(origin_flags(&parse(&["--config", "f.yml"])).is_empty());
        let args = parse(&["--config", "f.yml", "--port", "80", "--no-tls-verify"]);
        assert_eq!(origin_flags(&args), ["--port", "--no-tls-verify"]);
    }
}
