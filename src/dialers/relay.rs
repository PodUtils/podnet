//! The relay as a way out: TCP to an IPv4 address through the WebSocket
//! relay ([`Relay::connect`]).

use std::net::SocketAddr;
use std::sync::Arc;

use pod_core::Resolve;
use tokio::time::Instant;

use crate::network::{BoxFuture, Class, Connected, Dial, Failure, Stage, Target};
use crate::ws_relay::Relay;

/// TCP through the relay. The relay takes an IPv4 address and a port. The
/// dialer resolves a name on this host with its resolver, and tries the
/// IPv4 addresses in their order. UDP gives the default error.
pub struct RelayDialer {
    relay: Arc<Relay>,
    resolve: Arc<dyn Resolve>,
}

impl RelayDialer {
    /// A dialer through `relay`, which resolves names with `resolve`.
    pub fn new(relay: Arc<Relay>, resolve: Arc<dyn Resolve>) -> RelayDialer {
        RelayDialer { relay, resolve }
    }

    /// A dialer through `relay`, which resolves names with the resolver
    /// chain of the process ([`crate::egress::resolver`]).
    pub fn of_process(relay: Arc<Relay>) -> RelayDialer {
        RelayDialer::new(relay, crate::egress::resolver())
    }
}

impl Dial for RelayDialer {
    fn label(&self) -> String {
        format!("relay {}", self.relay.url())
    }

    fn dial<'a>(
        &'a self,
        target: &'a Target,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(async move {
            let (host, port) = match target {
                Target::Addr(addr) => {
                    let limit = deadline.saturating_duration_since(Instant::now());
                    let io = self
                        .relay
                        .connect(*addr, limit)
                        .await
                        .map_err(|e| super::failure(e, Stage::Relay))?;
                    return Ok(Connected {
                        io: Box::new(io),
                        peer: Some(*addr),
                        local: None,
                    });
                }
                Target::Name { host, port } => (host, *port),
            };
            let addrs = self.resolve.resolve(host, port, deadline).await?;
            let v4: Vec<SocketAddr> = addrs.into_iter().filter(SocketAddr::is_ipv4).collect();
            if v4.is_empty() {
                return Err(Failure::new(
                    Stage::Dns,
                    Class::NotFound,
                    format!("{host} has no IPv4 address, and the relay has no IPv6 egress"),
                ));
            }
            self.dial_addrs(host, &v4, deadline).await
        })
    }
}
