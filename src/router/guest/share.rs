//! The share of the flows of a guest (T-345). Each source address of the
//! guest is in one group: by default each address is its own group, and
//! `podnet wg serve` puts the sources of one peer in one group. A group has
//! at most its share of the TCP flows (held SYNs, released SYNs and open
//! flows) and of the UDP flows of the guest. A new flow over the share is
//! refused and counted (`share_refused`). The log gets one line for each
//! group in each [`LOG_EVERY`], not one line for each refused flow.

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::{GuestStats, MAX_FLOWS};

/// The time between two log lines of one group.
pub const LOG_EVERY: Duration = Duration::from_secs(10);
/// The groups with a recent log line that the table keeps.
const LOGGED_MOST: usize = 4096;
/// The highest total of TCP flows and of UDP flows that a setting can give.
pub const FLOWS_MOST: usize = 65536;

/// The limits of the flows of a guest: the totals, and the share of one
/// group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FlowLimits {
    /// TCP flows of the guest at one time.
    pub tcp: usize,
    /// UDP flows of the guest at one time.
    pub udp: usize,
    /// TCP flows of one group at one time.
    pub tcp_share: usize,
    /// UDP flows of one group at one time.
    pub udp_share: usize,
}

impl Default for FlowLimits {
    /// The totals of [`MAX_FLOWS`] and [`super::udp::MAX_FLOWS`]; one group
    /// can take all of them.
    fn default() -> Self {
        FlowLimits {
            tcp: MAX_FLOWS,
            udp: super::udp::MAX_FLOWS,
            tcp_share: MAX_FLOWS,
            udp_share: super::udp::MAX_FLOWS,
        }
    }
}

impl FlowLimits {
    /// The limits with each value that is `None` set to its default. The
    /// default share of `groups` groups is the total divided by the groups,
    /// at least 64 TCP flows and 16 UDP flows (not more than the total).
    pub fn with(
        tcp: Option<usize>,
        udp: Option<usize>,
        tcp_share: Option<usize>,
        udp_share: Option<usize>,
        groups: usize,
    ) -> Result<FlowLimits, String> {
        let tcp = tcp.unwrap_or(MAX_FLOWS);
        let udp = udp.unwrap_or(super::udp::MAX_FLOWS);
        let groups = groups.max(1);
        let limits = FlowLimits {
            tcp,
            udp,
            tcp_share: tcp_share.unwrap_or((tcp / groups).max(64).min(tcp)),
            udp_share: udp_share.unwrap_or((udp / groups).max(16).min(udp)),
        };
        limits.check()?;
        Ok(limits)
    }

    /// A fault when a value is 0, a total is over [`FLOWS_MOST`], or a share
    /// is over its total.
    pub fn check(&self) -> Result<(), String> {
        for (name, value, most) in [
            ("the TCP flows", self.tcp, FLOWS_MOST),
            ("the UDP flows", self.udp, FLOWS_MOST),
            ("the TCP share", self.tcp_share, self.tcp),
            ("the UDP share", self.udp_share, self.udp),
        ] {
            if value == 0 || value > most {
                return Err(format!("{name}: give 1 to {most}, not {value}"));
            }
        }
        Ok(())
    }
}

/// The groups of the sources of a guest.
pub trait Groups: Send + Sync {
    /// The group of the source `src`.
    fn group(&self, src: Ipv4Addr) -> u32;
    /// The name of a group, for a log line.
    fn name(&self, group: u32) -> String;
}

/// Each source address is a group.
pub struct BySource;

impl Groups for BySource {
    fn group(&self, src: Ipv4Addr) -> u32 {
        u32::from(src)
    }

    fn name(&self, group: u32) -> String {
        format!("the source {}", Ipv4Addr::from(group))
    }
}

/// A TCP flow or a UDP flow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Tcp,
    Udp,
}

impl Kind {
    fn at(self) -> usize {
        match self {
            Kind::Tcp => 0,
            Kind::Udp => 1,
        }
    }
}

