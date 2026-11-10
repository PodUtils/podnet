//! `podnet warp`: the WARP client. The prologue of the global options and the
//! dispatch to the files of the subcommands.

pub(crate) mod account;
pub(crate) mod args;
pub(crate) mod client;
pub(crate) mod export;
pub(crate) mod front;
pub(crate) mod kept;
pub(crate) mod pod;
pub(crate) mod probe;
pub(crate) mod run;
pub(crate) mod tun;
pub(crate) mod up;

#[cfg(test)]
mod cli_tests;

use std::time::Duration;

use anyhow::Result;

use podnet::exit::{Fault, ResultExt};

pub(crate) use args::{WarpArgs, WarpCommand};
use client::warp_client;

pub(crate) async fn warp(mut args: WarpArgs) -> Result<()> {
    // Issue 111: a bad PODNET_WARP_TCP_BURST fails before any network wait.
    podnet::warp::check_tcp_burst()?;
    // Issue 134: the congestion controller of the stack.
    if let Some(cc) = &args.cc {
        podnet::warp::ip::set_cc(podnet::warp::ip::parse_cc(cc)?);
    }
    // Issue 185: the renewal of the session.
    if let Some(secs) = args.session_max_age {
        podnet::warp::live::set_max_age(Some(Duration::from_secs(secs)));
    }
    if let Some(probe) = args.probe {
        podnet::warp::live::set_probe(probe);
    }
    // Issue 145: the resolvers of the tunnel.
    let resolvers = podnet::warp::proxy::parse_resolvers(&args.resolver).fault(Fault::Usage)?;
    podnet::warp::proxy::set_resolvers(resolvers);
    // Issue 135: the split of the ClientHello of the MASQUE connection.
    if let Some(split) = podnet::warp::live::parse_split(
        args.fragment_client_hello.as_deref().unwrap_or("off"),
        args.fragment_size.as_deref(),
    )
    .fault(Fault::Config)?
    {
        podnet::warp::live::set_split(split);
    }
    // Issue 141: the name, the model and the locale of a new device.
    podnet::warp::register::DeviceHints {
        name: args.device_name.clone(),
        model: args.device_model.clone(),
        locale: args.device_locale.clone(),
    }
    .set()
    .fault(Fault::Usage)?;
    // These commands use the running instance of podnet warp up.
    if up::before_client(&args).await? {
        return Ok(());
    }
    if export::before_client(&args).await? || front::before_client(&args)? {
        return Ok(());
    }
    // These commands use a saved device and register none.
    if matches!(
        args.command,
        WarpCommand::Status { .. }
            | WarpCommand::License { .. }
            | WarpCommand::Rotate
            | WarpCommand::Delete
    ) && !args.ephemeral
    {
        // Issue 178: find the saved identity with no directory made, in each
        // candidate directory; a stat error is a configuration error.
        let path = match args.identity.clone() {
            Some(path) => Some(path),
            None => match podnet::state::find_existing("warp.json").fault(Fault::Config)? {
                Some(found) => {
                    args.identity = Some(found.clone());
                    Some(found)
                }
                None => podnet::state::candidates(&|k| std::env::var_os(k))
                    .and_then(|c| c.into_iter().next())
                    .map(|(_, dir)| dir.join("warp.json")),
            },
        };
        match path {
            Some(path) if podnet::state::exists(&path).fault(Fault::Config)? => {}
            Some(path) => {
                return Err(podnet::exit::fault(
                    Fault::Config,
                    anyhow::anyhow!(
                        "no WARP identity at {}; run podnet warp register first",
                        path.display()
                    ),
                ))
            }
            None => {
                return Err(podnet::exit::fault(
                    Fault::Config,
                    anyhow::anyhow!(
                        "PODNET_STATE_DIR=none keeps no WARP identity; give --identity PATH"
                    ),
                ))
            }
        }
    }
    let (client, path) = warp_client(&args).await?;
    if let WarpCommand::Nc {
        host,
        port,
        udp,
        idle,
        shutdown,
        ..
    } = &args.command
    {
        return up::nc_session(client, host, *port, *udp, *idle, *shutdown).await;
    }
    match args.command {
        WarpCommand::Register
        | WarpCommand::Status { .. }
        | WarpCommand::License { .. }
        | WarpCommand::Rotate
        | WarpCommand::Delete => account::run(args, client, path).await,
        WarpCommand::Scan { .. } | WarpCommand::Exit { .. } | WarpCommand::Probe { .. } => {
            probe::run(args, client).await
        }
        WarpCommand::Proxy { .. } | WarpCommand::Forward { .. } => {
            front::run(args, client, path).await
        }
        WarpCommand::Tun { .. } => tun::run(args, client).await,
        WarpCommand::Run { .. } => run::run(args, client).await,
        WarpCommand::Up { .. }
        | WarpCommand::Down
        | WarpCommand::Export(_)
        | WarpCommand::Env
        | WarpCommand::Exec { .. }
        | WarpCommand::Shell(_)
        | WarpCommand::SshConfig { .. }
        | WarpCommand::Nc { .. } => unreachable!("handled before this function"),
    }
}
