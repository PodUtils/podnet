//! The attach of a stack to a switch: the connect, its time limit (#768)
//! and the hello of a TCP switch.

use anyhow::{Context, Result};
use podnet_netns::listen::ProxyListen;
use tokio::task::JoinHandle;

use super::{give_hello, pump, Faults, TOKEN_VARIABLE};
use crate::stack::NetStack;

/// The time limit of the TCP connect to a switch, as the other dials of
/// podnet have (#768).
pub const SWITCH_CONNECT: std::time::Duration = std::time::Duration::from_secs(20);

/// Connect `stack` to the switch at `switch`. The returned task moves the
/// packets until the connection closes. A TCP switch needs its `token`
/// (issue 309); a unix switch needs none.
pub async fn attach(
    stack: &NetStack,
    switch: &ProxyListen,
    faults: Faults,
    token: Option<&str>,
) -> Result<JoinHandle<()>> {
    attach_within(stack, switch, faults, token, SWITCH_CONNECT).await
}

/// [`attach`] with the time limit `limit` of the TCP connect.
pub async fn attach_within(
    stack: &NetStack,
    switch: &ProxyListen,
    faults: Faults,
    token: Option<&str>,
    limit: std::time::Duration,
) -> Result<JoinHandle<()>> {
    let link_in = stack.link_in();
    let link_out = stack.link_out();
    Ok(match switch {
        ProxyListen::Tcp(addr) => {
            let Some(token) = token else {
                anyhow::bail!(
                    "a TCP switch needs its token: set {TOKEN_VARIABLE}, or give --switch-token-file"
                );
            };
            // #768: a switch that does not answer ends the dial at a time
            // limit of its own, not at the limit of the kernel (135 s).
            let mut stream = tokio::time::timeout(limit, tokio::net::TcpStream::connect(addr))
                .await
                .map_err(|_| {
                    anyhow::anyhow!(
                        "connecting to the switch at tcp://{addr}: no answer in {} s",
                        limit.as_secs_f64()
                    )
                })?
                .with_context(|| format!("connecting to the switch at tcp://{addr}"))?;
            give_hello(&mut stream, token)
                .await
                .with_context(|| format!("the switch at tcp://{addr}"))?;
            tokio::spawn(pump(link_in, link_out, stream, faults))
        }
        ProxyListen::Unix(path) => {
            let stream = tokio::net::UnixStream::connect(path)
                .await
                .with_context(|| format!("connecting to the switch at unix:{}", path.display()))?;
            tokio::spawn(pump(link_in, link_out, stream, faults))
        }
    })
}
