//! `podnet relay`: the reverse relay.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Args, Subcommand};

use podnet::exit::{Fault, ResultExt};

use crate::cli::opts::DetachArgs;
use crate::cli::signals::shutdown_signal;

#[derive(Args, Debug)]
#[command(subcommand_required = true, arg_required_else_help = false)]
pub(crate) struct RelayArgs {
    /// The relay for a new pair. A pair keeps the relay that made it.
    #[arg(long, value_name = "URL", global = true, env = "PODNET_RELAY", hide_env_values = true, default_value = podnet::ws_relay::default_relay())]
    relay: String,
    #[command(subcommand)]
    pub(crate) command: RelayCommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum RelayCommand {
    /// Ask the relay for a pair: a name, a node token (kept in a file of
    /// mode 0600 in the state directory) and a connect token for the
    /// operator. A pair lasts at most 72 h.
    Pair {
        /// Print the pair as JSON (with the connect token, not the node
        /// token).
        #[arg(long)]
        json: bool,
    },
    /// Hold the node socket of a pair, and connect each session of an
    /// operator to a local service. A lost socket is opened again.
    Node {
        /// The pair name. Default: the pair with the most time left.
        pair: Option<String>,
        /// The local service: host:port or unix:/path.
        #[arg(long, value_name = "TARGET")]
        to: String,
        /// Run the resumable layer for each session: a session of relay
        /// connect --resume continues past the limits of the relay and after
        /// a lost leg. An operator with no --resume gets an error.
        #[arg(long)]
        resume: bool,
        /// Seal each session end to end (needs --resume and --allow): the
        /// operator and the node run the handshake of `pod-e2e` with the key
        /// of each install (the key of `podnet link`), and the relay sees
        /// sealed messages only. The node dials the target only for an
        /// operator whose key is in --allow (else the operator exits 77).
        /// The operator must use --seal too.
        #[arg(long, requires_all = ["resume", "allow"])]
        seal: bool,
        /// With --seal: the allow list file of the operator keys (the form
        /// of `link listen --allow`).
        #[arg(long, value_name = "FILE", requires = "seal")]
        allow: Option<PathBuf>,
        #[command(flatten)]
        detach: DetachArgs,
    },
    /// Open a session to a node and copy stdin and stdout (an SSH
    /// ProxyCommand), or each connection of --listen.
    Connect {
        /// The pair name.
        #[arg(value_name = "NAME")]
        pair: String,
        /// The connect token. Prefer the variable or --token-file: other
        /// users of the host can read a command line. Default: the local
        /// pair file.
        #[arg(long, env = "PODNET_RELAY_CONNECT_TOKEN", hide_env_values = true)]
        token: Option<String>,
        /// Read the connect token from this file.
        #[arg(long, value_name = "PATH")]
        token_file: Option<PathBuf>,
        /// Accept local connections here (tcp://[bind:]port or unix:/path),
        /// one session for each, in place of stdin and stdout.
        #[arg(long, value_name = "LISTEN")]
        listen: Option<String>,
        /// Run the resumable layer (the node needs --resume): the session
        /// moves to a new leg before the limits of the relay (48 MiB or
        /// 11 h; -v prints each move), and resumes after a lost leg. With a
        /// node with no --resume, the session runs in the plain mode.
        #[arg(long)]
        resume: bool,
        /// Seal the session end to end (needs --resume, and --peer or
        /// --allow): the node must prove a key of the list (else exit 77),
        /// and it must let in the key of this install (else exit 77). The
        /// relay sees sealed messages only. The node must use --seal too.
        #[arg(long, requires = "resume")]
        seal: bool,
        /// With --seal: the fingerprint that the node must prove (`podnet
        /// link status` on the node prints it). Repeat it for more than one.
        #[arg(
            long,
            value_name = "FINGERPRINT",
            requires = "seal",
            conflicts_with = "allow"
        )]
        peer: Vec<String>,
        /// With --seal: an allow list file of the node keys that this
        /// install trusts (the form of `link listen --allow`).
        #[arg(long, value_name = "FILE", requires = "seal")]
        allow: Option<PathBuf>,
        /// With --listen: --detach, --name, --replace, --no-restart and
        /// --ready-timeout, as `connect -L`. They have no effect without
        /// --listen.
        #[command(flatten)]
        detach: DetachArgs,
    },
    /// Stop a node and its sessions at the relay (POST /v1/stop), and
    /// remove the local pair file.
    Stop {
        /// The pair name.
        name: String,
    },
}

