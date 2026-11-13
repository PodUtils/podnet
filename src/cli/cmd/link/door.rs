//! `podnet link dial --service socks5 --listen ADDRESS` (T-291): the local
//! SOCKS5 door at the opener of a link (`podnet::link_door`).

use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};

use podnet::exit::{self, Fault};
use podnet::link::Link;
use podnet::link_door::{Resolve, Way};

use crate::cli::signals::shutdown_signal;

/// Listen at `listen` and serve the door until a stop signal or the end of
/// the link.
pub(super) async fn run(link: Arc<Link>, listen: &str, local: bool) -> Result<()> {
    let resolve = if local { Resolve::Local } else { Resolve::Peer };
    let door = Arc::new(Way::new(link.clone(), resolve).door());
    let bind =
        |e: std::io::Error| exit::fault(Fault::Unavailable, anyhow!("--listen {listen:?}: {e}"));
    let (shown, serving): (String, tokio::task::JoinHandle<Result<()>>) =
        match listen.strip_prefix("unix:") {
            Some(path) => {
                let _ = std::fs::remove_file(path);
                let socket = tokio::net::UnixListener::bind(path).map_err(bind)?;
                (
                    format!("unix:{path}"),
                    tokio::spawn(door.serve_unix(socket)),
                )
            }
            None => {
                let socket = tokio::net::TcpListener::bind(listen).await.map_err(bind)?;
                let at = socket.local_addr().context("the address of the door")?;
                (at.to_string(), tokio::spawn(door.serve_tcp(socket)))
            }
        };
    println!(
        "podnet: socks5 door     {shown} (names resolve at the {})",
        if local { "local host" } else { "peer" }
    );
    let ended = async {
        loop {
            if link.ended().is_some() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    };
    let result = tokio::select! {
        _ = shutdown_signal() => Ok(()),
        _ = ended => Err(exit::fault(Fault::Unavailable, anyhow!("the link to the peer ended"))),
        done = serving => match done {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(e),
            Err(e) => Err(anyhow!("the door stopped: {e}")),
        },
    };
    link.close().await;
    result
}