/// The flows of each group, and the time of the last log line of each.
pub struct Shares {
    groups: Arc<dyn Groups>,
    limits: FlowLimits,
    counts: Mutex<HashMap<u32, [usize; 2]>>,
    /// The time of the last line of a group, and the flows refused since.
    logged: Mutex<HashMap<u32, (Instant, u64)>>,
}

/// One flow of a group. The drop gives its place back.
pub struct Held {
    shares: Arc<Shares>,
    group: u32,
    kind: Kind,
}

impl Drop for Held {
    fn drop(&mut self) {
        let mut counts = self.shares.counts();
        if let Some(count) = counts.get_mut(&self.group) {
            count[self.kind.at()] = count[self.kind.at()].saturating_sub(1);
            if *count == [0, 0] {
                counts.remove(&self.group);
            }
        }
    }
}

impl Shares {
    pub fn new(groups: Arc<dyn Groups>, limits: FlowLimits) -> Arc<Shares> {
        Arc::new(Shares {
            groups,
            limits,
            counts: Mutex::new(HashMap::new()),
            logged: Mutex::new(HashMap::new()),
        })
    }

    pub fn limits(&self) -> FlowLimits {
        self.limits
    }

    fn counts(&self) -> std::sync::MutexGuard<'_, HashMap<u32, [usize; 2]>> {
        self.counts.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn share(&self, kind: Kind) -> usize {
        match kind {
            Kind::Tcp => self.limits.tcp_share,
            Kind::Udp => self.limits.udp_share,
        }
    }

    /// The flows of `kind` that the group of `src` has now.
    pub fn count(&self, src: Ipv4Addr, kind: Kind) -> usize {
        let group = self.groups.group(src);
        self.counts().get(&group).map_or(0, |c| c[kind.at()])
    }

    /// A place for a new flow of `src`, or `None` when its group is at its
    /// share: then the flow is counted in `stats` and the log can get a line.
    pub fn take(self: &Arc<Self>, src: Ipv4Addr, kind: Kind, stats: &GuestStats) -> Option<Held> {
        let group = self.groups.group(src);
        {
            let mut counts = self.counts();
            let count = counts.entry(group).or_insert([0, 0]);
            if count[kind.at()] < self.share(kind) {
                count[kind.at()] += 1;
                return Some(self.held(group, kind));
            }
            if *count == [0, 0] {
                counts.remove(&group);
            }
        }
        stats.add(&stats.share_refused);
        self.refused(group, kind, Instant::now());
        None
    }

    /// A place for a flow of `src` that the stack already took: it is
    /// counted, also over the share.
    pub fn force(self: &Arc<Self>, src: Ipv4Addr, kind: Kind) -> Held {
        let group = self.groups.group(src);
        self.counts().entry(group).or_insert([0, 0])[kind.at()] += 1;
        self.held(group, kind)
    }

    fn held(self: &Arc<Self>, group: u32, kind: Kind) -> Held {
        Held {
            shares: self.clone(),
            group,
            kind,
        }
    }

    /// Write one line for the group in each [`LOG_EVERY`]. True when this
    /// refusal wrote the line.
    pub(super) fn refused(&self, group: u32, kind: Kind, now: Instant) -> bool {
        let mut logged = self.logged.lock().unwrap_or_else(|e| e.into_inner());
        let since = match logged.get_mut(&group) {
            Some((at, missed)) if now.saturating_duration_since(*at) < LOG_EVERY => {
                *missed += 1;
                return false;
            }
            Some((_, missed)) => *missed,
            None => 0,
        };
        if logged.len() >= LOGGED_MOST {
            logged.retain(|_, (at, _)| now.saturating_duration_since(*at) < LOG_EVERY);
        }
        if logged.len() < LOGGED_MOST {
            logged.insert(group, (now, 0));
        }
        let (what, share) = match kind {
            Kind::Tcp => ("TCP", self.limits.tcp_share),
            Kind::Udp => ("UDP", self.limits.udp_share),
        };
        tracing::warn!(
            "{} is at its share of {share} {what} flows: a new flow is refused ({since} more \
             refused since the last line)",
            self.groups.name(group)
        );
        true
    }
}
