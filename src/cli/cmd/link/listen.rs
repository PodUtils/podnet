//! `podnet link listen`.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use clap::Args;
use pod_core::BoxedIo;
use podnet::relay_reverse::Target;
use tokio::sync::Semaphore;

use podnet::exit::{self, Fault};
use podnet::identity::{self, AllowList, Identity};
use podnet::link::{Link, Settings};
use podnet::{carriers, hello, link};

use super::CarrierArgs;
use crate::cli::opts::DetachArgs;
use crate::cli::signals::shutdown_signal;

/// The most links that a listener serves at one time.
const MAX_LINKS: usize = 64;

/// The time that a connection to --to can take.
const CONNECT_LIMIT: Duration = Duration::from_secs(10);

#[derive(Args, Debug)]
pub(crate) struct ListenArgs {
    /// The address, in the form of the carrier: a path for `unix`, a name
    /// for `memory`.
    #[arg(value_name = "ADDRESS")]
    pub(crate) address: String,
    #[command(flatten)]
    pub(crate) carrier: CarrierArgs,
    /// The allow list: a file with one peer on each line (the fingerprint
    /// that `podnet link status` shows, a `SHA256:` fingerprint, or a key in
    /// hex), 4096 lines at most. The list is default deny: a peer that it
    /// does not hold gets exit 77. The file must not be writable by others
    /// (else exit 71); a missing file exits 66.
    #[arg(long, value_name = "FILE", required = true)]
    pub(crate) allow: PathBuf,
    /// Where each stream of an admitted peer goes: `host:port` or
    /// `unix:/path`. Without it, podnet closes each stream at once.
    #[arg(long, value_name = "TARGET")]
    pub(crate) to: Option<String>,
    /// Print the ready document as one line of JSON in place of the text.
    #[arg(long)]
    pub(crate) json: bool,
    #[command(flatten)]
    pub(crate) detach: DetachArgs,
}

/// The mode of the socket file of the unix carrier, as `0600`.
fn mode_of(carrier: &str, address: &str) -> Option<String> {
    use std::os::unix::fs::PermissionsExt as _;
    if carrier != "unix" {
        return None;
    }
    let meta = std::fs::symlink_metadata(address).ok()?;
    Some(format!("{:04o}", meta.permissions().mode() & 0o7777))
}

pub(crate) async fn listen(args: ListenArgs) -> Result<()> {
    let options = args.carrier.options();
    let carrier = carriers::open(&args.carrier.carrier, &options)?;
    let target = match &args.to {
        Some(text) => Some(
            Target::parse(text)
                .map_err(|e| anyhow!("--to {e}"))
                .map_err(|e| exit::fault(Fault::Usage, e))?,
        ),
        None => None,
    };
    let allow = AllowList::read(&args.allow)?;
    let me = Arc::new(identity::create(&identity::default_dir()?)?);
    let mut door = link::listen(&*carrier, &args.address).await?;
    let address = door.address();
    let mode = mode_of(&args.carrier.carrier, &address);
    let doc = serde_json::json!({
        "carrier": args.carrier.carrier,
        "address": address,
        "listen": address,
        "mode": mode,
        "fingerprint": me.fingerprint(),
        "allow": allow.len(),
        "to": target.as_ref().map(|t| t.to_string()),
    });
    // A stop signal after the ready line starts the clean shutdown.
    podnet::detach::arm_stop();
    if args.json {
        println!("{}", podnet::status::stamped(doc.clone()));
    } else {
        let shown = match &mode {
            Some(mode) => format!("{address} (mode {mode})"),
            None => address.clone(),
        };
        println!("podnet: link listening  {} {shown}", args.carrier.carrier);
        println!("podnet: fingerprint     {}", me.fingerprint());
        println!("podnet: allow list      {} entries", allow.len());
    }
    podnet::detach::ready(doc);
    let slots = Arc::new(Semaphore::new(MAX_LINKS));
    loop {
        let Ok(slot) = slots.clone().acquire_owned().await else {
            return Ok(());
        };
        let accepted = tokio::select! {
            _ = shutdown_signal() => return Ok(()),
            accepted = door.accept(Settings::default()) => accepted,
        };
        match accepted {
            Ok(link) => {
                let (me, allow, target) = (me.clone(), allow.clone(), target.clone());
                tokio::spawn(async move {
                    serve(link, me, allow, target).await;
                    drop(slot);
                });
            }
            Err(err) => tracing::warn!(error = %format!("{err:#}"), "a peer could not connect"),
        }
    }
}

/// One link: the hello, then the streams.
async fn serve(link: Link, me: Arc<Identity>, allow: AllowList, target: Option<Target>) {
    let peer = match hello::accept(&link, &me, allow).await {
        Ok(peer) => peer,
        Err(err) => {
            eprintln!("podnet: peer refused: {err:#}");
            // The close sends what waits (the verdict of the hello) first.
            link.close().await;
            return;
        }
    };
    let peer = peer.display_fingerprint();
    println!("podnet: peer admitted   {peer}");
    while let Ok(stream) = link.accept().await {
        let Some(target) = target.clone() else {
            // No service: the stream closes at once.
            drop(stream);
            continue;
        };
        let peer = peer.clone();
        tokio::spawn(async move {
            if let Err(err) = bridge(stream, &target).await {
                tracing::warn!(peer, error = %format!("{err:#}"), "a stream ended");
            }
        });
    }
    println!("podnet: peer gone       {peer}");
}

/// Copy a stream to a connection to `target`, and back.
async fn bridge(mut stream: link::Stream, target: &Target) -> Result<()> {
    let mut io = connect(target).await?;
    tokio::io::copy_bidirectional(&mut stream, &mut io).await?;
    Ok(())
}

/// A connection to `target`. A name resolves with the resolver of this
/// host, and no proxy is used: the service is on this host or its network.
async fn connect(target: &Target) -> Result<BoxedIo> {
    let work = async {
        Ok::<BoxedIo, std::io::Error>(match target {
            Target::Tcp { host, port } => {
                let s = tokio::net::TcpStream::connect((host.as_str(), *port)).await?;
                let _ = s.set_nodelay(true);
                Box::new(s)
            }
            Target::Unix(path) => Box::new(tokio::net::UnixStream::connect(path).await?),
        })
    };
    tokio::time::timeout(CONNECT_LIMIT, work)
        .await
        .map_err(|_| anyhow!("no answer from {target} in {} s", CONNECT_LIMIT.as_secs()))?
        .with_context(|| format!("connecting to {target}"))
}
