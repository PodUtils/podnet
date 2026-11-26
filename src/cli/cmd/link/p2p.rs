//! `podnet link p2p`: a TCP stream to a peer, on the direct path first and
//! through a TURN relay when the direct path is refused or times out.

use std::time::Duration;

use anyhow::{anyhow, Result};
use clap::Args;

use pod_core::Target;
use podnet::exit::{self, Fault};
use podnet::link_p2p::{self, Path};
use podnet::network::{Ctx, Link, Network as _};
use podnet::networks;

/// The default time limit of the direct path.
pub(crate) const DEFAULT_TIMEOUT_MS: u64 = 3000;

#[derive(Args, Debug)]
pub(crate) struct P2pArgs {
    /// The peer to reach: HOST:PORT.
    #[arg(long, value_name = "HOST:PORT")]
    pub(crate) peer: String,
    /// The address of the direct path, when it differs from --peer. A peer
    /// that answers here takes no relay.
    #[arg(long, value_name = "HOST:PORT")]
    pub(crate) direct: Option<String>,
    /// The relay: a network spec (turn://HOST[:PORT] or turns://HOST[:PORT],
    /// with PODNET_TURN_USER and PODNET_TURN_PASSWORD_FILE). It is used only
    /// when the direct path is refused or times out.
    #[arg(long, value_name = "SPEC")]
    pub(crate) relay: Option<String>,
    /// The time limit of the direct path, in milliseconds.
    #[arg(long, value_name = "MS", default_value_t = DEFAULT_TIMEOUT_MS)]
    pub(crate) timeout: u64,
    /// Connect, print the path and exit. It carries no byte.
    #[arg(long)]
    pub(crate) check: bool,
    /// Print one JSON document.
    #[arg(long)]
    pub(crate) json: bool,
}

/// The way of the direct path.
async fn direct_way() -> Result<std::sync::Arc<dyn podnet::network::Dial>> {
    let network = networks::parse("direct")?;
    let link = network.up(&Ctx::default()).await?;
    link.way
        .map(|way| way.dial)
        .ok_or_else(|| anyhow!("the direct network gave no way out"))
}

/// The line of a path and its allocation, for the text output.
fn lines(
    path: Path,
    failure: Option<pod_core::Class>,
    status: Option<&serde_json::Value>,
) -> Vec<String> {
    let mut out = vec![format!("path        {}", path.name())];
    if let Some(class) = failure {
        out.push(format!("direct      {}", class.name()));
    }
    if path == Path::Relay {
        let lifetime = status
            .and_then(|s| s["lifetime_s"].as_u64())
            .map_or("-".to_string(), |s| format!("{s} s"));
        let rate = status
            .and_then(|s| s["rate_limit"].as_u64())
            .map_or("-".to_string(), |b| format!("{b} B/s"));
        out.push(format!("lifetime    {lifetime}"));
        out.push(format!("rate limit  {rate}"));
    }
    out
}

fn json(
    path: Path,
    failure: Option<pod_core::Class>,
    status: Option<&serde_json::Value>,
) -> serde_json::Value {
    serde_json::json!({
        "path": path.name(),
        "direct_failure": failure.map(|c| c.name()),
        "lifetime_s": status.and_then(|s| s["lifetime_s"].as_u64()),
        "rate_limit": status.and_then(|s| s["rate_limit"].as_u64()),
    })
}

/// The relay network, up, and its link (kept for its close function).
async fn relay_link(spec: &str) -> Result<Link> {
    let network = networks::parse(spec)?;
    match network.up(&Ctx::default()).await {
        Ok(link) => Ok(link),
        Err(err) if link_p2p::refuses_udp(&format!("{err:#}")) => {
            // A named skip: the host refuses the UDP socket, and its words
            // are the reason. No relay is attempted, and no byte is lost.
            let line = link_p2p::skip_line(&format!("{err:#}"));
            Err(exit::fault(Fault::Unavailable, anyhow!(line)))
        }
        Err(err) => Err(err),
    }
}

