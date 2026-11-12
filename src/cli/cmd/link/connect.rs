//! `podnet link connect`: hold a link to a listener (T-266).

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use clap::Args;

use podnet::exit::{self, Fault};
use podnet::identity;
use podnet::link_ctl::Hub;

use super::dial::trusted;
use super::CarrierArgs;
use crate::cli::opts::DetachArgs;
use crate::cli::signals::shutdown_signal;

#[derive(Args, Debug)]
pub(crate) struct ConnectArgs {
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
    /// this install trusts. Give --peer or --allow (else exit 64); a
    /// listener that the list does not hold exits 77.
    #[arg(long, value_name = "FILE")]
    pub(crate) allow: Option<PathBuf>,
    /// The control socket (mode 0600, with the token file PATH.token). The
    /// default is `link-PID.ctl` in the run directory of podnet. The ready
    /// document names it.
    #[arg(long, value_name = "PATH")]
    pub(crate) control: Option<PathBuf>,
    /// Print the ready document as one line of JSON in place of the text.
    #[arg(long)]
    pub(crate) json: bool,
    #[command(flatten)]
    pub(crate) detach: DetachArgs,
}

pub(crate) async fn connect(mut args: ConnectArgs) -> Result<()> {
    args.address = super::apply_query(&args.address, &mut args.carrier)?;
    let carrier = super::open_carrier(&args.carrier)?;
    let allow = trusted(&args.allow, &args.peer)?;
    let me = identity::create(&identity::default_dir()?)?;
    let (link, peer) = super::dial_peer(carrier, &args.address, &me, allow).await?;
    let shown = peer.display_fingerprint();
    // T-308: a name maps to the fingerprint that it proved first.
    if let Some((dir, name)) = podnet::detach::instance() {
        podnet::link_alias::bind(&dir, &name, &shown)?;
    }
    let link = Arc::new(link);
    let hub = Hub::new(&me.fingerprint());
    let slot = hub.hold(
        link.clone(),
        peer,
        &args.carrier.carrier,
        &args.address,
        "dialer",
    );
    let control = super::start_control(&hub, args.control.as_deref())?;
    let doc = serde_json::json!({
        "carrier": args.carrier.carrier,
        "address": args.address,
        "fingerprint": me.fingerprint(),
        "peer": shown,
        "control": control.as_ref().map(|c| c.path().display().to_string()),
    });
    podnet::detach::arm_stop();
    if args.json {
        println!("{}", podnet::status::stamped(doc.clone()));
    } else {
        println!(
            "podnet: link connected  {} {}",
            args.carrier.carrier, args.address
        );
        println!("podnet: peer admitted   {shown}");
        println!("podnet: fingerprint     {}", me.fingerprint());
        if let Some(control) = &control {
            println!(
                "podnet: control socket  {} (mode 0600)",
                control.path().display()
            );
        }
    }
    podnet::detach::ready(doc);
    // The peer may open streams too. This end offers no service: it closes
    // each one.
    loop {
        tokio::select! {
            _ = shutdown_signal() => {
                link.close().await;
                return Ok(());
            }
            next = link.accept() => match next {
                Ok(stream) => {
                    slot.count_accepted();
                    drop(stream);
                }
                Err(err) => {
                    return Err(exit::fault(
                        Fault::Unavailable,
                        anyhow!("the link to {shown} ended: {err}"),
                    ));
                }
            },
        }
    }
}
