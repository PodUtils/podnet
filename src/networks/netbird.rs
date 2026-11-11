//! NetBird in netstack mode: TCP to a peer through the SOCKS5 listener of
//! the daemon on 127.0.0.1 (T-246).
//!
//! The listener takes TCP and the address of a peer. It has no DNS and no
//! authentication.

use std::time::Duration;

use podnet_vnet::dialer::UpstreamChain;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::time::Instant;

use crate::network::{BoxFuture, Class, Connected, Dial, Failure, Stage, Target};

/// The port of the listener when `NB_SOCKS5_LISTENER_PORT` is not set.
pub const DEFAULT_PORT: u16 = 1080;

/// The way into NetBird through the listener on `127.0.0.1:PORT`.
pub struct NetbirdDialer {
    port: u16,
    chain: UpstreamChain,
}

impl NetbirdDialer {
    /// A dialer for the listener on `127.0.0.1:port`.
    pub fn new(port: u16) -> anyhow::Result<NetbirdDialer> {
        let hop = format!("127.0.0.1:{port}");
        Ok(NetbirdDialer {
            port,
            chain: UpstreamChain::parse(&[hop.as_str()])?,
        })
    }
}

impl Dial for NetbirdDialer {
    fn label(&self) -> String {
        format!("netbird:{}", self.port)
    }

    fn dial<'a>(
        &'a self,
        target: &'a Target,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(async move {
            if let Target::Name { host, .. } = target {
                return Err(Failure::new(
                    Stage::Dns,
                    Class::Unsupported,
                    format!(
                        "{host}: NetBird netstack mode has no DNS and takes the IP address of a peer only"
                    ),
                ));
            }
            self.chain.dial(target, deadline).await
        })
    }
}

/// Send the SOCKS5 greeting (no authentication) to `127.0.0.1:port`. The
/// answer is a line for `podnet net doctor`, or the reason of a failure.
pub async fn greeting(port: u16) -> Result<String, String> {
    let run = async {
        let mut io = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .map_err(|e| format!("127.0.0.1:{port} does not accept: {e}"))?;
        io.write_all(&[5, 1, 0])
            .await
            .map_err(|e| format!("127.0.0.1:{port}: {e}"))?;
        let mut reply = [0u8; 2];
        io.read_exact(&mut reply)
            .await
            .map_err(|e| format!("127.0.0.1:{port} gives no SOCKS5 reply: {e}"))?;
        match reply {
            [5, 0] => Ok(format!(
                "127.0.0.1:{port} answers the SOCKS5 greeting (TCP, peers by IP address)"
            )),
            other => Err(format!(
                "127.0.0.1:{port} is not a SOCKS5 listener without authentication (reply {other:02x?})"
            )),
        }
    };
    tokio::time::timeout(Duration::from_secs(3), run)
        .await
        .unwrap_or_else(|_| Err(format!("127.0.0.1:{port} gives no reply in 3 s")))
}

/// The port of the listener: `NB_SOCKS5_LISTENER_PORT`, else the default.
pub fn port_from_env() -> u16 {
    std::env::var("NB_SOCKS5_LISTENER_PORT")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .filter(|p| *p != 0)
        .unwrap_or(DEFAULT_PORT)
}
