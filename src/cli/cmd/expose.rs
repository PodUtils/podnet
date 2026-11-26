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
    /// The local service. --via tunnel: an http:// or https:// URL (an
    /// HTTP origin), a port, host:port (raw TCP), tcp://host:port or
    /// unix:/path. --via relay and --via switch carry TCP: a port,
    /// host:port, tcp://host:port or unix:/path; a URL exits 64 and names
    /// the HOST:PORT to give.
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
    /// tunnel --detach, the pair name of relay node, or net proxy --switch.
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
            if is_url(target) {
                return Err(no_url("relay", target));
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
            if is_url(target) {
                return Err(no_url("switch", target));
            }
            let real = match target.parse::<u16>() {
                Ok(_) => format!("127.0.0.1:{target}"),
                Err(_) => tcp.unwrap_or(target).to_string(),
            };
            let mut words: Vec<String> = vec![
                "net".into(),
                "proxy".into(),
                "--forward".into(),
                format!("{virt}={real}"),
            ];
            // #773: on a switch, the stack needs its address: the address
            // of --as, unless the words after `--` give one.
            let named = |flag: &str| {
                rest.iter()
                    .any(|w| w == flag || w.starts_with(&format!("{flag}=")))
            };
            if named("--switch") && !named("--address") {
                let ip = virt.rsplit_once(':').map_or(virt, |(ip, _)| ip);
                words.extend(["--address".into(), ip.to_string()]);
            }
            words
        }
    };
    words.extend(rest.iter().cloned());
    Ok(words)
}

fn is_url(target: &str) -> bool {
    target.starts_with("http://") || target.starts_with("https://")
}

/// The refusal of a URL on a road that carries TCP (GitHub #646). It names
/// the HOST:PORT of the URL (port 80 or 443 when the URL has none).
fn no_url(via: &str, target: &str) -> anyhow::Error {
    let hint = url_host_port(target)
        .map(|a| format!(": give {a}"))
        .unwrap_or_default();
    anyhow::anyhow!(
        "--via {via} carries TCP: give HOST:PORT, tcp://HOST:PORT or unix:/PATH, not a URL{hint}"
    )
}

fn url_host_port(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let has_port = host.rsplit_once(':').is_some_and(|(h, p)| {
        !h.ends_with(':') && !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())
    });
    match (host.is_empty(), has_port, scheme) {
        (true, _, _) => None,
        (false, true, _) => Some(host.to_string()),
        (false, false, "https") => Some(format!("{host}:443")),
        (false, false, _) => Some(format!("{host}:80")),
    }
}

/// The command line and the parse of the command that `cli` (an `expose`)
/// stands for (issue 612). main runs that command as if the user typed
/// it: the arguments after `--` (--detach, --name) reach the detach of
/// main. The global options of `cli` stay; a global option after `--`
/// replaces the one of `cli`. The token file of the arguments after `--`
/// is read before the parse.
pub(crate) fn resolve(
    argv: &[String],
    cli: crate::cli::Cli,
) -> std::result::Result<(Vec<String>, crate::cli::Cli), clap::Error> {
    use clap::error::ErrorKind;
    use clap::FromArgMatches as _;
    let crate::cli::Command::Expose(args) = &cli.command else {
        return Ok((argv.to_vec(), cli));
    };
    let fail = |kind: ErrorKind, err: anyhow::Error| {
        let mut top = crate::cli::help::top_command();
        top.build();
        top.find_subcommand_mut("expose").map_or_else(
            || clap::Error::raw(kind, format!("{err:#}\n")),
            |sub| sub.error(kind, format!("{err:#}")),
        )
    };
    let words = plan(args.via, &args.target, args.virt.as_deref(), &args.rest)
        .map_err(|e| fail(ErrorKind::MissingRequiredArgument, e))?;
    // T-060: each --egress after `--` becomes its old flag, as in main.
    let inner = crate::cli::way::egress_aliases(
        std::iter::once("podnet".to_string())
            .chain(words)
            .map(Into::into)
            .collect(),
    )
    .map_err(|e| fail(ErrorKind::ValueValidation, anyhow::anyhow!(e)))?;
    let inner: Vec<String> = inner
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    crate::cli::argv::apply_token_files(&inner).map_err(|e| fail(ErrorKind::ValueValidation, e))?;
    let parsed = crate::cli::help::top_command()
        .try_get_matches_from(&inner)
        .and_then(|matches| crate::cli::Cli::from_arg_matches(&matches))?;
    Ok((inner, merge(cli, parsed)))
}

