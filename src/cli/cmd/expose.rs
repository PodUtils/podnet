//! `podnet expose` (T-099): one word for the three ways to publish a local
//! service. It is an alias: it builds the arguments of `podnet tunnel`,
//! `podnet relay node` or `podnet net proxy --forward`, and runs that
//! command.

use anyhow::Result;
use clap::{Args, ValueEnum};
use podnet::exit::{Fault, ResultExt};

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum Via {
    /// `podnet tunnel`: a Cloudflare tunnel.
    Tunnel,
    /// `podnet relay node`: a node of the reverse relay.
    Relay,
    /// `podnet net proxy --forward`: a virtual address on a switch.
    Switch,
}

#[derive(Args, Debug)]
pub(crate) struct ExposeArgs {
    /// The local service: an http:// or https:// URL, a port, host:port,
    /// tcp://host:port, or unix:/path.
    #[arg(value_name = "TARGET")]
    target: String,
    /// The way to publish it.
    #[arg(long, value_enum, default_value = "tunnel")]
    via: Via,
    /// With --via switch: the virtual address of the service, VIRT as in
    /// `podnet net proxy --forward VIRT=REAL` (for example 10.66.0.2:8080).
    #[arg(long = "as", value_name = "VIRT")]
    virt: Option<String>,
    /// More arguments for the command that runs (after --): for example
    /// --detach, a pair name or a --switch.
    #[arg(last = true, value_name = "ARGS")]
    rest: Vec<String>,
}

/// The arguments of `podnet` (after its name) that publish `target`.
pub(crate) fn plan(
    via: Via,
    target: &str,
    virt: Option<&str>,
    rest: &[String],
) -> Result<Vec<String>> {
    let tcp = target.strip_prefix("tcp://");
    let mut words: Vec<String> = match via {
        Via::Tunnel => {
            let (flag, value) = if target.starts_with("http://") || target.starts_with("https://") {
                ("--url", target)
            } else if target.starts_with("unix:") || tcp.is_some() {
                ("--forward", target)
            } else if target.parse::<u16>().is_ok() {
                ("--port", target)
            } else {
                ("--tcp", target)
            };
            vec!["tunnel".into(), flag.into(), value.into()]
        }
        Via::Relay => {
            if target.starts_with("http://") || target.starts_with("https://") {
                anyhow::bail!("--via relay takes host:port or unix:/path, not a URL");
            }
            let to = match target.parse::<u16>() {
                Ok(_) => format!("127.0.0.1:{target}"),
                Err(_) => tcp.unwrap_or(target).to_string(),
            };
            vec!["relay".into(), "node".into(), "--to".into(), to]
        }
        Via::Switch => {
            let virt = virt.ok_or_else(|| {
                anyhow::anyhow!("--via switch needs --as VIRT, the virtual address of the service")
            })?;
            let real = match target.parse::<u16>() {
                Ok(_) => format!("127.0.0.1:{target}"),
                Err(_) => tcp.unwrap_or(target).to_string(),
            };
            vec![
                "net".into(),
                "proxy".into(),
                "--forward".into(),
                format!("{virt}={real}"),
            ]
        }
    };
    words.extend(rest.iter().cloned());
    Ok(words)
}

/// Run the command that `args` names.
pub(crate) async fn expose(args: ExposeArgs) -> Result<()> {
    use clap::Parser as _;
    let words =
        plan(args.via, &args.target, args.virt.as_deref(), &args.rest).fault(Fault::Usage)?;
    let cli = crate::cli::Cli::try_parse_from(std::iter::once("podnet".to_string()).chain(words))
        .map_err(anyhow::Error::from)
        .fault(Fault::Usage)?;
    Box::pin(crate::cli::async_main(cli)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{cmd::relay::RelayCommand, Cli, Command};
    use clap::Parser as _;

    fn parse(words: &[String]) -> Command {
        Cli::try_parse_from(std::iter::once("podnet".to_string()).chain(words.iter().cloned()))
            .unwrap()
            .command
    }

    /// T-099, the Prove: `expose --via relay` is `relay node --to TARGET`.
    #[test]
    fn expose_via_relay_is_a_relay_node() {
        let rest = ["pair-name".to_string()];
        let words = plan(Via::Relay, "tcp://127.0.0.1:22", None, &rest).unwrap();
        assert_eq!(
            words,
            ["relay", "node", "--to", "127.0.0.1:22", "pair-name"]
        );
        match parse(&words) {
            Command::Relay(args) => match args.command {
                RelayCommand::Node { to, pair, .. } => {
                    assert_eq!(to, "127.0.0.1:22");
                    assert_eq!(pair.as_deref(), Some("pair-name"));
                }
                other => panic!("{other:?}"),
            },
            other => panic!("{other:?}"),
        }
        let words = plan(Via::Relay, "unix:/run/app.sock", None, &[]).unwrap();
        assert_eq!(words, ["relay", "node", "--to", "unix:/run/app.sock"]);
    }

    #[test]
    fn expose_via_tunnel_picks_the_origin_flag() {
        let cases = [
            (
                "https://app.internal:8443",
                ["--url", "https://app.internal:8443"],
            ),
            ("3000", ["--port", "3000"]),
            ("db.internal:5432", ["--tcp", "db.internal:5432"]),
            ("tcp://127.0.0.1:22", ["--forward", "tcp://127.0.0.1:22"]),
            ("unix:/run/app.sock", ["--forward", "unix:/run/app.sock"]),
        ];
        for (target, flag) in cases {
            let words = plan(Via::Tunnel, target, None, &[]).unwrap();
            assert_eq!(words, ["tunnel", flag[0], flag[1]], "{target}");
            assert!(matches!(parse(&words), Command::Tunnel(_)), "{target}");
        }
    }

    #[test]
    fn expose_via_switch_needs_a_virtual_address() {
        assert!(plan(Via::Switch, "127.0.0.1:3000", None, &[]).is_err());
        let words = plan(Via::Switch, "127.0.0.1:3000", Some("10.66.0.2:8080"), &[]).unwrap();
        assert_eq!(
            words,
            ["net", "proxy", "--forward", "10.66.0.2:8080=127.0.0.1:3000"]
        );
        assert!(matches!(parse(&words), Command::Net(_)));
    }
}