/// `podnet relay` (issue 88).
pub(crate) async fn relay(args: RelayArgs) -> Result<()> {
    use podnet::relay_reverse as reverse;
    let relay = podnet::ws_relay::Relay::new(&args.relay, None).fault(Fault::Usage)?;
    let dir = reverse::pairs_dir()?;
    match args.command {
        RelayCommand::Pair { json } => {
            let pair = reverse::pair(&relay).await.fault(Fault::Unavailable)?;
            podnet::report::hide(pair.node_token());
            podnet::report::hide(pair.stop_token());
            let path = reverse::save(&pair, &dir)?;
            let left = pair.left().as_secs();
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "name": pair.name,
                        "connect_token": pair.connect_token(),
                        "expires": pair.expires,
                        "relay": pair.relay,
                        "file": path,
                    })
                );
            } else {
                println!("name        {}", pair.name);
                println!("expires     in {} h {} min", left / 3600, left % 3600 / 60);
                println!("file        {} (node token, mode 0600)", path.display());
                println!("connect     {}", pair.connect_token());
                println!();
                println!(
                    "On this host: podnet relay node {} --to 127.0.0.1:22 --detach",
                    pair.name
                );
                println!("Give the connect token to the operator over a channel that you trust. The operator:");
                println!(
                    "  {}=<connect token> ssh -o ProxyCommand='podnet relay connect {}' USER@{}",
                    podnet::relay_reverse::CONNECT_TOKEN_VARIABLE,
                    pair.name,
                    pair.name
                );
            }
            Ok(())
        }
        RelayCommand::Node {
            pair,
            to,
            resume,
            seal,
            allow,
            ..
        } => {
            let target = reverse::Target::parse(&to)
                .map_err(|e| anyhow::anyhow!("--to {e}"))
                .fault(Fault::Usage)?;
            // With no pair name, the node takes the next stored pair at
            // the expiry.
            let follow = pair.is_none().then_some(dir.as_path());
            let pair = reverse::load(&dir, pair.as_deref())?;
            podnet::report::hide(pair.node_token());
            podnet::ws_relay::Relay::single(&pair.relay, None).fault(Fault::Config)?;
            if seal {
                let allow = podnet::identity::AllowList::read(
                    allow.as_deref().context("--seal needs --allow FILE")?,
                )?;
                let seal = reverse::Seal::new(install_key()?.key(), move |k| allow.admits(k));
                tokio::select! {
                    result = reverse::run_node_sealed(follow, pair, &target, &seal) => result,
                    _ = shutdown_signal() => Ok(()),
                }
            } else if resume {
                tokio::select! {
                    result = reverse::run_node_resumable(follow, pair, &target) => result,
                    _ = shutdown_signal() => Ok(()),
                }
            } else {
                tokio::select! {
                    result = reverse::run_node(follow, pair, &target) => result,
                    _ = shutdown_signal() => Ok(()),
                }
            }
        }
        RelayCommand::Connect {
            pair: name,
            token,
            listen,
            resume,
            seal,
            peer,
            allow,
            detach,
            ..
        } => {
            let (token, relay, expires) = match token {
                Some(token) => (token, relay, None),
                None => {
                    let pair = reverse::load(&dir, Some(&name))
                        .context("no --token and no local pair file")?;
                    let relay =
                        podnet::ws_relay::Relay::new(&pair.relay, None).fault(Fault::Config)?;
                    (pair.connect_token().to_string(), relay, Some(pair.expires))
                }
            };
            podnet::report::hide(&token);
            // T-087: the layer of --resume, else the plain mode.
            let layer = resume.then_some(reverse::Resume {
                expires,
                ..reverse::Resume::default()
            });
            let seal = match seal {
                false => None,
                true => {
                    // The usage check comes before the key file.
                    let nodes = trusted_nodes(allow.as_deref(), &peer)?;
                    Some(Arc::new(reverse::Seal::new(
                        install_key()?.key(),
                        move |k| nodes.admits(k),
                    )))
                }
            };
            match listen {
                None => {
                    session(
                        &relay,
                        &name,
                        &token,
                        tokio::io::stdin(),
                        tokio::io::stdout(),
                        layer,
                        seal.as_deref(),
                    )
                    .await?;
                    // stdin of tokio blocks the shutdown of the runtime.
                    std::process::exit(0);
                }
                Some(listen) => {
                    let listen =
                        podnet::vnet::proxy::ProxyListen::parse(&listen).fault(Fault::Usage)?;
                    let relay = Arc::new(relay);
                    let (name, token) = (Arc::new(name), Arc::new(token));
                    let serve = |io: Box<dyn RelayIo>| {
                        let (relay, name, token) = (relay.clone(), name.clone(), token.clone());
                        let seal = seal.clone();
                        tokio::spawn(async move {
                            let (read, write) = tokio::io::split(io);
                            if let Err(err) =
                                session(&relay, &name, &token, read, write, layer, seal.as_deref())
                                    .await
                            {
                                tracing::warn!(error = %format!("{err:#}"), "a relay session failed");
                            }
                        });
                    };
                    // An accept error ends the command with exit 69: a
                    // supervisor (--detach) starts it again.
                    let accepted = |err: std::io::Error| {
                        tracing::warn!(error = %err, "accept failed");
                        Err(anyhow::anyhow!("accept failed: {err}")).fault(Fault::Unavailable)
                    };
                    match listen {
                        podnet::vnet::proxy::ProxyListen::Tcp(addr) => {
                            let listener = podnet::net::bind_tcp(addr).await?;
                            let local = format!("tcp://{}", listener.local_addr()?);
                            println!("podnet: relay    {name} -> {local}");
                            podnet::detach::ready(
                                serde_json::json!({ "pair": *name, "listen": local }),
                            );
                            podnet::detach::arm_stop();
                            loop {
                                tokio::select! {
                                    got = listener.accept() => match got {
                                        Ok((socket, _)) => serve(Box::new(socket)),
                                        Err(err) => return accepted(err),
                                    },
                                    _ = shutdown_signal() => return Ok(()),
                                }
                            }
                        }
                        podnet::vnet::proxy::ProxyListen::Unix(path) => {
                            // The file of this run goes at the end.
                            let (listener, _file) =
                                super::connect::unix_listener(&path, detach.replace)?;
                            let local = format!("unix:{}", path.display());
                            println!("podnet: relay    {name} -> {local}");
                            podnet::detach::ready(
                                serde_json::json!({ "pair": *name, "listen": local }),
                            );
                            podnet::detach::arm_stop();
                            loop {
                                tokio::select! {
                                    got = listener.accept() => match got {
                                        Ok((socket, _)) => serve(Box::new(socket)),
                                        Err(err) => return accepted(err),
                                    },
                                    _ = shutdown_signal() => return Ok(()),
                                }
                            }
                        }
                    }
                }
            }
        }
        RelayCommand::Stop { name } => {
            let pair = reverse::load(&dir, Some(&name))?;
            // Check the pair before the first local stop: a stop that the
            // relay cannot get must not leave a node that is stopped.
            podnet::ws_relay::Relay::single(&pair.relay, None).fault(Fault::Config)?;
            if pair.stop_token().is_empty() {
                return Err(anyhow::anyhow!("the pair {name} has no stop token"))
                    .fault(Fault::Unavailable);
            }
            // Issue 119: the local node of the pair stops first, so that it
            // does not connect again.
            if let Ok(run) = podnet::instances::run_dir() {
                let records: Vec<_> = podnet::instances::list(&run)
                    .into_iter()
                    .filter(|e| e.alive)
                    .map(|e| e.record)
                    .collect();
                for instance in reverse::instances_of_pair(&records, &name) {
                    podnet::detach::stop(&run, &instance, Duration::from_secs(10))?;
                    println!("podnet: relay    stopped the node instance {instance}");
                }
            }
            let file = reverse::file_of(&dir, &name)?;
            reverse::stop(&pair)
                .await
                .with_context(|| {
                    format!(
                        "the relay side of {name} is live for {} more; the pair file {} stays",
                        reverse::left_words(pair.left()),
                        file.display()
                    )
                })
                .fault(Fault::Unavailable)?;
            let _ = reverse::remove(&dir, &name);
            println!("podnet: relay    {name} stopped");
            Ok(())
        }
    }
}