/// `outer` with the command of `inner`, and each global option that
/// `inner` sets.
fn merge(outer: crate::cli::Cli, inner: crate::cli::Cli) -> crate::cli::Cli {
    let pick = |inner: String, outer: String, default: &str| {
        if inner == default {
            outer
        } else {
            inner
        }
    };
    crate::cli::Cli {
        ca_file: inner.ca_file.or(outer.ca_file),
        proxy: inner.proxy.or(outer.proxy),
        no_proxy: inner.no_proxy || outer.no_proxy,
        egress: if inner.egress.is_empty() {
            outer.egress
        } else {
            inner.egress
        },
        resolve: inner.resolve.or(outer.resolve),
        quiet: inner.quiet || outer.quiet,
        verbose: inner.verbose.max(outer.verbose),
        color: pick(inner.color, outer.color, "auto"),
        log_format: pick(inner.log_format, outer.log_format, "text"),
        command: inner.command,
    }
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

    /// #773: `--via switch` with a `--switch` after `--` gives the stack
    /// the address of --as; an --address after `--` stays the one.
    #[test]
    fn expose_via_a_switch_gives_the_stack_the_address_of_as() {
        let words = |tail: &str| {
            let argv: Vec<String> =
                format!("podnet expose 3000 --via switch --as 10.66.0.2:8080 -- {tail}")
                    .split_whitespace()
                    .map(String::from)
                    .collect();
            let cli = Cli::try_parse_from(&argv).unwrap();
            resolve(&argv, cli).unwrap().0
        };
        let given = words("--switch unix:/tmp/s.sock");
        let at = given
            .iter()
            .position(|w| w == "--address")
            .expect("--address");
        assert_eq!(given[at + 1], "10.66.0.2");
        let own = words("--switch unix:/tmp/s.sock --address 10.66.0.9");
        assert_eq!(own.iter().filter(|w| *w == "--address").count(), 1);
        assert!(own.contains(&"10.66.0.9".to_string()));
        assert!(!words("--detach").contains(&"--address".to_string()));
    }

    /// Issue 612: the arguments after `--` reach the detach of main, and
    /// the global options before and after `expose` stay.
    #[test]
    fn expose_gives_its_command_line_to_main() {
        let argv: Vec<String> = "podnet -q expose 3000 --via switch --as 10.66.0.2:8080 \
             --proxy socks5h://127.0.0.1:1 -- --detach --name web -v"
            .split_whitespace()
            .map(String::from)
            .collect();
        let cli = Cli::try_parse_from(&argv).unwrap();
        let (inner, cli) = resolve(&argv, cli).unwrap();
        assert_eq!(
            inner,
            [
                "podnet",
                "net",
                "proxy",
                "--forward",
                "10.66.0.2:8080=127.0.0.1:3000",
                "--detach",
                "--name",
                "web",
                "-v"
            ]
        );
        let detach = crate::cli::opts::detach_of(&cli.command).expect("net proxy detaches");
        assert!(detach.detach);
        assert_eq!(detach.name.as_deref(), Some("web"));
        assert!(cli.quiet && cli.verbose == 1);
        assert_eq!(cli.proxy.as_deref(), Some("socks5h://127.0.0.1:1"));
        assert_eq!(
            crate::cli::argv::command_name(&inner).as_deref(),
            Some("net proxy")
        );
        for (via, extra) in [("tunnel", &[][..]), ("relay", &["pair-name"][..])] {
            let mut argv: Vec<String> = ["podnet", "expose", "3000", "--via", via, "--"]
                .map(String::from)
                .to_vec();
            argv.extend(extra.iter().map(|w| w.to_string()));
            argv.push("--detach".into());
            let cli = Cli::try_parse_from(&argv).unwrap();
            let (_, cli) = resolve(&argv, cli).unwrap();
            let detach = crate::cli::opts::detach_of(&cli.command).expect(via);
            assert!(detach.detach, "{via}");
        }
        let argv: Vec<String> = ["podnet", "expose", "3000", "--via", "switch"]
            .map(String::from)
            .to_vec();
        let cli = Cli::try_parse_from(&argv).unwrap();
        let err = resolve(&argv, cli).unwrap_err();
        assert!(err.to_string().contains("--as VIRT"), "{err}");
        assert!(err.render().to_string().contains("podnet expose"), "{err}");
    }

    /// GitHub #646: a URL on a road that carries TCP exits 64 and names the
    /// HOST:PORT to give.
    #[test]
    fn a_url_on_a_tcp_road_names_the_host_and_port() {
        for (via, name) in [(Via::Switch, "switch"), (Via::Relay, "relay")] {
            for (url, want) in [
                ("http://127.0.0.1:8081", "give 127.0.0.1:8081"),
                ("http://127.0.0.1:8081/app", "give 127.0.0.1:8081"),
                ("https://app.internal", "give app.internal:443"),
                ("http://[::1]/", "give [::1]:80"),
            ] {
                let err = plan(via, url, Some("10.66.0.2:8080"), &[]).unwrap_err();
                let text = format!("{err:#}");
                assert!(
                    text.contains(&format!("--via {name} carries TCP")),
                    "{text}"
                );
                assert!(text.contains(want), "{url}: {text}");
            }
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
