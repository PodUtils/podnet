//! The direct way out: TCP from this host with no proxy, and the UDP socket
//! of the system.

use std::net::SocketAddr;
use std::sync::Arc;

use tokio::time::Instant;

use crate::network::{
    BoxFuture, Connected, Datagrams, Dial, Failure, Stage, SystemDatagrams, Target,
};

/// TCP with no proxy ([`crate::egress::dial_direct`]: the resolver chain of
/// podnet, and the race of the addresses), and UDP on an IPv4 socket of
/// the system.
#[derive(Clone, Copy, Debug, Default)]
pub struct Direct;

impl Dial for Direct {
    fn label(&self) -> String {
        "direct".to_string()
    }

    fn dial<'a>(
        &'a self,
        target: &'a Target,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(async move {
            let limit = deadline.saturating_duration_since(Instant::now());
            let stream = crate::egress::dial_direct(&target.host(), target.port(), limit)
                .await
                .map_err(|e| super::failure(e, Stage::Connect))?;
            let peer = stream.peer_addr().ok();
            let local = stream.local_addr().ok();
            Ok(Connected {
                io: Box::new(stream),
                peer,
                local,
            })
        })
    }

    /// A UDP socket of the system on an IPv4 address and any port. The
    /// [`SystemDatagrams`] of `pod-core` sends to the address as it is, so
    /// a socket of one family sends to that family only.
    fn datagrams<'a>(&'a self) -> BoxFuture<'a, Result<Arc<dyn Datagrams>, Failure>> {
        Box::pin(async move {
            let any = SocketAddr::from((std::net::Ipv4Addr::UNSPECIFIED, 0));
            let socket = SystemDatagrams::bind(any)
                .await
                .map_err(|e| Failure::io(Stage::Connect, e))?;
            Ok(Arc::new(socket) as Arc<dyn Datagrams>)
        })
    }
}
