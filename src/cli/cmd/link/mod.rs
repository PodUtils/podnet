//! `podnet link`: a link between two installs over a carrier (T-265).

mod connect;
mod dial;
mod info;
pub(crate) mod listen;
mod services;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use clap::{Args, Subcommand};

use pod_identity::PublicKey;
use pod_link::Carrier;
use podnet::carriers::{self, Options};
use podnet::hello;
use podnet::identity::{AllowList, Identity};
use podnet::link::{Door, Link, Settings};
use podnet::link_ctl::{Control, Hub};

use crate::cli::opts::DetachArgs;

#[derive(Args, Debug)]
#[command(subcommand_required = true, arg_required_else_help = false)]
pub(crate) struct LinkArgs {
    #[command(subcommand)]
    pub(crate) command: LinkCommand,
}

/// The flags that choose and set a carrier.
#[derive(Args, Debug, Clone)]
pub(crate) struct CarrierArgs {
    /// The carrier (`podnet link carriers` lists them). `tcp` (build feature
    /// `tcp`) runs TLS 1.3 and pins the Ed25519 key of each end: listen at
    /// `IP:PORT`, dial `KEY@HOST:PORT`. A key that is refused exits 77.
    #[arg(long, value_name = "NAME", default_value = "unix")]
    pub(crate) carrier: String,
    /// The unix carrier accepts a peer of another uid. Each end checks the
    /// uid of the other end (SO_PEERCRED), so give the flag at both ends.
    /// Without it, a peer of another uid is refused (exit 69).
    #[arg(long)]
    pub(crate) allow_other_uid: bool,
    /// The bytes of each ring of the memory carrier: 65536 to 67108864
    /// (64 KiB to 64 MiB). Default 1048576.
    #[arg(long, value_name = "BYTES")]
    pub(crate) ring: Option<usize>,
}

impl CarrierArgs {
    pub(crate) fn options(&self) -> Options {
        let base = Options::default();
        Options {
            allow_other_uid: self.allow_other_uid,
            ring: self.ring.unwrap_or(base.ring),
        }
    }
}

/// The carrier `args` names, or `None` for `tcp`: that carrier needs the key
/// of this install and the allow list, so [`dial_peer`] and [`listen_door`]
/// build it. A name that is unknown exits 64; a carrier that this build
/// leaves out (`tcp` too) exits 69.
pub(super) fn open_carrier(args: &CarrierArgs) -> Result<Option<Box<dyn Carrier>>> {
    if args.carrier == "tcp" {
        podnet::build_features::require("tcp")?;
        return Ok(None);
    }
    Ok(Some(carriers::open(&args.carrier, &args.options())?))
}

/// Split the query off `address` and check it before any socket opens
/// (T-306). The key `carrier` takes the place of --carrier. A key that has
/// no effect yet exits 64. The result is the address with no query.
pub(super) fn apply_query(address: &str, carrier: &mut CarrierArgs) -> Result<String> {
    let (base, query) = podnet::link_query::split(address)?;
    if let Some(name) = &query.carrier {
        carrier.carrier.clone_from(name);
    }
    carriers::check_query(&carrier.carrier, &query)?;
    query.require_wired()?;
    Ok(base.to_string())
}

/// Connect to `address` and prove the peer. A carrier with no key of its own
/// (unix, memory) runs the hello above it. `tcp` proves the key in the TLS
/// handshake, and `address` is `KEY@HOST:PORT`.
pub(super) async fn dial_peer(
    carrier: Option<Box<dyn Carrier>>,
    address: &str,
    me: &Identity,
    allow: AllowList,
    via: Option<Arc<dyn podnet::network::Dial>>,
) -> Result<(Link, PublicKey)> {
    match carrier {
        Some(carrier) => {
            let link = podnet::link::dial(&*carrier, address, Settings::default()).await?;
            let peer = hello::dial(&link, me, allow).await?;
            Ok((link, peer))
        }
        None => tcp::dial(me, allow, address, via).await,
    }
}

/// The chain of the `--egress` elements of `link dial` and `link connect`
/// (T-303), checked and up before the carrier opens. `None` with no element.
/// With `check`, resolve and dial each element once first, and print the
/// result (`json`: the documents are returned in the second value instead).
/// Only the `tcp` carrier opens a network connection: a chain with another
/// carrier exits 64.
pub(super) async fn open_chain(
    egress: &[String],
    carrier: &CarrierArgs,
    address: &str,
    check: bool,
    json: bool,
) -> Result<(Option<podnet::link_chain::Opened>, Vec<serde_json::Value>)> {
    if egress.is_empty() {
        return Ok((None, Vec::new()));
    }
    if carrier.carrier != "tcp" {
        return Err(podnet::exit::fault(
            podnet::exit::Fault::Usage,
            anyhow::anyhow!(
                "--egress: the {} carrier opens no network connection; a chain is for the tcp carrier",
                carrier.carrier
            ),
        ));
    }
    let plan = podnet::link_chain::Plan::parse(egress, address)?;
    let mut docs = Vec::new();
    if check {
        let rows = plan.check().await?;
        for row in &rows {
            if !json {
                println!("{}", row.line(plan.len()));
            }
            docs.push(row.json());
        }
    }
    Ok((Some(plan.open().await?), docs))
}