/// The key of this install: the key of `podnet link`, made when there is
/// none.
fn install_key() -> Result<podnet::identity::Identity> {
    podnet::identity::create(&podnet::identity::default_dir()?)
}

/// The node keys of `relay connect --seal`: --allow FILE or --peer. Exit 64
/// when there is none.
fn trusted_nodes(
    allow: Option<&std::path::Path>,
    peer: &[String],
) -> Result<podnet::identity::AllowList> {
    match allow {
        Some(path) => podnet::identity::AllowList::read(path),
        None if peer.is_empty() => Err(anyhow::anyhow!(
            "--seal needs --peer FINGERPRINT or --allow FILE: this end trusts no node by default"
        ))
        .fault(Fault::Usage),
        None => {
            podnet::identity::AllowList::parse(&peer.join("\n")).map_err(|e| e.context("--peer"))
        }
    }
}

/// One session of an operator: with the layer when `layer` is set (and the
/// end-to-end channel above it with `seal`), else in the plain mode.
async fn session<R, W>(
    relay: &podnet::ws_relay::Relay,
    name: &str,
    token: &str,
    input: R,
    output: W,
    layer: Option<podnet::relay_reverse::Resume>,
    seal: Option<&podnet::relay_reverse::Seal>,
) -> Result<()>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    use podnet::relay_reverse as reverse;
    match (layer, seal) {
        (Some(layer), Some(seal)) => {
            reverse::connect_sealed(relay, name, token, input, output, layer, seal).await
        }
        (Some(layer), None) => {
            reverse::connect_resumable(relay, name, token, input, output, layer).await
        }
        (None, _) => reverse::connect(relay, name, token, input, output).await,
    }
}

trait RelayIo: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send {}
impl<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send> RelayIo for T {}
