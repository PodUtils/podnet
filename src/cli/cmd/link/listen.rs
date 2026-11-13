//! `podnet link listen`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use clap::Args;
use tokio::sync::Semaphore;

use pod_identity::PublicKey;
use podnet::hello;
use podnet::identity::{self, AllowList, Grant, Identity};
use podnet::link::{Link, Settings};
use podnet::link_announce::{self as announce, Guard, Mesh};
use podnet::link_ctl::{Hub, Slot};
use podnet::mesh_announce::INTERVAL_MS;

use super::services::{services_of, ServiceArgs, Services, Streams};
use super::CarrierArgs;
use crate::cli::opts::DetachArgs;
use crate::cli::signals::shutdown_signal;

/// The most links that a listener serves at one time.
const MAX_LINKS: usize = 64;

#[derive(Args, Debug)]
pub(crate) struct ListenArgs {
    /// The address, in the form of the carrier: a path for `unix`, a name
    /// for `memory`, `IP:PORT` for `tcp`.
    /// A query can follow the address: `ADDRESS?key=value&key=value` (keys:
    /// carrier, tls, retry, lb, compress, pad, mtu, maxhops, poll, window,
    /// name). Only `carrier` has an effect yet; a key with no effect, a repeated
    /// key, an unknown key or a secret key (password, key, token, identity)
    /// exits 64, and no message shows a value.
    #[arg(value_name = "ADDRESS")]
    pub(crate) address: String,
    #[command(flatten)]
    pub(crate) carrier: CarrierArgs,
    /// The allow list: a file with one peer on each line (the fingerprint
    /// that `podnet link status` shows, a `SHA256:` fingerprint, or a key in
    /// hex), 4096 lines at most. The list is default deny: a peer that it
    /// does not hold gets exit 77. The file must not be writable by others
    /// (else exit 71); a missing file exits 66. A line can end in a grant:
    /// `services=NAME,NAME` (64 names at most, or `*`) and `forward=yes`.
    /// A bare key has every service and forwarding. With an option, what
    /// it does not name is denied; a refused stream is counted in `link
    /// status` and the link stays up.
    #[arg(long, value_name = "FILE", required = true)]
    pub(crate) allow: PathBuf,
    /// Where the streams of the service `raw` go: `host:port` or
    /// `unix:/path`. It is the same as `--service raw=TARGET`. The bytes
    /// pass with no framing, and half-close goes both ways. Without a
    /// service, the listener refuses each stream (the opener exits 69).
    #[arg(long, value_name = "TARGET")]
    pub(crate) to: Option<String>,
    #[command(flatten)]
    pub(crate) services: ServiceArgs,
    /// The control socket (mode 0600, with the token file PATH.token). The
    /// default is `link-PID.ctl` in the run directory of podnet. The ready
    /// document names it. `podnet link status` reads it.
    #[arg(long, value_name = "PATH")]
    pub(crate) control: Option<PathBuf>,
    /// Print the ready document as one line of JSON in place of the text.
    #[arg(long)]
    pub(crate) json: bool,
    /// Send and take signed route announcements on each link (the service
    /// `announce`, each 10 s): this end tells its peer its neighbours and
    /// its services, and learns the routes of the peer. The control socket
    /// shows the routes with the verb `routes`. The name `announce` cannot
    /// be a --service with this flag. The peer must run it too. A node
    /// forwards the announcements of others, and a stream that a peer sends
    /// for another node goes to the next hop of the best route (`link dial
    /// --node`). The name `fwd` cannot be a --service with this flag.
    #[arg(long)]
    pub(crate) announce: bool,
    /// With --announce: the cost of each link, 1 to 4294967295. Without it,
    /// the cost is the RTT of the link in ms (10 until it is known).
    #[arg(long, value_name = "COST", requires = "announce", value_parser = clap::value_parser!(u32).range(1..))]
    pub(crate) announce_weight: Option<u32>,
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

pub(crate) async fn listen(mut args: ListenArgs) -> Result<()> {
    args.address = super::apply_query(&args.address, &mut args.carrier)?;
    let carrier = super::open_carrier(&args.carrier)?;
    let services: Services = Arc::new(services_of(
        args.to.as_deref(),
        &args.services,
        args.announce,
    )?);
    let allow = AllowList::read(&args.allow)?;
    let me = Arc::new(identity::create(&identity::default_dir()?)?);
    let mut door = super::listen_door(carrier, &args.address, &me, &allow).await?;
    let address = door.address();
    let mode = mode_of(&args.carrier.carrier, &address);
    let hub = Hub::new(&me.fingerprint());
    let mesh = args.announce.then(|| {
        let names = services.keys().cloned().collect();
        let mesh = Mesh::new(me.clone(), names, announce::now_ms());
        hub.attach_mesh(mesh.clone());
        mesh
    });
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
            accepted = door.accept_peer(Settings::default()) => accepted,
        };
        match accepted {
            Ok((link, proved)) => {
                let held = Held {
                    me: me.clone(),
                    allow: allow.clone(),
                    services: services.clone(),
                    hub: hub.clone(),
                    carrier: args.carrier.carrier.clone(),
                    address: address.clone(),
                    mesh: mesh.clone(),
                    weight: args.announce_weight,
                };
                tokio::spawn(async move {
                    serve(link, proved, held).await;
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
    mesh: Option<Arc<Mesh>>,
    weight: Option<u32>,
}

/// One link: the hello, then the streams. A carrier that proved the key of
/// the peer in its own handshake (`tcp`) needs no hello.
async fn serve(link: Link, proved: Option<PublicKey>, held: Held) {
    let link = Arc::new(link);
    let admitted = match proved {
        Some(key) => held.allow.check(&key).map(|()| key),
        None => hello::accept(&link, &held.me, held.allow.clone()).await,
    };
    let peer = match admitted {
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
    // The guard stops the announcements and ends the route of the peer.
    let grant = held.allow.grant(&peer).unwrap_or_else(Grant::closed);
    let _guard = held
        .mesh
        .as_ref()
        .map(|mesh| Guard::start(mesh, &link, peer, held.weight, INTERVAL_MS));
    let streams = Streams {
        hub: held.hub.clone(),
        services: held.services.clone(),
        grant,
        peer,
        shown: shown.clone(),
        mesh: held
            .mesh
            .clone()
            .map(|mesh| (mesh, link.clone(), held.weight)),
    };
    while let Ok(stream) = link.accept().await {
        streams.spawn(stream, &slot);
    }
    println!("podnet: peer gone       {shown}");
}