/// True for the commands that take a chain of `--egress` elements.
pub(crate) fn takes_chain(args: &LinkArgs) -> bool {
    matches!(args.command, LinkCommand::Dial(_) | LinkCommand::Connect(_))
}

/// Listen at `address`. `tcp` takes the clients of `allow` in its handshake.
pub(super) async fn listen_door(
    carrier: Option<Box<dyn Carrier>>,
    address: &str,
    me: &Identity,
    allow: &AllowList,
) -> Result<Door> {
    match carrier {
        Some(carrier) => podnet::link::listen(&*carrier, address).await,
        None => tcp::listen(me, allow, address).await,
    }
}

#[cfg(feature = "tcp")]
mod tcp {
    use super::*;

    pub(super) async fn dial(
        me: &Identity,
        allow: AllowList,
        address: &str,
        via: Option<Arc<dyn podnet::network::Dial>>,
    ) -> Result<(Link, PublicKey)> {
        let carrier = match via {
            Some(via) => podnet::link_tcp::carrier_via(me, &allow, via),
            None => podnet::link_tcp::carrier(me, &allow),
        };
        podnet::link_tcp::dial(&carrier, &allow, address, Settings::default()).await
    }

    pub(super) async fn listen(me: &Identity, allow: &AllowList, address: &str) -> Result<Door> {
        let carrier = podnet::link_tcp::carrier(me, allow);
        podnet::link_tcp::listen(&carrier, address).await
    }
}

/// This build has no `tcp` carrier: `open_carrier` already exited 69.
#[cfg(not(feature = "tcp"))]
mod tcp {
    use super::*;

    pub(super) async fn dial(
        _: &Identity,
        _: AllowList,
        _: &str,
        _: Option<Arc<dyn podnet::network::Dial>>,
    ) -> Result<(Link, PublicKey)> {
        podnet::build_features::require("tcp").map(|()| unreachable!())
    }

    pub(super) async fn listen(_: &Identity, _: &AllowList, _: &str) -> Result<Door> {
        podnet::build_features::require("tcp").map(|()| unreachable!())
    }
}

#[derive(Subcommand, Debug)]
pub(crate) enum LinkCommand {
    /// Listen at an address of a carrier. A peer must prove its key, and the
    /// allow list (--allow) must hold it, before it can open a stream. Print
    /// the address (for the unix carrier: the path and its mode 0600) and
    /// the fingerprint of this install. Each stream of an admitted peer goes
    /// to --to. A second listener at a live address exits 69; a peer that
    /// the list does not hold exits 77 at its side.
    Listen(listen::ListenArgs),
    /// Connect to a listener, prove this key, and check the key of the
    /// listener against --peer or --allow. Then copy stdin and stdout over
    /// one stream. A listener that is not allowed exits 77.
    Dial(dial::DialArgs),
    /// Connect to a listener, prove this key, check the key of the listener
    /// against --peer or --allow, and hold the link. With --check: connect,
    /// check, print and exit 0, and start no control socket. Other programs open
    /// streams of the link through its control socket: `peer:FINGERPRINT/
    /// SERVICE` as a network (podnet forward -L LISTEN=peer:FINGERPRINT/
    /// SERVICE). The command ends when the link ends (exit 69) or at a stop
    /// signal. --name is 1 to 32 characters from a-z 0-9 -, unique among
    /// the live links (a duplicate exits 64), and bound to the fingerprint
    /// of the peer: another key under the same name exits 77.
    Connect(connect::ConnectArgs),
    /// Show the fingerprint of this install and the link instances that run
    /// (podnet ps lists the other instances). With a NAME, show that
    /// instance only. For an instance with a control socket, status asks the
    /// socket for the links it holds (peer, streams, counters) and shows them
    /// under `live`; if the socket does not answer, status shows the record.
    Status {
        /// The name of a link instance (podnet ps). Default: each one.
        name: Option<String>,
        /// Make the key of this install when there is none (mode 0600 in the
        /// state directory). Without it, status writes nothing.
        #[arg(long)]
        create: bool,
        /// Print one JSON document.
        #[arg(long)]
        json: bool,
        /// Ask this control socket only (the `control` of the ready
        /// document of an instance). A wrong token exits 77, and a socket
        /// that does not answer exits 69.
        #[arg(long, value_name = "PATH")]
        control: Option<PathBuf>,
        /// The token file of --control. Default: PATH.token.
        #[arg(long, value_name = "FILE", requires = "control")]
        token_file: Option<PathBuf>,
    },
    /// Show the local view of a mesh: the nodes (fingerprint, name, last
    /// seq, hops, services) and the edges (peer, cost, last probe, state)
    /// that the announcements of a listener with --announce taught it. Ask
    /// the control socket of that listener (mode 0600; there is no network
    /// endpoint). --json prints the document; --metrics prints the counters
    /// as Prometheus text. The view holds 4096 nodes, 64 edges for each
    /// node and 256 KiB; `truncated` counts what was cut. A listener
    /// without --announce exits 69; a wrong token exits 77.
    Mesh {
        /// The control socket of the listener: unix:/PATH.
        #[arg(long, value_name = "unix:PATH")]
        control: String,
        /// The token file of --control. Default: PATH.token.
        #[arg(long, value_name = "FILE")]
        token_file: Option<PathBuf>,
        /// Print one JSON document.
        #[arg(long, conflicts_with = "metrics")]
        json: bool,
        /// Print the counters as Prometheus text.
        #[arg(long)]
        metrics: bool,
        /// Show at most N nodes (a lower limit than 4096).
        #[arg(long, value_name = "N")]
        max_nodes: Option<u64>,
        /// Show at most N edges for each node (a lower limit than 64).
        #[arg(long, value_name = "N")]
        max_edges: Option<u64>,
    },
    /// Stop a link instance that runs in the background. First end each
    /// link of the instance through its control socket (the verb
    /// close-link), so that the peers get a GOAWAY; then SIGTERM, and
    /// SIGKILL after --timeout. The listener removes its unix socket.
    Close {
        /// The name of the link instance (podnet link status).
        name: String,
        /// Seconds to wait before SIGKILL.
        #[arg(long, default_value_t = 10)]
        timeout: u64,
    },
    /// List the carriers of this build, with the flags of each and whether
    /// this host can use it. A carrier that this host cannot use gives its
    /// reason.
    Carriers {
        #[command(flatten)]
        carrier: CarrierArgs,
        /// Print one JSON document: {"carriers": [...]}.
        #[arg(long)]
        json: bool,
    },
}

