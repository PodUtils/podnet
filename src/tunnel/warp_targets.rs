//! The edge addresses and the WARP session of the `quic+warp` leg.

use std::sync::Arc;

use anyhow::{bail, Context, Result};
use tracing::info;

use super::{edge, edge_override, IpVersionFilter, WarpClient, WarpLink};

pub(super) async fn warp_targets(
    identity: Option<&std::path::Path>,
    masque_endpoint: Option<std::net::SocketAddr>,
) -> Result<(Vec<edge::EdgeAddr>, Arc<WarpLink>, noq::Endpoint)> {
    let discovered = match edge_override()? {
        Some(list) => list,
        None => crate::edge_discovery::discover(IpVersionFilter::V4Only)
            .await
            .context("edge discovery")?,
    };
    let Some(first) = discovered.first() else {
        bail!("edge discovery returned no addresses");
    };
    let client = match identity {
        Some(path) => WarpClient::register_or_load(path)
            .await
            .context("registering with WARP")?,
        None => WarpClient::register()
            .await
            .context("registering with WARP")?,
    };
    let client = client.with_masque(crate::warp::MasqueOptions {
        endpoint: masque_endpoint,
        ..Default::default()
    });
    let stack = client
        .connect()
        .await
        .context("opening the WARP MASQUE tunnel on TCP 443")?;
    let endpoint = crate::warp::quic::endpoint(&stack)
        .await
        .context("building the QUIC socket inside the WARP session")?;
    info!(
        edge = %first.socket(),
        "the tunnel edge is reached through WARP on TCP 443"
    );
    let link = Arc::new(WarpLink {
        client,
        current: tokio::sync::Mutex::new((stack, endpoint.clone(), std::time::Instant::now())),
    });
    Ok((discovered, link, endpoint))
}
