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

use pod_identity::PublicKey;
use podnet::exit::{self, Fault};
use podnet::identity::{self, AllowList, Identity};
use podnet::link::{Link, Settings};
use podnet::link_announce::{self as announce, Guard, Mesh};
use podnet::link_ctl::{Hub, Slot};
use podnet::link_service as service;
use podnet::mesh_announce::INTERVAL_MS;
use podnet::{hello, link};

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
    /// (else exit 71); a missing file exits 66.
    #[arg(long, value_name = "FILE", required = true)]
    pub(crate) allow: PathBuf,
    /// Where the streams of the service `raw` go: `host:port` or
    /// `unix:/path`. It is the same as `--service raw=TARGET`. The bytes
    /// pass with no framing, and half-close goes both ways. Without a
    /// service, the listener refuses each stream (the opener exits 69).
    #[arg(long, value_name = "TARGET")]
    pub(crate) to: Option<String>,
    /// A service of this listener: NAME=TARGET, with a NAME of 1 to 32
    /// characters from A-Z a-z 0-9 . _ - and a TARGET as in --to. A peer
    /// reaches it with `link dial --service NAME` or `peer:FINGERPRINT/NAME`.
    /// The service `socks5` takes no target: `--service socks5` makes this
    /// end the server side of SOCKS5 (CONNECT only; a name resolves here).
    /// The service `http` takes no target either: `--service http` makes
    /// this end an HTTP proxy (`CONNECT` and absolute form, one request for
    /// each stream, 8 KiB of head at most). A target is the name of
    /// another service of this listener; there is no DNS lookup.
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

/// What a service does with its streams.
enum Service {
    /// Copy the bytes to a target (`raw` and each `NAME=TARGET`).
    Target(Target),
    /// Serve SOCKS5 on the stream (`--service socks5`, T-291).
    Socks5,
    /// Serve an HTTP proxy on the stream (`--service http`, T-292).
    Http,
}

impl std::fmt::Display for Service {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Service::Target(target) => write!(f, "{target}"),
            Service::Socks5 => f.write_str("socks5"),
            Service::Http => f.write_str("http"),
        }
    }
}

/// The services of a listener: a name and what its streams do.
type Services = Arc<BTreeMap<String, Service>>;

/// The services of --to and --service. A bad name, a bad target, a
/// duplicate name or more than 16 services exits 64.
fn services_of(args: &ListenArgs) -> Result<BTreeMap<String, Service>> {
    let usage = |text: String| exit::fault(Fault::Usage, anyhow!(text));
    let target =
        |text: &str, flag: &str| Target::parse(text).map_err(|e| usage(format!("{flag} {e}")));
    let mut out = BTreeMap::new();
    if let Some(text) = &args.to {
        out.insert(
            service::RAW.to_string(),
            Service::Target(target(text, "--to")?),
        );
    }
    for item in &args.service {
        if item == podnet::link_socks::NAME || item == podnet::link_http::NAME {
            let kind = if item == podnet::link_http::NAME {
                Service::Http
            } else {
                Service::Socks5
            };
            if out.insert(item.clone(), kind).is_some() {
                return Err(usage(format!("--service {item}: given twice")));
            }
            continue;
        }
        let (name, text) = item.split_once('=').ok_or_else(|| {
            usage(format!(
                "--service {item:?}: give NAME=TARGET (or `socks5` or `http` with no target)"
            ))
        })?;
        service::check_name(name)?;
        if name == podnet::link_socks::NAME || name == podnet::link_http::NAME {
            return Err(usage(format!(
                "--service {name}: the service {name} takes no target: give `--service {name}`"
            )));
        }
        let found = Service::Target(target(text, "--service")?);
        if out.insert(name.to_string(), found).is_some() {
            return Err(usage(format!(
                "--service {name}: the service {name:?} has two targets (--to is the service raw)"
            )));
        }
    }
    if args.announce && out.contains_key(announce::NAME) {
        return Err(usage(format!(
            "--service {}: this name is the stream of --announce",
            announce::NAME
        )));
    }
    if args.announce && out.contains_key(podnet::link_forward::NAME) {
        return Err(usage(format!(
            "--service {}: this name is the stream of a forwarded stream (--announce)",
            podnet::link_forward::NAME
        )));
    }
    if out.len() > MAX_SERVICES {
        return Err(usage(format!(
            "a listener has {MAX_SERVICES} services at most"
        )));
    }
    Ok(out)
}

pub(crate) async fn listen(mut args: ListenArgs) -> Result<()> {
    args.address = super::apply_query(&args.address, &mut args.carrier)?;
    let carrier = super::open_carrier(&args.carrier)?;
    let services: Services = Arc::new(services_of(&args)?);
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
    let _guard = held
        .mesh
        .as_ref()
        .map(|mesh| Guard::start(mesh, &link, peer, held.weight, INTERVAL_MS));
    while let Ok(stream) = link.accept().await {
        slot.count_accepted();
        let (services, shown) = (held.services.clone(), shown.clone());
        // `close-stream` of the control socket names the stream by its id.
        let tracked = slot.track(&stream);
        let hub = held.hub.clone();
        let announced = held
            .mesh
            .clone()
            .map(|mesh| (mesh, link.clone(), held.weight));
        tokio::spawn(async move {
            let mut stream = stream;
            tokio::select! {
                done = bridge(&mut stream, &services, announced, peer) => {
                    if let Err(err) = done {
                        // Tell the opener that the stream is over.
                        let _ = tokio::io::AsyncWriteExt::shutdown(&mut stream).await;
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
async fn bridge(
    stream: &mut link::Stream,
    services: &BTreeMap<String, Service>,
    announced: Option<(Arc<Mesh>, Arc<Link>, Option<u32>)>,
    peer: PublicKey,
) -> Result<()> {
    let mut name = service::accept(stream).await?;
    if let Some((mesh, link, weight)) = announced {
        if name == announce::NAME {
            announce::receive_stream(mesh, link, peer, weight, stream).await;
            return Ok(());
        }
        // A stream for another node goes on; a stream for this node is a
        // request for its service.
        if name == podnet::link_forward::NAME {
            match podnet::link_forward::accept(&mesh, stream).await? {
                podnet::link_forward::Arrived::Local(local) => name = local,
                podnet::link_forward::Arrived::Done => return Ok(()),
            }
        }
    }
    let Some(found) = services.get(&name) else {
        service::reply(stream, service::NO_SERVICE).await?;
        return Err(anyhow!(
            "the peer asked for the service {name:?}, which this listener does not have"
        ));
    };
    let target = match found {
        Service::Target(target) => target,
        Service::Socks5 => {
            service::reply(stream, service::OPEN).await?;
            return podnet::link_socks::serve(stream).await;
        }
        Service::Http => {
            service::reply(stream, service::OPEN).await?;
            return podnet::link_http::serve(stream, resolver(services)).await;
        }
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

/// The names that the service `http` reaches: each service with a target.
fn resolver(services: &BTreeMap<String, Service>) -> podnet::link_http::Resolver {
    use podnet::link_http::Origin;
    let map: BTreeMap<String, Origin> = services
        .iter()
        .filter_map(|(name, service)| match service {
            Service::Target(Target::Tcp { host, port }) => {
                Some((name.clone(), Origin::Tcp(host.clone(), *port)))
            }
            Service::Target(Target::Unix(path)) => Some((name.clone(), Origin::Unix(path.clone()))),
            _ => None,
        })
        .collect();
    Arc::new(move |name| map.get(name).cloned())
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
