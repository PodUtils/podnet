//! Address-family policy and socket-attempt tracking for MASQUE (QUIC-1).

use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, Mutex, PoisonError};

use pod_core::{BoxFuture, Class, Datagrams, Failure, Stage, SystemDatagrams};
use pod_warp::UdpWay;

use super::error::MasqueError;

/// Address-family policy for MASQUE UDP sockets.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum IpPolicy {
    /// Allow IPv4 and IPv6 endpoints.
    DualStack,
    /// Refuse all IPv4 socket binds and connects before any system call.
    Ipv6Only,
    /// Refuse remote IPv4 endpoints when the host has no IPv4 default route.
    #[default]
    Auto,
}

impl IpPolicy {
    /// Return true when `addr` is permitted under this policy.
    pub fn permits(self, addr: SocketAddr) -> bool {
        if addr.is_ipv6() {
            return true;
        }
        match self {
            Self::DualStack => true,
            Self::Ipv6Only => false,
            Self::Auto => addr.ip().is_loopback() || crate::resolve::has_default_route(false),
        }
    }
}

/// Kind of socket event recorded by [`AttemptTracker`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttemptKind {
    /// Local UDP socket bind.
    Bind,
    /// QUIC connect target selected for a bound socket.
    Connect,
    /// IPv4 target skipped or refused before any socket call.
    RefusedIpv4,
}

/// One recorded address-family or socket decision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SocketAttempt {
    pub kind: AttemptKind,
    pub addr: SocketAddr,
    pub detail: String,
}

/// Thread-safe log of UDP bind, connect, and family-refusal events.
#[derive(Clone, Debug, Default)]
pub struct AttemptTracker {
    entries: Arc<Mutex<Vec<SocketAttempt>>>,
}

impl AttemptTracker {
    /// Create an empty attempt tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Append one attempt record.
    pub fn record(&self, kind: AttemptKind, addr: SocketAddr, detail: impl Into<String>) {
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        entries.push(SocketAttempt {
            kind,
            addr,
            detail: detail.into(),
        });
    }

    /// Return all recorded events in order.
    pub fn attempts(&self) -> Vec<SocketAttempt> {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Return all IPv4 socket bind or connect attempts.
    pub fn ipv4_attempts(&self) -> Vec<SocketAttempt> {
        self.attempts()
            .into_iter()
            .filter(|e| {
                e.addr.is_ipv4() && matches!(e.kind, AttemptKind::Bind | AttemptKind::Connect)
            })
            .collect()
    }

    /// Return all IPv6 socket bind or connect attempts.
    pub fn ipv6_attempts(&self) -> Vec<SocketAttempt> {
        self.attempts()
            .into_iter()
            .filter(|e| {
                e.addr.is_ipv6() && matches!(e.kind, AttemptKind::Bind | AttemptKind::Connect)
            })
            .collect()
    }

    /// Return all IPv4 targets refused before any socket call.
    pub fn refused_ipv4(&self) -> Vec<SocketAttempt> {
        self.attempts()
            .into_iter()
            .filter(|e| e.kind == AttemptKind::RefusedIpv4)
            .collect()
    }

    /// Format recorded events as human-readable lines.
    pub fn lines(&self) -> Vec<String> {
        self.attempts()
            .into_iter()
            .map(|e| format!("{:?} {}: {}", e.kind, e.addr, e.detail))
            .collect()
    }
}

/// Filter candidate endpoints under `policy` before any socket call.
pub fn select_endpoints(
    candidates: &[SocketAddr],
    policy: IpPolicy,
    tracker: &AttemptTracker,
) -> Result<Vec<SocketAddr>, MasqueError> {
    let mut allowed = Vec::new();
    for &addr in candidates {
        if policy.permits(addr) {
            allowed.push(addr);
        } else {
            tracker.record(
                AttemptKind::RefusedIpv4,
                addr,
                "IPv4 target skipped under IPv6-only policy",
            );
        }
    }
    if allowed.is_empty() {
        return Err(MasqueError::transport(
            "no permitted MASQUE endpoint remains under the active IP policy (zero IPv4 attempts made)",
        ));
    }
    Ok(allowed)
}

/// A `UdpWay` that enforces [`IpPolicy`] and records every bind and connect.
#[derive(Clone, Debug)]
pub struct TrackedUdp {
    policy: IpPolicy,
    tracker: AttemptTracker,
}

impl TrackedUdp {
    /// Build a tracked UDP factory with `policy` and `tracker`.
    pub fn new(policy: IpPolicy, tracker: AttemptTracker) -> Self {
        Self { policy, tracker }
    }

