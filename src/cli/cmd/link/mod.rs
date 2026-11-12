//! `podnet link`: a link between two installs over a carrier (T-265).

mod connect;
mod dial;
mod info;
mod listen;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use clap::{Args, Subcommand};

use podnet::carriers::Options;
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
    /// The carrier (`podnet link carriers` lists them).
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
    /// against --peer or --allow, and hold the link. Other programs open
    /// streams of the link through its control socket: `peer:FINGERPRINT/
    /// SERVICE` as a network (podnet forward -L LISTEN=peer:FINGERPRINT/
    /// SERVICE). The command ends when the link ends (exit 69) or at a stop
    /// signal.
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
    /// Stop a link instance that runs in the background (SIGTERM, then
    /// SIGKILL after --timeout). The listener removes its unix socket.
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
pub(crate) async fn link(args: LinkArgs) -> Result<()> {
    match args.command {
        LinkCommand::Listen(args) => listen::listen(args).await,
        LinkCommand::Dial(args) => dial::dial(args).await,
        LinkCommand::Connect(args) => connect::connect(args).await,
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
        LinkCommand::Close { name, timeout } => info::close(&name, timeout),
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
