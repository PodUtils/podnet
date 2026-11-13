//! `podnet link dial`.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use clap::Args;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use podnet::exit::{self, Fault};
use podnet::identity::{self, AllowList};
use podnet::link;
use podnet::link_announce::{self as announce, Guard, Mesh};
use podnet::mesh_announce::INTERVAL_MS;

use super::CarrierArgs;

#[derive(Args, Debug)]
pub(crate) struct DialArgs {
    /// The address of the listener, in the form of the carrier (for `tcp`:
    /// `KEY@HOST:PORT`, with the key of the listener).
    /// A query can follow the address: `ADDRESS?key=value&key=value` (keys:
    /// carrier, tls, retry, lb, compress, pad, mtu, maxhops, poll, window,
    /// name). Only `carrier` has an effect yet; a key with no effect, a repeated
    /// key, an unknown key or a secret key (password, key, token, identity)
    /// exits 64, and no message shows a value.
    #[arg(value_name = "ADDRESS")]
    pub(crate) address: String,
    #[command(flatten)]
    pub(crate) carrier: CarrierArgs,
    /// The fingerprint that the listener must prove (the line that its
    /// `podnet link listen` prints). Repeat it for more than one.
    #[arg(long, value_name = "FINGERPRINT", conflicts_with = "allow")]
    pub(crate) peer: Vec<String>,
    /// An allow list file, as `link listen --allow`, of the listeners that
    /// this install trusts. Give --peer or --allow: the list is default
    /// deny, and a listener that it does not hold exits 77.
    #[arg(long, value_name = "FILE")]
    pub(crate) allow: Option<PathBuf>,
    /// The service to ask the listener for (`link listen --service`). A name
    /// of 1 to 32 characters from A-Z a-z 0-9 . _ -. A listener with no such
    /// service, or whose origin does not answer, exits 69.
    #[arg(long, value_name = "NAME", default_value = podnet::link_service::RAW)]
    pub(crate) service: String,
    /// Send and take signed route announcements on this link (the service
    /// `announce`; see `link listen --announce`). The listener must run it
    /// too. The dialer has no control socket: it logs a dropped announcement
    /// and shows no routes.
    #[arg(long, conflicts_with = "check")]
    pub(crate) announce: bool,
    /// With --announce: the cost of the link (1 or more). Default: the RTT.
    #[arg(long, value_name = "COST", requires = "announce", value_parser = clap::value_parser!(u32).range(1..))]
    pub(crate) announce_weight: Option<u32>,
    /// With --announce: reach the service of the node with this fingerprint
    /// (as `routes` shows it), which may be many links away. The stream goes
    /// to the next hop of the best route and is opened again at each hop. The
    /// dialer waits up to 20 s for a route. A node with no route, a service
    /// that the node does not have, or a path longer than --hops exits 69.
    #[arg(long, value_name = "FINGERPRINT", requires = "announce")]
    pub(crate) node: Option<String>,
    /// With --node: the most links that the stream may cross, 1 to 16.
    #[arg(long, value_name = "N", requires = "node", default_value_t = podnet::link_forward::DEFAULT_HOPS, value_parser = clap::value_parser!(u8).range(1..=16))]
    pub(crate) hops: u8,
    /// Connect and check the peer, print its fingerprint, and exit 0. It
    /// opens no stream.
    #[arg(long)]
    pub(crate) check: bool,
    /// With --check: print one JSON document.
    #[arg(long, requires = "check")]
    pub(crate) json: bool,
}

/// The allow list of --peer or --allow. Exit 64 when there is none.
pub(super) fn trusted(allow: &Option<PathBuf>, peer: &[String]) -> Result<AllowList> {
    match allow {
        Some(path) => AllowList::read(path),
        None if peer.is_empty() => Err(exit::fault(
            Fault::Usage,
            anyhow!(
                "give --peer FINGERPRINT or --allow FILE: the dialer trusts no listener by default"
            ),
        )),
        None => AllowList::parse(&peer.join("\n")).map_err(|e| e.context("--peer")),
    }
}

/// The time that the dialer waits for a route to the node of --node.
const ROUTE_WAIT: std::time::Duration = std::time::Duration::from_secs(20);

/// Open a stream to the service of the node with the fingerprint `node`.
async fn open_node(mesh: &Arc<Mesh>, node: &str, service: &str, hops: u8) -> Result<link::Stream> {
    let unavailable = |e: anyhow::Error| exit::fault(Fault::Unavailable, e);
    let end = std::time::Instant::now() + ROUTE_WAIT;
    let dest = loop {
        if let Some(dest) = mesh.origin_named(node) {
            break dest;
        }
        if std::time::Instant::now() >= end {
            return Err(unavailable(anyhow!(
                "no route to the node {node} in {} s",
                ROUTE_WAIT.as_secs()
            )));
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    };
    podnet::link_forward::open(mesh, &dest, service, hops)
        .await
        .map_err(|e| unavailable(anyhow!("the stream to the node {node}: {e}")))
}

pub(crate) async fn dial(mut args: DialArgs) -> Result<()> {
    args.address = super::apply_query(&args.address, &mut args.carrier)?;
    let carrier = super::open_carrier(&args.carrier)?;
    let allow = trusted(&args.allow, &args.peer)?;
    let me = Arc::new(identity::create(&identity::default_dir()?)?);
    let (link, peer) = super::dial_peer(carrier, &args.address, &me, allow).await?;
    let (mine, theirs) = (me.fingerprint(), peer.display_fingerprint());
    if args.check {
        if args.json {
            println!(
                "{}",
                podnet::status::stamped(serde_json::json!({
                    "carrier": args.carrier.carrier,
                    "address": args.address,
                    "fingerprint": mine,
                    "peer": theirs,
                }))
            );
        } else {
            println!("podnet: peer admitted   {theirs}");
        }
        link.close().await;
        return Ok(());
    }
    eprintln!("podnet: peer {theirs}, this install {mine}");
    let link = Arc::new(link);
    // The guard stops the announcements and ends the route at the exit.
    let mesh = args
        .announce
        .then(|| Mesh::new(me.clone(), Vec::new(), announce::now_ms()));
    let _guard = mesh.as_ref().map(|mesh| {
        tokio::spawn(announce::serve_streams(
            mesh.clone(),
            link.clone(),
            peer,
            args.announce_weight,
        ));
        Guard::start(mesh, &link, peer, args.announce_weight, INTERVAL_MS)
    });
    let stream = match (&args.node, &mesh) {
        (Some(node), Some(mesh)) => open_node(mesh, node, &args.service, args.hops).await?,
        _ => {
            let mut stream = link.open().map_err(link::fault)?;
            podnet::link_service::request(&mut stream, &args.service).await?;
            stream
        }
    };
    let (mut down, mut up) = tokio::io::split(stream);
    let send = tokio::spawn(async move {
        let mut stdin = tokio::io::stdin();
        let _ = tokio::io::copy(&mut stdin, &mut up).await;
        let _ = up.shutdown().await;
    });
    let mut stdout = tokio::io::stdout();
    let mut buf = vec![0u8; 16 * 1024];
    loop {
        let n = down.read(&mut buf).await.unwrap_or(0);
        if n == 0 {
            break;
        }
        stdout.write_all(&buf[..n]).await?;
        stdout.flush().await?;
    }
    let stdin_done = send.is_finished();
    send.abort();
    link.close().await;
    if !stdin_done {
        // A blocking read of stdin cannot be stopped, and the runtime would
        // wait for it at exit: the far end has ended the stream, so exit now.
        stdout.flush().await?;
        std::process::exit(0);
    }
    Ok(())
}