/// Start the control socket of an instance at `explicit`, else at
/// `link-PID.ctl` in the run directory. A host with no run directory runs
/// with no socket (the ready document says `control: null`); an explicit
/// path that cannot be used exits 69 or 71.
pub(crate) fn start_control(hub: &Arc<Hub>, explicit: Option<&Path>) -> Result<Option<Control>> {
    let path = match explicit {
        Some(path) => path.to_path_buf(),
        None => match podnet::instances::run_dir() {
            Ok(dir) => dir.join(format!("link-{}.ctl", std::process::id())),
            Err(err) => {
                tracing::warn!(error = %format!("{err:#}"), "no control socket: there is no run directory");
                return Ok(None);
            }
        },
    };
    Control::start(&path, hub.clone()).map(Some)
}

/// The options of --detach of `link listen` and `link connect`.
pub(crate) fn detach_of(args: &LinkArgs) -> Option<&DetachArgs> {
    match &args.command {
        LinkCommand::Listen(listen) => Some(&listen.detach),
        LinkCommand::Connect(connect) => Some(&connect.detach),
        _ => None,
    }
}

/// `podnet link`.
pub(crate) async fn link(args: LinkArgs, egress: Vec<String>) -> Result<()> {
    match args.command {
        LinkCommand::Listen(args) => listen::listen(args).await,
        LinkCommand::Dial(args) => dial::dial(args, egress).await,
        LinkCommand::Connect(args) => connect::connect(args, egress).await,
        LinkCommand::Status {
            name,
            json,
            create,
            control,
            token_file,
        } => {
            let ask = info::Ask {
                name: name.as_deref(),
                json,
                create,
                control: control.as_deref(),
                token_file: token_file.as_deref(),
            };
            info::status(ask).await
        }
        LinkCommand::Mesh {
            control,
            token_file,
            json,
            metrics,
            max_nodes,
            max_edges,
        } => {
            let ask = info::MeshAsk {
                control: &control,
                token_file: token_file.as_deref(),
                json,
                metrics,
                max_nodes,
                max_edges,
            };
            info::mesh(ask).await
        }
        LinkCommand::Close { name, timeout } => info::close(&name, timeout).await,
        LinkCommand::Carriers { carrier, json } => info::carriers(&carrier.options(), json),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(allow_other_uid: bool, ring: Option<usize>) -> CarrierArgs {
        CarrierArgs {
            carrier: "unix".into(),
            allow_other_uid,
            ring,
        }
    }

    #[test]
    fn the_flags_reach_the_options_of_the_carriers() {
        let default = Options::default();
        assert!(!args(false, None).options().allow_other_uid);
        assert!(args(true, None).options().allow_other_uid);
        assert_eq!(args(false, None).options().ring, default.ring);
        assert_eq!(args(false, Some(65536)).options().ring, 65536);
    }
}
