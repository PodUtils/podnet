//! The resolver of a member (T-098, GitHub #374): a name that a flow brings
//! resolves through the network of the member that carries the flow, never
//! with the resolver of this host. The query is DNS over TCP (RFC 7766) to
//! an upstream at the far end of the network, over the dial of the member
//! itself. The upstream is operator input (the key `resolvers`): the router
//! names no resolver of its own.
//!
//! A way whose far end resolves names (`names_inside`, a SOCKS5h proxy or
//! the relay) gets the name as it is. A name that the network cannot resolve
//! is an error: the flow never falls back to this host.

use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use podnet_netns::dns::{build_query, parse_answers, TYPE_A, TYPE_AAAA};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::time::Instant;

use crate::network::{BoxFuture, Class, Connected, Datagrams, Dial, Failure, Stage, Target};

/// The time that the lookup of a name may take at most.
pub const QUERY_LIMIT: Duration = Duration::from_secs(10);
/// The most addresses that a flow tries.
const MAX_ADDRESSES: usize = 8;
/// The limit of one DNS message over TCP (RFC 1035: 64 KiB less one).
const MAX_MESSAGE: usize = 65535;

/// A dial that resolves the names of its flows through its own network.
pub struct Resolving {
    inner: Arc<dyn Dial>,
    upstream: Target,
}

fn dns(class: Class, text: String) -> Failure {
    Failure::new(Stage::Dns, class, text)
}

impl Resolving {
    /// `inner` with the DNS `upstream` at the far end of its network.
    pub fn new(inner: Arc<dyn Dial>, upstream: Target) -> Resolving {
        Resolving { inner, upstream }
    }

    /// The addresses of `host`: A and AAAA over one connection, IPv4 first.
    async fn lookup(&self, host: &str, deadline: Instant) -> Result<Vec<IpAddr>, Failure> {
        let deadline = deadline.min(Instant::now() + QUERY_LIMIT);
        let work = async {
            let mut conn = self.inner.dial(&self.upstream, deadline).await?;
            let mut found: Vec<IpAddr> = Vec::new();
            let mut missing = 0;
            for (id, kind) in [(0x5030u16, TYPE_A), (0x5031, TYPE_AAAA)] {
                let query = build_query(id, host, kind)
                    .map_err(|e| dns(Class::Usage, format!("{host}: {e}")))?;
                let io = &mut conn.io;
                let sent = async {
                    io.write_all(&(query.len() as u16).to_be_bytes()).await?;
                    io.write_all(&query).await?;
                    io.flush().await?;
                    let len = io.read_u16().await? as usize;
                    let mut message = vec![0u8; len.min(MAX_MESSAGE)];
                    io.read_exact(&mut message).await?;
                    Ok::<_, std::io::Error>(message)
                };
                let message = sent.await.map_err(|e| Failure::io(Stage::Dns, e))?;
                if message.len() < 12 || message[..2] != id.to_be_bytes() {
                    return Err(dns(
                        Class::Protocol,
                        format!(
                            "{host}: the answer of {} is not for the query",
                            self.upstream
                        ),
                    ));
                }
                if message[3] & 0x0f == 3 {
                    missing += 1;
                    continue;
                }
                found.extend(parse_answers(&message).unwrap_or_default());
            }
            if found.is_empty() {
                let what = if missing == 2 {
                    "no such name"
                } else {
                    "no address"
                };
                return Err(dns(
                    Class::NotFound,
                    format!("{host}: {what} through {}", self.inner.label()),
                ));
            }
            let (v4, v6): (Vec<IpAddr>, Vec<IpAddr>) =
                found.into_iter().partition(|ip| ip.is_ipv4());
            let mut all = v4;
            all.extend(v6);
            all.dedup();
            all.truncate(MAX_ADDRESSES);
            Ok(all)
        };
        tokio::time::timeout_at(deadline, work).await.map_err(|_| {
            dns(
                Class::Timeout,
                format!("{host}: the query through {} timed out", self.inner.label()),
            )
        })?
    }

    async fn connect(
        &self,
        target: &Target,
        deadline: Instant,
        http: bool,
    ) -> Result<Connected, Failure> {
        let reach = |t: Target| async move {
            if http {
                self.inner.dial_http(&t, deadline).await
            } else {
                self.inner.dial(&t, deadline).await
            }
        };
        let Target::Name { host, port } = target else {
            return reach(target.clone()).await;
        };
        if self.inner.names_inside() {
            return reach(target.clone()).await;
        }
        let mut last = None;
        for ip in self.lookup(host, deadline).await? {
            match reach(Target::Addr(std::net::SocketAddr::new(ip, *port))).await {
                Ok(conn) => return Ok(conn),
                Err(e) => last = Some(e),
            }
        }
        Err(last.unwrap_or_else(|| dns(Class::NotFound, format!("{host}: no address"))))
    }
}

impl Dial for Resolving {
    fn label(&self) -> String {
        self.inner.label()
    }

    fn dial<'a>(
        &'a self,
        target: &'a Target,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(self.connect(target, deadline, false))
    }

    fn dial_http<'a>(
        &'a self,
        target: &'a Target,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(self.connect(target, deadline, true))
    }

    /// The name resolves inside the network: through the upstream, or at
    /// the far end of the way.
    fn names_inside(&self) -> bool {
        true
    }

    fn datagrams<'a>(&'a self) -> BoxFuture<'a, Result<Arc<dyn Datagrams>, Failure>> {
        self.inner.datagrams()
    }
}

#[cfg(test)]
mod tests;