pub(crate) async fn p2p(args: P2pArgs) -> Result<()> {
    let peer: Target = args
        .peer
        .parse()
        .map_err(|e| exit::fault(Fault::Usage, anyhow!("--peer: {e}")))?;
    let direct_target: Target = match &args.direct {
        Some(text) => text
            .parse()
            .map_err(|e| exit::fault(Fault::Usage, anyhow!("--direct: {e}")))?,
        None => peer.clone(),
    };
    let timeout = Duration::from_millis(args.timeout.max(1));
    let direct = direct_way().await?;
    let mut relay = None;
    let relay_way = match &args.relay {
        Some(spec) => {
            let link = relay_link(spec).await?;
            let way = link
                .way
                .as_ref()
                .map(|way| way.dial.clone())
                .ok_or_else(|| anyhow!("{spec}: the network gave no way out"))?;
            relay = Some(link);
            Some(way)
        }
        None => None,
    };
    let status = relay.as_ref().and_then(|link| {
        if link.status.is_null() {
            None
        } else {
            Some(link.status.clone())
        }
    });
    let opened = link_p2p::open(&direct, &direct_target, relay_way.as_ref(), &peer, timeout).await;
    let opened = match opened {
        Ok(opened) => opened,
        Err(failure) => {
            if let Some(link) = relay.take() {
                link.close().await;
            }
            return Err(exit::fault(fault_of(failure.class), anyhow!(failure)));
        }
    };
    if args.json {
        println!(
            "{}",
            podnet::status::stamped(json(opened.path, opened.direct_failure, status.as_ref()))
        );
    } else {
        println!("peer        {peer}");
        for line in lines(opened.path, opened.direct_failure, status.as_ref()) {
            println!("{line}");
        }
    }
    if args.check {
        if let Some(link) = relay.take() {
            link.close().await;
        }
        return Ok(());
    }
    // The stream is a byte pipe: stdin up, stdout down. At the end of stdin
    // the FIN goes to the peer, and the answer is read until the peer
    // closes (T-592: the end of stdin dropped the answer).
    let end = pod_warp::nc::PipeEnd {
        shutdown: true,
        idle: pod_warp::nc::TCP_IDLE,
    };
    let piped = pod_warp::nc::pipe_with(opened.io, tokio::io::stdin(), tokio::io::stdout(), end)
        .await
        .map_err(|failure| exit::fault(fault_of(failure.class), anyhow!(failure)));
    if let Some(link) = relay.take() {
        link.close().await;
    }
    piped
}

/// The exit class of a failure of the path.
fn fault_of(class: pod_core::Class) -> Fault {
    match class {
        pod_core::Class::Usage => Fault::Usage,
        pod_core::Class::Refused => Fault::NoPermission,
        pod_core::Class::Timeout => Fault::Timeout,
        pod_core::Class::Config => Fault::Config,
        pod_core::Class::NotFound => Fault::NoHost,
        _ => Fault::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lines_name_the_path_the_failure_and_the_lifetime() {
        let relay = serde_json::json!({"lifetime_s": 600, "rate_limit": 500_000});
        let text = lines(Path::Relay, Some(pod_core::Class::Refused), Some(&relay)).join("\n");
        assert!(text.contains("path        relay"), "{text}");
        assert!(text.contains("direct      refused"), "{text}");
        assert!(text.contains("lifetime    600 s"), "{text}");
        assert!(text.contains("rate limit  500000 B/s"), "{text}");
        let direct = lines(Path::Direct, None, None).join("\n");
        assert!(
            direct.contains("path        direct") && !direct.contains("lifetime"),
            "{direct}"
        );
    }

    #[test]
    fn the_json_names_the_path_and_the_allocation() {
        let relay = serde_json::json!({"lifetime_s": 90, "rate_limit": null});
        let doc = json(Path::Relay, Some(pod_core::Class::Timeout), Some(&relay));
        assert_eq!(doc["path"], "relay");
        assert_eq!(doc["direct_failure"], "timeout");
        assert_eq!(doc["lifetime_s"], 90);
        assert!(doc["rate_limit"].is_null());
    }
}
