//! `podnet warp --tuic nc`: nc through a TUIC version 5 road (issue 486,
//! QUIC-8), with no WARP identity and no WARP session.
//!
//! TCP is one bidirectional stream of the QUIC connection after the
//! `Connect` command. UDP is the bounded queue of the road: a full queue
//! stops the read of stdin, as the stop-read signal of QUIC-8.

use std::time::Duration;

use anyhow::Result;

use podnet::exit::{Fault, ResultExt};
use podnet::roads::tuic::{Link, Tuic};

/// The gate: nc HOST PORT through the link.
pub(crate) async fn nc(
    link: &str,
    host: &str,
    port: u16,
    udp: bool,
    idle: Option<u64>,
    shutdown: bool,
) -> Result<()> {
    let link = Link::parse(link).fault(Fault::Usage)?;
    let client = Tuic::connect(&link).await.fault(Fault::Unavailable)?;
    if udp {
        let (out, back) = client
            .udp(host, port)
            .fault(Fault::Unavailable)?
            .into_ends();
        return podnet::warp::nc::udp_pipe_with(
            out,
            back,
            tokio::io::stdin(),
            tokio::io::stdout(),
            Duration::from_secs(idle.unwrap_or(3)),
        )
        .await
        .fault(Fault::Unavailable);
    }
    let io = client.tcp_io(host, port).await.fault(Fault::Unavailable)?;
    let end = podnet::warp::nc::PipeEnd {
        shutdown,
        idle: idle.map_or(podnet::warp::nc::TCP_IDLE, Duration::from_secs),
    };
    podnet::warp::nc::pipe_with(Box::new(io), tokio::io::stdin(), tokio::io::stdout(), end)
        .await
        .fault(Fault::Unavailable)?;
    // A read of stdin blocks a thread: end the process here, as warp nc.
    std::process::exit(0);
}
