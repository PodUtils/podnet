//! The probe of the WANs of the kernel mode (T-120, part 4). Each interval,
//! one TCP connect to `health.target` leaves through each WAN: the socket
//! has the fwmark of the WAN, so the rule `fwmark M lookup T` sends it into
//! the table of that WAN, not into the aggregate route. A WAN whose link is
//! up but whose session is dead (a TUN of `warp tun` with no answer) fails
//! its probe, because the probe crosses the session.
//!
//! The steps of each WAN are those of a member of a group (`Tracker`, T-097):
//! K misses in a row take a WAN down, R passes in a row take it back. The
//! control of a round is "one WAN or more passed": when each WAN fails, the
//! fault is the host or the target, and no WAN goes down. A probe that times
//! out is a clean miss here: a dead session gives no answer, and the control
//! keeps an outage of the host from a step down.
//!
//! The aggregate route holds each WAN that takes flows (`up`, `cooldown` or
//! `probation`). When no WAN takes flows, it holds each WAN: the link state
//! of the kernel and the kill switch decide. The tables of the WANs do not
//! change, so the probe of a WAN that is down goes on.

use std::net::{SocketAddr, SocketAddrV4};
use std::os::fd::AsRawFd;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use tokio::time::Instant;

use super::{ifindex, new_route_request, NextHop, Plan, Socket};
use crate::network::{Health, ProbeResult, Thresholds, Tracker};

/// The probe of the WANs of one plan.
pub struct Watch {
    plan: Plan,
    target: SocketAddrV4,
    thresholds: Thresholds,
    trackers: Vec<Tracker>,
    /// The WANs that the aggregate route holds now.
    routed: Vec<bool>,
}

impl Watch {
    /// The probe of `plan` to `target`. [`super::apply`] wrote the plan, so
    /// the aggregate route holds each WAN. The probe needs `SO_MARK`
    /// (CAP_NET_ADMIN): an error names it.
    pub fn new(plan: Plan, target: SocketAddrV4, thresholds: Thresholds) -> Result<Watch> {
        let socket = tokio::net::TcpSocket::new_v4().context("kernel probe: socket")?;
        set_mark(&socket, 1).context("kernel probe: setsockopt SO_MARK")?;
        let n = plan.wans.len();
        Ok(Watch {
            plan,
            target,
            thresholds,
            trackers: vec![Tracker::default(); n],
            routed: vec![true; n],
        })
    }

    /// The name and the health of each WAN.
    pub fn health(&self) -> Vec<(String, Health)> {
        let names = self.plan.wans.iter().map(|w| w.name.clone());
        names
            .zip(self.trackers.iter().map(Tracker::health))
            .collect()
    }

    /// The names of the WANs that the aggregate route holds.
    pub fn routed(&self) -> Vec<String> {
        let wans = self.plan.wans.iter().zip(&self.routed);
        wans.filter(|(_, r)| **r)
            .map(|(w, _)| w.name.clone())
            .collect()
    }

    /// One round at `now`: probe each WAN at the same time, take each step,
    /// and write the aggregate route when the set of WANs changes. True when
    /// the route changed.
    pub async fn round(&mut self, now: Instant) -> Result<bool> {
        let timeout = self.thresholds.timeout;
        let probes = self
            .plan
            .wans
            .iter()
            .map(|w| probe(w.mark, self.target, timeout));
        let results = futures::future::join_all(probes).await;
        let control = results.iter().any(|r| matches!(r, ProbeResult::Ok(_)));
        for (tracker, result) in self.trackers.iter_mut().zip(&results) {
            tracker.observe(*result, control, now, &self.thresholds);
        }
        let mut want: Vec<bool> = self
            .trackers
            .iter()
            .map(|t| t.health().takes_flows())
            .collect();
        if !want.contains(&true) {
            want = vec![true; want.len()];
        }
        if want == self.routed {
            return Ok(false);
        }
        write_aggregate(&self.plan, &want)?;
        self.routed = want;
        Ok(true)
    }

    /// Probe each interval until an error. Print a line at each change.
    pub async fn run(mut self) -> Result<()> {
        loop {
            if self.round(Instant::now()).await? {
                let health: Vec<String> = self
                    .health()
                    .iter()
                    .map(|(n, h)| format!("{n} {}", h.name()))
                    .collect();
                println!(
                    "podnet: route  kernel  WANs: {}; the aggregate route holds {}",
                    health.join(", "),
                    self.routed().join(", ")
                );
            }
            tokio::time::sleep(self.thresholds.interval).await;
        }
    }
}

/// The IPv4 address of `target` (HOST:PORT) for the probe of the WANs. A
/// name is resolved one time, with the resolver of the host.
pub async fn resolve_target(target: &str) -> Result<SocketAddrV4> {
    let found = tokio::net::lookup_host(target)
        .await
        .with_context(|| format!("health.target {target}"))?;
    for addr in found {
        if let SocketAddr::V4(v4) = addr {
            return Ok(v4);
        }
    }
    bail!("health.target {target}: the kernel mode probes IPv4 only; give an IPv4 address")
}

fn set_mark(socket: &tokio::net::TcpSocket, mark: u32) -> std::io::Result<()> {
    // SAFETY: setsockopt with a u32 that lives for the call.
    let rc = unsafe {
        libc::setsockopt(
            socket.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_MARK,
            std::ptr::from_ref(&mark).cast(),
            std::mem::size_of::<u32>() as libc::socklen_t,
        )
    };
    match rc {
        0 => Ok(()),
        _ => Err(std::io::Error::last_os_error()),
    }
}

/// A TCP connect to `target` with the fwmark `mark`, closed at once.
async fn probe(mark: u32, target: SocketAddrV4, timeout: Duration) -> ProbeResult {
    let start = Instant::now();
    let Ok(socket) = tokio::net::TcpSocket::new_v4() else {
        return ProbeResult::ConnectFailed { clean: false };
    };
    if set_mark(&socket, mark).is_err() {
        return ProbeResult::ConnectFailed { clean: false };
    }
    match tokio::time::timeout(timeout, socket.connect(target.into())).await {
        Ok(Ok(stream)) => {
            drop(stream);
            ProbeResult::Ok(start.elapsed())
        }
        // A refusal, an unreachable host or a time out: a clean miss.
        Ok(Err(_)) | Err(_) => ProbeResult::ConnectFailed { clean: true },
    }
}

/// Write the aggregate route of `plan` with the WANs of `want`.
fn write_aggregate(plan: &Plan, want: &[bool]) -> Result<()> {
    let mut hops: Vec<NextHop> = Vec::new();
    for (w, _) in plan.wans.iter().zip(want).filter(|(_, on)| **on) {
        hops.push((ifindex(&w.dev)?, w.gateway, w.weight));
    }
    let sock = Socket::open()?;
    let what = format!("route default table {}", plan.aggregate);
    super::check(
        &what,
        sock.ack(&new_route_request(plan.aggregate, &hops)),
        &[],
    )
}
