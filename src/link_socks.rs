//! The service `socks5` of a stream of a link (T-291).
//!
//! The peer that opened the stream speaks SOCKS5 on it. This end runs the
//! server side of the front door ([`podnet_netns::door::Door`], the same
//! code as `podnet pod --serve`) with a direct dial: a `CONNECT` goes from
//! this host, and a name resolves with the resolver of this host. The
//! allow list of the listener has admitted the peer before the stream
//! opened. The door takes SOCKS5 only here: a first byte that is not 5
//! ends the stream. Not built yet: `BIND`, `UDP ASSOCIATE`, `--resolve
//! local`.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Result};
use pod_core::{BoxFuture, Class, Connected, Dial, Failure, Stage, Target};
use podnet_netns::door::Door;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};
use tokio::time::Instant;

/// The name of the service.
pub const NAME: &str = "socks5";

/// The way out of the service: a TCP connection from this host.
struct Direct;

impl Dial for Direct {
    fn label(&self) -> String {
        "link socks5".to_string()
    }

    /// The door sends the name: this host resolves it.
    fn names_inside(&self) -> bool {
        true
    }

    fn dial<'a>(
        &'a self,
        target: &'a Target,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(async move {
            let limit = deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_secs(30));
            let host = target.host();
            let work = tokio::net::TcpStream::connect((host.as_str(), target.port()));
            let fail = |text: String| Failure::new(Stage::Connect, Class::Unavailable, text);
            match tokio::time::timeout(limit, work).await {
                Err(_) => Err(fail(format!("no answer from {target} in time"))),
                Ok(Err(e)) => Err(fail(format!("connecting to {target}: {e}"))),
                Ok(Ok(tcp)) => {
                    let _ = tcp.set_nodelay(true);
                    let (local, peer) = (tcp.local_addr().ok(), tcp.peer_addr().ok());
                    let mut done = Connected::new(Box::new(tcp));
                    done.local = local;
                    done.peer = peer;
                    Ok(done)
                }
            }
        })
    }
}

/// Serve SOCKS5 on `stream` until the flow ends. The first byte must be 5.
pub async fn serve<S>(stream: S) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    let (mut read, write) = tokio::io::split(stream);
    let mut first = [0u8; 1];
    read.read_exact(&mut first).await?;
    if !podnet_netns::socks::looks_like_socks5(first[0]) {
        bail!("the stream of the service {NAME} does not start with SOCKS5");
    }
    // The door reads the byte again: give it back in front of the stream.
    let again = tokio::io::join(std::io::Cursor::new(first).chain(read), write);
    let door = Door::new(Arc::new(Direct)).connect_only();
    door.handle(again).await
}
