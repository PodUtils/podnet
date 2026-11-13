//! `podnet link connect`: hold a link to a listener (T-266).

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use clap::Args;

use pod_identity::PublicKey;
use podnet::exit::{self, Fault};
use podnet::identity;
use podnet::link::Link;
use podnet::link_ctl::{Dialing, Hub, Redial, Slot};

use super::dial::trusted;
use super::services::{services_of, Seal, ServiceArgs, Streams};
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
    #[command(flatten)]
    pub(crate) services: ServiceArgs,
    /// The control socket (mode 0600, with the token file PATH.token). The
    /// default is `link-PID.ctl` in the run directory of podnet. The ready
    /// document names it.
    #[arg(long, value_name = "PATH")]
    pub(crate) control: Option<PathBuf>,
    /// Print the ready document as one line of JSON in place of the text.
    #[arg(long)]
    pub(crate) json: bool,
    /// Connect through the --egress elements and check the peer, print the
    /// result (each element, then the peer), and exit 0. It holds no link and
    /// starts no control socket. An element that fails exits 69 and is named.
    #[arg(long, conflicts_with_all = ["detach", "control"])]
    pub(crate) check: bool,
    #[command(flatten)]
    pub(crate) detach: DetachArgs,
}

pub(crate) async fn connect(mut args: ConnectArgs, egress: Vec<String>) -> Result<()> {
    args.address = super::apply_query(&args.address, &mut args.carrier)?;
    let services = Arc::new(services_of(None, &args.services, false)?);
    let carrier = super::open_carrier(&args.carrier)?;
    let allow = trusted(&args.allow, &args.peer)?;
    let me = Arc::new(identity::create(&identity::default_dir()?)?);
    let (chain, rows) =
        super::open_chain(&egress, &args.carrier, &args.address, args.check, args.json).await?;
    let via = chain.as_ref().map(podnet::link_chain::Opened::dial);
    let (link, peer) =
        super::dial_peer(carrier, &args.address, &me, allow.clone(), via.clone()).await?;
    let shown = peer.display_fingerprint();
    if args.check {
        // No name, no control socket and no listener.
        if args.json {
            println!(
                "{}",
                podnet::status::stamped(serde_json::json!({
                    "carrier": args.carrier.carrier,
                    "address": args.address,
                    "fingerprint": me.fingerprint(),
                    "peer": shown,
                    "elements": rows,
                }))
            );
        } else {
            println!("podnet: peer admitted   {shown}");
        }
        link.close().await;
        return Ok(());
    }
    // T-308: a name maps to the fingerprint that it proved first.
    if let Some((dir, name)) = podnet::detach::instance() {
        podnet::link_alias::bind(&dir, &name, &shown)?;
    }
    let link = Arc::new(link);
    let hub = Hub::new(&me.fingerprint());
    hub.reserve(services.keys());
    // T-307: `reconnect` and `rotate` dial again with what this dial used.
    let dialer = Arc::new(Dialer {
        args: args.carrier.clone(),
        address: args.address.clone(),
        me: me.clone(),
        allow: allow.clone(),
        services: services.clone(),
        via: via.clone(),
        _chain: std::sync::Mutex::new(chain),
    });
    hub.attach_redial(dialer.clone());
    dialer.serve(&hub, link, peer);
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
    // The instance ends at a stop signal, or when no link is left (the link
    // ended, or a `reconnect` could not dial again).
    let down = async {
        while !hub.down() {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    };
    tokio::select! {
        _ = shutdown_signal() => {
            for link in hub.links() {
                link.close().await;
            }
            Ok(())
        }
        _ = down => Err(exit::fault(
            Fault::Unavailable,
            anyhow!("the link to {shown} ended"),
        )),
    }
}

/// What a dial of `link connect` needs to be made again (T-307).
struct Dialer {
    args: CarrierArgs,
    address: String,
    me: Arc<identity::Identity>,
    allow: identity::AllowList,
    services: super::services::Services,
    via: Option<Arc<dyn podnet::network::Dial>>,
    /// Keeps the chain of --egress up.
    _chain: std::sync::Mutex<Option<podnet::link_chain::Opened>>,
}

impl Dialer {
    /// Hold `link` in `hub` and serve it in its own task: the streams that
    /// the peer opens go to the services of --service, after the grant of the
    /// peer in the allow list (T-290). With no service, each stream gets the
    /// answer "no such service". The task ends with the link.
    fn serve(&self, hub: &Arc<Hub>, link: Arc<Link>, peer: PublicKey) -> u64 {
        let shown = peer.display_fingerprint();
        let slot = hub.hold(
            link.clone(),
            peer,
            &self.args.carrier,
            &self.address,
            "dialer",
        );
        let id = slot.id();
        let streams = Streams {
            hub: hub.clone(),
            services: self.services.clone(),
            grant: self.allow.grant(&peer).unwrap_or_default(),
            peer,
            shown: shown.clone(),
            mesh: None,
            seal: self.args.record.clone().map(|options| Seal {
                me: self.me.clone(),
                allow: self.allow.clone(),
                options,
            }),
        };
        tokio::spawn(accept_streams(link, slot, streams));
        id
    }
}

async fn accept_streams(link: Arc<Link>, slot: Slot, streams: Streams) {
    while let Ok(stream) = link.accept().await {
        streams.spawn(stream, &slot);
    }
}

impl Redial for Dialer {
    fn redial<'a>(&'a self, hub: &'a Arc<Hub>, expect: &'a PublicKey) -> Dialing<'a> {
        Box::pin(async move {
            let carrier = super::open_carrier(&self.args)?;
            let (link, peer) = super::dial_peer(
                carrier,
                &self.address,
                &self.me,
                self.allow.clone(),
                self.via.clone(),
            )
            .await?;
            let shown = peer.display_fingerprint();
            // The new link proves the key of the old one, and the name keeps
            // its fingerprint (T-308).
            let checked = if peer != *expect {
                Err(exit::fault(
                    Fault::NoPermission,
                    anyhow!("the peer proved {shown}, not the key of the link that it replaces"),
                ))
            } else if let Some((dir, name)) = podnet::detach::instance() {
                podnet::link_alias::bind(&dir, &name, &shown)
            } else {
                Ok(())
            };
            if let Err(err) = checked {
                link.close().await;
                return Err(err);
            }
            Ok(self.serve(hub, Arc::new(link), peer))
        })
    }
}
