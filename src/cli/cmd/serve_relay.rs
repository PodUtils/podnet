//! `serve --ingress relay` (T-091): the file server behind a node of a relay
//! pair. The router listens on a private unix socket in the run
//! directory, and the node of the pair connects each session of an
//! operator to it (as `serve --local unix:PATH` with `relay node --to
//! unix:PATH`, in one process).

use std::path::Path;

use anyhow::{Context, Result};

use podnet::exit::{Fault, ResultExt};

use crate::cli::cmd::local::bind_local;
use crate::cli::opts::Show;
use crate::cli::signals::sleep_or_signal;

/// Serve `router` through the node of the pair `name` (default: the pair
/// with the most time left).
pub(super) async fn serve_relay(
    name: Option<&str>,
    router: axum::Router,
    dir: &Path,
    show: Show,
    pin: Option<&str>,
    run_for: Option<u64>,
) -> Result<()> {
    use podnet::relay_reverse as reverse;
    let pairs = reverse::pairs_dir()?;
    // With no pair name, the node takes the next stored pair at the expiry.
    let follow = name.is_none().then_some(pairs.as_path());
    let pair = reverse::load(&pairs, name)?;
    podnet::report::hide(&pair.node_token);
    podnet::ws_relay::Relay::single(&pair.relay, None).fault(Fault::Config)?;
    let socket = podnet::instances::run_dir()?.join(format!(
        "serve-{}.sock",
        &podnet::util::random_token()[..16]
    ));
    let local = bind_local(&format!("unix:{}", socket.display()), false, router).await?;
    let target = reverse::Target::Unix(socket);
    let shown = dir.display().to_string();
    podnet::detach::arm_stop();
    if show.json {
        let doc = serde_json::json!({
            "event": "ready",
            "pair": pair.name,
            "relay": pair.relay,
            "dir": shown,
            "pin": pin,
        });
        println!("{}", podnet::status::stamped(doc));
    } else {
        println!(
            "podnet: serve  {shown}  through the relay pair {}  ({})",
            pair.name, pair.relay
        );
        if let Some(pin) = pin {
            println!("podnet: PIN    {pin}");
        }
        println!(
            "The operator: podnet relay connect {} --listen tcp://127.0.0.1:8080",
            pair.name
        );
    }
    // T-236: --detach returns when the node is connected (run_node calls
    // ready), not before: a visitor before that gets "node offline".
    podnet::detach::ready_when_reachable(serde_json::json!({ "pair": pair.name, "pin": pin }));
    let (server, _file) = local.split();
    tokio::select! {
        result = reverse::run_node(follow, pair, &target) => result,
        _ = sleep_or_signal(run_for) => Ok(()),
        ended = server => {
            ended
                .context("the local server stopped")?
                .context("the local server stopped")
        }
    }
}
