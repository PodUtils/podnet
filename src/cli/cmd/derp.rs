//! `podnet derp`: the DERP road (T-128).

use std::path::PathBuf;

use anyhow::{anyhow, Result};
use clap::{Args, Subcommand};

use podnet::derp::{self, Keys, Road, Ticket};
use podnet::exit::{self, Fault, ResultExt};
use podnet::identity::AllowList;

use crate::cli::opts::DetachArgs;
use crate::cli::signals::sleep_or_signal;

#[derive(Args, Debug)]
#[command(subcommand_required = true, arg_required_else_help = false)]
pub(crate) struct DerpArgs {
    #[command(subcommand)]
    pub(crate) command: DerpCommand,
}

/// The servers of the road.
#[derive(Args, Debug, Clone)]
pub(crate) struct ServerArgs {
    /// The DERP servers, in order: HOST[:PORT][,HOST[:PORT]...] (port 443
    /// by default), for example a derper of your own. Default: the fleet of
    /// the data of pod-services: Tailscale's four servers for tailcat, then
    /// its main fleet.
    #[arg(long = "server", value_name = "LIST")]
    pub(crate) server: Option<String>,
    /// With no --server: yes (the default) or no. no keeps the four servers
    /// for tailcat alone.
    #[arg(long, value_name = "ANSWER", value_parser = ["yes", "no"], conflicts_with = "server")]
    pub(crate) main_fleet: Option<String>,
}

impl ServerArgs {
    fn servers(&self) -> Result<Vec<derp::Server>> {
        derp::servers_of(self.server.as_deref(), self.main_fleet.as_deref())
    }
}

#[derive(Subcommand, Debug)]
pub(crate) enum DerpCommand {
    /// Serve a local service at a DERP server: print the ticket (derp:...)
    /// of this node, and connect each session of a client that the allow
    /// list admits to --to. The node moves to the next server when its
    /// server goes away, and prints its new ticket.
    Node {
        /// The local service: host:port or unix:/path.
        #[arg(long, value_name = "TARGET")]
        to: String,
        #[command(flatten)]
        servers: ServerArgs,
        /// The fingerprint of a client that may connect (the line that
        /// `podnet link status` prints on the client). Repeat it for more
        /// than one.
        #[arg(long, value_name = "FINGERPRINT", conflicts_with = "allow")]
        peer: Vec<String>,
        /// An allow list file of the client keys (the form of `link listen
        /// --allow`). Give --peer or --allow: the list is default deny. A
        /// client that it does not admit exits 77, and the node does not
        /// dial the target for it.
        #[arg(long, value_name = "FILE")]
        allow: Option<PathBuf>,
        /// Print the ready document as one JSON line on stdout when the node
        /// is online: ticket, server, fingerprint and target. --detach prints
        /// the same document.
        #[arg(long)]
        json: bool,
        /// Stop after this many seconds (a planned end: no restart).
        #[arg(long, value_name = "SECONDS")]
        run_for: Option<u64>,
        #[command(flatten)]
        detach: DetachArgs,
    },
    /// Open a session to the node of a ticket and copy stdin and stdout (an
    /// SSH ProxyCommand). The node must prove the key of the ticket, and
    /// admit the key of this install (else exit 77). The session goes on
    /// after the loss of a link, for 10 minutes.
    Connect {
        /// The ticket that `podnet derp node` printed (derp:...).
        #[arg(value_name = "TICKET")]
        ticket: String,
        #[command(flatten)]
        servers: ServerArgs,
    },
}

/// The options of --detach of `derp node`.
pub(crate) fn detach_of(args: &DerpArgs) -> Option<&DetachArgs> {
    match &args.command {
        DerpCommand::Node { detach, .. } => Some(detach),
        DerpCommand::Connect { .. } => None,
    }
}

/// The client keys of `derp node`: --allow FILE or --peer. Exit 64 when
/// there is none.
fn clients(allow: Option<&std::path::Path>, peer: &[String]) -> Result<AllowList> {
    match allow {
        Some(path) => AllowList::read(path),
        None if peer.is_empty() => Err(exit::fault(
            Fault::Usage,
            anyhow!(
                "give --peer FINGERPRINT or --allow FILE: the node admits no client by default"
            ),
        )),
        None => AllowList::of_peers(peer),
    }
}

/// The key of this install: the key of `podnet link`, made when there is
/// none.
fn install_key() -> Result<podnet::identity::Identity> {
    podnet::identity::create(&podnet::identity::default_dir()?)
}

/// The ready document of a node.
fn ready(ticket: &Ticket, target: &str) -> serde_json::Value {
    serde_json::json!({
        "ticket": ticket.to_string(),
        "server": ticket.server.to_string(),
        "fingerprint": ticket.node.display_fingerprint(),
        "target": target,
    })
}

/// `podnet derp`.
pub(crate) async fn derp(args: DerpArgs) -> Result<()> {
    match args.command {
        DerpCommand::Node {
            to,
            servers,
            peer,
            allow,
            json,
            run_for,
            ..
        } => {
            // The usage checks come before the key file and the network.
            let target = podnet::relay_reverse::Target::parse(&to)
                .map_err(|e| anyhow!("--to {e}"))
                .fault(Fault::Usage)?;
            let list = servers.servers()?;
            let admitted = clients(allow.as_deref(), &peer)?;
            let me = install_key()?;
            let keys = Keys::new(me.key(), move |k| admitted.admits(k));
            let road = Road::out(list);
            let online = derp::online(me.key(), &road, derp::probe()).await?;
            let ticket = online.ticket.clone();
            let doc = ready(&ticket, &target.to_string());
            if json {
                println!("{}", podnet::status::stamped(doc.clone()));
            } else {
                println!(
                    "podnet: derp     node {} at {}",
                    me.fingerprint(),
                    ticket.server
                );
                println!("podnet: derp     ticket {ticket}");
                println!("On the client: podnet derp connect {ticket}");
            }
            podnet::detach::ready(doc);
            let moved = move |ticket: &Ticket| {
                println!(
                    "podnet: derp     moved to {}; ticket {ticket}",
                    ticket.server
                );
            };
            tokio::select! {
                _ = derp::serve(online, road, target, keys, moved) => Ok(()),
                _ = sleep_or_signal(run_for) => Ok(()),
            }
        }
        DerpCommand::Connect { ticket, servers } => {
            let ticket = derp::ticket_of(&ticket)?;
            let list = servers.servers()?;
            let me = install_key()?;
            let road = Road::out(list);
            let app = tokio::io::join(
                tokio::io::stdin(),
                podnet::relay_reverse::FlushEach::new(tokio::io::stdout()),
            );
            derp::connect(&ticket, &road, app, me.key()).await?;
            // stdin of tokio blocks the shutdown of the runtime.
            std::process::exit(0);
        }
    }
}