    /// Return the local unspecified bind address for `door`.
    pub fn bind_addr_for(door: SocketAddr) -> SocketAddr {
        match door {
            SocketAddr::V4(_) => SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)),
            SocketAddr::V6(_) => SocketAddr::from((Ipv6Addr::UNSPECIFIED, 0)),
        }
    }
}

impl UdpWay for TrackedUdp {
    fn label(&self) -> String {
        match self.policy {
            IpPolicy::Ipv6Only => "direct UDP (IPv6-only)".into(),
            IpPolicy::DualStack => "direct UDP (dual-stack)".into(),
            IpPolicy::Auto => "direct UDP".into(),
        }
    }

    fn datagrams<'a>(
        &'a self,
        door: SocketAddr,
    ) -> BoxFuture<'a, Result<Arc<dyn Datagrams>, Failure>> {
        Box::pin(async move {
            if !self.policy.permits(door) {
                self.tracker.record(
                    AttemptKind::RefusedIpv4,
                    door,
                    "IPv4 socket refused before bind",
                );
                return Err(Failure::new(
                    Stage::Connect,
                    Class::Unavailable,
                    format!("{door}: IPv4 endpoint refused on an IPv6-only host"),
                ));
            }
            let bind_addr = Self::bind_addr_for(door);
            self.tracker.record(
                AttemptKind::Bind,
                bind_addr,
                format!("bind UDP socket for {door}"),
            );
            let socket = SystemDatagrams::bind(bind_addr).await.map_err(|e| {
                Failure::caused(
                    Stage::Connect,
                    Class::of_io(e.kind()),
                    format!("bind UDP {bind_addr} for {door}: {e}"),
                    e,
                )
            })?;
            self.tracker
                .record(AttemptKind::Connect, door, "QUIC target selected");
            Ok(Arc::new(socket) as Arc<dyn Datagrams>)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipv6_only_policy_skips_ipv4_without_socket_attempts() {
        let tracker = AttemptTracker::new();
        let v4: SocketAddr = "127.0.0.1:443".parse().unwrap();
        let v6: SocketAddr = "[::1]:443".parse().unwrap();
        let allowed = select_endpoints(&[v4, v6], IpPolicy::Ipv6Only, &tracker).unwrap();
        assert_eq!(allowed, vec![v6]);
        assert!(tracker.ipv4_attempts().is_empty());
        assert_eq!(tracker.refused_ipv4().len(), 1);
        assert_eq!(TrackedUdp::bind_addr_for(v6), "[::]:0".parse().unwrap());
    }

    #[tokio::test]
    async fn tracked_udp_refuses_ipv4_before_bind_when_ipv6_only() {
        let tracker = AttemptTracker::new();
        let udp = TrackedUdp::new(IpPolicy::Ipv6Only, tracker.clone());
        let err = udp
            .datagrams("127.0.0.1:443".parse().unwrap())
            .await
            .err()
            .expect("IPv4 target must fail under Ipv6Only");
        assert_eq!(err.class, Class::Unavailable);
        assert!(tracker.ipv4_attempts().is_empty());
        assert_eq!(tracker.refused_ipv4().len(), 1);
    }
}
