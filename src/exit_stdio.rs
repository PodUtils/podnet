//! `podnet exit --stdio` (T-329): one SOCKS5 session on one stream.
//!
//! The stream is stdin and stdout of the process, or any stream of a test.
//! The server side is the front door ([`podnet_netns::door::Door`], the code
//! of `podnet pod --serve`), with no listener. `CONNECT` is the stream
//! itself. `UDP ASSOCIATE` needs a UDP socket next to the control stream,
//! which a pipe does not have, so the door takes the command 0x83 of
//! `pod-proxy` (UDP in the stream): each datagram is a frame on the stream
//! (see `docs/proxies.md`, "UDP in the stream"). The command 3 is refused
//! with the reply 0x07: its UDP socket would be on this host, and the
//! client cannot reach it.
//!
//! The way out is the way of the process (`--proxy`, `--no-proxy`, the
//! variables, `--egress`). TCP follows it. UDP leaves from this host when
//! the process has no proxy; with a proxy it is refused (reply 0x07), as a
//! SOCKS5 or HTTP proxy of the process carries no UDP here.

use std::sync::Arc;

use anyhow::Result;
use pod_core::{BoxFuture, Connected, Datagrams, Dial, Failure, Stage, Target};
use podnet_netns::door::{Door, Duplex, Extension};
use tokio::io::AsyncWriteExt as _;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::time::Instant;

use crate::egress::ProxyChoice;

/// The way out of the exit: TCP through the way of the process, UDP from
/// this host when the process has no proxy.
pub struct ExitDial {
    tcp: Arc<dyn Dial>,
    udp: Option<Arc<dyn Dial>>,
}

impl ExitDial {
    /// The exit over the way of the process.
    pub fn process() -> ExitDial {
        let proxied = match crate::egress::choice() {
            ProxyChoice::Via(_) => true,
            ProxyChoice::Direct => false,
            ProxyChoice::FromEnvironment => crate::egress::proxy_for("udp.invalid", 53).is_some(),
        };
        let udp = (!proxied).then(|| Arc::new(crate::dialers::Direct) as Arc<dyn Dial>);
        ExitDial {
            tcp: crate::dialers::process(),
            udp,
        }
    }

    /// An exit over `tcp`, with the datagrams of `udp` (a test).
    pub fn with(tcp: Arc<dyn Dial>, udp: Option<Arc<dyn Dial>>) -> ExitDial {
        ExitDial { tcp, udp }
    }
}

impl Dial for ExitDial {
    fn label(&self) -> String {
        format!("exit {}", self.tcp.label())
    }

    fn names_inside(&self) -> bool {
        self.tcp.names_inside()
    }

    fn dial<'a>(
        &'a self,
        target: &'a Target,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        self.tcp.dial(target, deadline)
    }

    fn datagrams<'a>(&'a self) -> BoxFuture<'a, Result<Arc<dyn Datagrams>, Failure>> {
        match &self.udp {
            Some(udp) => udp.datagrams(),
            None => Box::pin(async move {
                Err(Failure::new(
                    Stage::Connect,
                    pod_core::Class::Unavailable,
                    "the way out of this process carries no UDP",
                ))
            }),
        }
    }
}

/// The refusal of `UDP ASSOCIATE` (command 3) on a stream.
struct NoAssociate;

impl Extension for NoAssociate {
    fn command<'a>(
        &'a self,
        command: u8,
        control: &'a mut dyn Duplex,
    ) -> BoxFuture<'a, Result<bool>> {
        Box::pin(async move {
            if command != 3 {
                return Ok(false);
            }
            control
                .write_all(&[0x05, 0x07, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
                .await?;
            control.flush().await?;
            Ok(true)
        })
    }
}

/// Serve one SOCKS5 session on `stream` over `dial` until it ends.
pub async fn serve<S>(stream: S, dial: Arc<dyn Dial>) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    let door = Door::new(dial)
        .connect_only()
        .with_extension(Arc::new(NoAssociate))
        .with_udp_in_stream();
    door.handle(stream).await
}

#[cfg(test)]
#[path = "exit_stdio_tests.rs"]
mod tests;
