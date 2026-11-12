//! `podnet link dial`.

use std::path::PathBuf;

use anyhow::{anyhow, Result};
use clap::Args;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use podnet::exit::{self, Fault};
use podnet::identity::{self, AllowList};
use podnet::link;

use super::CarrierArgs;

#[derive(Args, Debug)]
pub(crate) struct DialArgs {
    /// The address of the listener, in the form of the carrier (for `tcp`:
    /// `KEY@HOST:PORT`, with the key of the listener).
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

pub(crate) async fn dial(args: DialArgs) -> Result<()> {
    let carrier = super::open_carrier(&args.carrier)?;
    let allow = trusted(&args.allow, &args.peer)?;
    let me = identity::create(&identity::default_dir()?)?;
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
    send.abort();
    link.close().await;
    Ok(())
}
