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
    let _guard = args.announce.then(|| {
        let mesh = Mesh::new(me.clone(), Vec::new(), announce::now_ms());
        tokio::spawn(announce::serve_streams(
            mesh.clone(),
            link.clone(),
            peer,
            args.announce_weight,
        ));
        Guard::start(&mesh, &link, peer, args.announce_weight, INTERVAL_MS)
    });
    let mut stream = link.open().map_err(link::fault)?;
    podnet::link_service::request(&mut stream, &args.service).await?;
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
