//! The DNS forwarder of `podnet warp proxy --dns` (issue 117) with a bound
//! (T-215): at most [`MAX_DNS_TASKS`] queries are in flight, and a datagram
//! that is too short for a DNS header starts no task.

use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tracing::{debug, info};

use super::{forward_dns, STATS};
use crate::warp::ip::WarpStack;

/// The most DNS queries in flight at the same time. A query waits for a
/// session for up to 20 s, so an unbound burst would hold a task each.
pub(super) const MAX_DNS_TASKS: usize = 256;

/// The shortest DNS message: the header.
const DNS_HEADER: usize = 12;

/// The places of the DNS queries in flight.
pub(super) struct DnsGate(Arc<Semaphore>);

impl DnsGate {
    pub(super) fn new(limit: usize) -> Self {
        Self(Arc::new(Semaphore::new(limit)))
    }

    /// A place for a datagram of `len` bytes. `None` for a datagram under
    /// the header (it is no DNS message), and for an excess one (counted in
    /// `STATS.dns_dropped`).
    pub(super) fn admit(&self, len: usize) -> Option<OwnedSemaphorePermit> {
        if len < DNS_HEADER {
            debug!(len, "a DNS datagram is too short; dropped");
            return None;
        }
        match self.0.clone().try_acquire_owned() {
            Ok(permit) => Some(permit),
            Err(_) => {
                STATS.dns_dropped.fetch_add(1, Relaxed);
                None
            }
        }
    }
}

/// A DNS forwarder (issue 117): each query that comes to `socket` goes
/// through the WARP session of `live` to [`super::tunnel_resolver`], and the
/// answer goes back. For a program that sends its own DNS on UDP (a host
/// with a fake or broken resolver). Each query is one task, and the first
/// answer wins.
pub fn serve_dns(live: Arc<crate::warp::Live<WarpStack>>, socket: tokio::net::UdpSocket) {
    let socket = Arc::new(socket);
    if let Ok(addr) = socket.local_addr() {
        info!(%addr, "the DNS forwarder through WARP is listening");
    }
    let gate = DnsGate::new(MAX_DNS_TASKS);
    tokio::spawn(async move {
        let mut buf = vec![0u8; 4096];
        while let Ok((n, from)) = socket.recv_from(&mut buf).await {
            let Some(permit) = gate.admit(n) else {
                continue;
            };
            let query = buf[..n].to_vec();
            let socket = socket.clone();
            let live = live.clone();
            tokio::spawn(async move {
                let _permit = permit;
                let Ok(stack) = live.get().await else { return };
                match forward_dns(&stack, &query).await {
                    Ok(answer) => {
                        let _ = socket.send_to(&answer, from).await;
                    }
                    Err(err) => {
                        debug!(error = %format!("{err:#}"), "a DNS query through WARP failed")
                    }
                }
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_burst_gives_at_most_the_bound_in_flight() {
        let gate = DnsGate::new(4);
        let before = STATS.dns_dropped.load(Relaxed);
        let held: Vec<_> = (0..10).filter_map(|_| gate.admit(40)).collect();
        assert_eq!(held.len(), 4);
        assert_eq!(STATS.dns_dropped.load(Relaxed) - before, 6);
        // A finished query frees its place.
        drop(held);
        assert!(gate.admit(40).is_some());
    }

    #[test]
    fn a_short_datagram_starts_nothing() {
        let gate = DnsGate::new(1);
        assert!(gate.admit(1).is_none());
        assert!(gate.admit(11).is_none());
        // It took no place.
        assert!(gate.admit(12).is_some());
    }
}
