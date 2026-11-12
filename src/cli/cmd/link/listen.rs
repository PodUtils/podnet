//! `podnet link listen`.

use std::collections::BTreeMap;
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
use podnet::link_ctl::{Hub, Slot};
use podnet::link_service as service;
use podnet::{carriers, hello, link};

use super::CarrierArgs;
use crate::cli::opts::DetachArgs;
use crate::cli::signals::shutdown_signal;

/// The most links that a listener serves at one time.
const MAX_LINKS: usize = 64;

/// The most services of one listener.
const MAX_SERVICES: usize = 16;

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
    /// Where the streams of the service `raw` go: `host:port` or
    /// `unix:/path`. It is the same as `--service raw=TARGET`. Without a
    /// service, the listener refuses each stream (the opener exits 69).
    #[arg(long, value_name = "TARGET")]
    pub(crate) to: Option<String>,
    /// A service of this listener: NAME=TARGET, with a NAME of 1 to 32
    /// characters from A-Z a-z 0-9 . _ - and a TARGET as in --to. A peer
    /// reaches it with `link dial --service NAME` or `peer:FINGERPRINT/NAME`.
    /// Repeat it: 16 services at most.
    #[arg(long, value_name = "NAME=TARGET")]
    pub(crate) service: Vec<String>,
    /// The control socket (mode 0600, with the token file PATH.token). The
    /// default is `link-PID.ctl` in the run directory of podnet. The ready
    /// document names it. `podnet link status` reads it.
    #[arg(long, value_name = "PATH")]
    pub(crate) control: Option<PathBuf>,
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

/// The services of a listener: a name and the target of its streams.
type Services = Arc<BTreeMap<String, Target>>;

/// The services of --to and --service. A bad name, a bad target, a
/// duplicate name or more than 16 services exits 64.
fn services_of(args: &ListenArgs) -> Result<BTreeMap<String, Target>> {
    let usage = |text: String| exit::fault(Fault::Usage, anyhow!(text));
    let target =
        |text: &str, flag: &str| Target::parse(text).map_err(|e| usage(format!("{flag} {e}")));
    let mut out = BTreeMap::new();
    if let Some(text) = &args.to {
        out.insert(service::RAW.to_string(), target(text, "--to")?);
    }
    for item in &args.service {
        let (name, text) = item
            .split_once('=')
            .ok_or_else(|| usage(format!("--service {item:?}: give NAME=TARGET")))?;
        service::check_name(name)?;
        let found = target(text, "--service")?;
        if out.insert(name.to_string(), found).is_some() {
            return Err(usage(format!(
                "--service {name}: the service {name:?} has two targets (--to is the service raw)"
            )));
        }
    }
    if out.len() > MAX_SERVICES {
        return Err(usage(format!(
            "a listener has {MAX_SERVICES} services at most"
        )));
    }
    Ok(out)
}

pub(crate) async fn listen(args: ListenArgs) -> Result<()> {
    let options = args.carrier.options();
    let carrier = carriers::open(&args.carrier.carrier, &options)?;
    let services: Services = Arc::new(services_of(&args)?);
    let allow = AllowList::read(&args.allow)?;
    let me = Arc::new(identity::create(&identity::default_dir()?)?);
    let mut door = link::listen(&*carrier, &args.address).await?;
    let address = door.address();
    let mode = mode_of(&args.carrier.carrier, &address);
    let hub = Hub::new(&me.fingerprint());
    let control = super::start_control(&hub, args.control.as_deref())?;
    let doc = serde_json::json!({
        "carrier": args.carrier.carrier,
        "address": address,
        "listen": address,
        "mode": mode,
        "fingerprint": me.fingerprint(),
        "allow": allow.len(),
        "to": args.to,
        "services": services.iter().map(|(n, t)| (n.clone(), t.to_string())).collect::<BTreeMap<_, _>>(),
        "control": control.as_ref().map(|c| c.path().display().to_string()),
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
        if let Some(control) = &control {
            println!(
                "podnet: control socket  {} (mode 0600)",
                control.path().display()
            );
        }
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
                let held = Held {
                    me: me.clone(),
                    allow: allow.clone(),
                    services: services.clone(),
                    hub: hub.clone(),
                    carrier: args.carrier.carrier.clone(),
                    address: address.clone(),
                };
                tokio::spawn(async move {
                    serve(link, held).await;
                    drop(slot);
                });
            }
            Err(err) => tracing::warn!(error = %format!("{err:#}"), "a peer could not connect"),
        }
    }
}

/// What the task of one link needs.
struct Held {
    me: Arc<Identity>,
    allow: AllowList,
    services: Services,
    hub: Arc<Hub>,
    carrier: String,
    address: String,
}

/// One link: the hello, then the streams.
async fn serve(link: Link, held: Held) {
    let link = Arc::new(link);
    let peer = match hello::accept(&link, &held.me, held.allow.clone()).await {
        Ok(peer) => peer,
        Err(err) => {
            eprintln!("podnet: peer refused: {err:#}");
            held.hub.note_error(&format!("peer refused: {err:#}"));
            // The close sends what waits (the verdict of the hello) first.
            link.close().await;
            return;
        }
    };
    let shown = peer.display_fingerprint();
    println!("podnet: peer admitted   {shown}");
    let slot: Slot = held
        .hub
        .hold(link.clone(), peer, &held.carrier, &held.address, "listener");
    while let Ok(stream) = link.accept().await {
        slot.count_accepted();
        let (services, shown) = (held.services.clone(), shown.clone());
        // `close-stream` of the control socket names the stream by its id.
        let tracked = slot.track(&stream);
        let hub = held.hub.clone();
        tokio::spawn(async move {
            let mut stream = stream;
            tokio::select! {
                done = bridge(&mut stream, &services) => {
                    if let Err(err) = done {
                        hub.note_error(&format!("a stream of {shown} ended: {err:#}"));
                        tracing::warn!(peer = shown, error = %format!("{err:#}"), "a stream ended");
                    }
                }
                _ = tracked.cancelled() => stream.reset(link::ResetCode::CANCEL),
            }
        });
    }
    println!("podnet: peer gone       {shown}");
}

/// Read the service of a stream, connect to its target, and copy the
/// stream to the connection and back.
async fn bridge(stream: &mut link::Stream, services: &BTreeMap<String, Target>) -> Result<()> {
    let name = service::accept(stream).await?;
    let Some(target) = services.get(&name) else {
        service::reply(stream, service::NO_SERVICE).await?;
        return Err(anyhow!(
            "the peer asked for the service {name:?}, which this listener does not have"
        ));
    };
    let mut io = match connect(target).await {
        Ok(io) => io,
        Err(err) => {
            service::reply(stream, service::NO_ORIGIN).await?;
            return Err(err);
        }
    };
    service::reply(stream, service::OPEN).await?;
    tokio::io::copy_bidirectional(stream, &mut io).await?;
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
