//! A group of links (T-288): the members are `peer:FINGERPRINT/SERVICE`
//! ways, and the group is a [`Balance`] of the router engine. The pick,
//! the pin of a stream to its member, the policies, the health steps and
//! the kill switch are the code of the router (`network::balance`,
//! `policy`, `health`, `meter`). This module adds the limits of a link
//! group and the probes of its members.
//!
//! * A group has 1 to 16 members. A weight is 1 to 100. A probe interval
//!   is 1 to 60 s.
//! * A flow takes one member when it opens, and the stream stays on it.
//! * The policies are `weighted` (default), `rr`, `minrtt` and
//!   `leastload`. `protect` is also a policy of the router engine.
//! * The control probe of a member asks the control socket whether the
//!   link to the peer is up. A member that fails it is `Down` at once.
//! * The data probe of a member opens one stream to its service. A member
//!   that fails it is `Down` after `down_after` failures in a row
//!   (default 4).
//! * When each member is down, a flow is refused with the class `Policy`.
//!   There is no direct fallback.

use std::sync::Arc;
use std::time::Duration;

use tokio::time::Instant;

use crate::network::{probe, Balance, Class, Failure, Member, Policy, Stage, Thresholds};

/// The most members of a link group.
pub const MAX_MEMBERS: usize = 16;
/// The greatest weight of a member. The least is 1.
pub const MAX_WEIGHT: u32 = 100;
/// The least probe interval, in seconds.
pub const MIN_INTERVAL: u64 = 1;
/// The greatest probe interval, in seconds.
pub const MAX_INTERVAL: u64 = 60;

fn usage(text: String) -> Failure {
    Failure::new(Stage::Connect, Class::Usage, text)
}

/// Check the members of a link group: 1 to 16, each weight 1 to 100.
pub fn check_limits(members: &[Member]) -> Result<(), Failure> {
    if members.is_empty() || members.len() > MAX_MEMBERS {
        return Err(usage(format!(
            "a link group has 1 to {MAX_MEMBERS} members, not {}",
            members.len()
        )));
    }
    match members
        .iter()
        .find(|m| !(1..=MAX_WEIGHT).contains(&m.weight))
    {
        None => Ok(()),
        Some(m) => Err(usage(format!(
            "the weight of {} is 1 to {MAX_WEIGHT}, not {}",
            m.name, m.weight
        ))),
    }
}

/// Check the thresholds of a link group: the interval is 1 to 60 s, and
/// the steps keep K > R > 1 and R > K/2.
pub fn check_thresholds(t: &Thresholds) -> Result<(), Failure> {
    let seconds = t.interval.as_secs();
    if t.interval.subsec_nanos() != 0 || !(MIN_INTERVAL..=MAX_INTERVAL).contains(&seconds) {
        return Err(usage(format!(
            "the probe interval of a link group is {MIN_INTERVAL} to {MAX_INTERVAL} s"
        )));
    }
    t.check()
}

/// A link group and its probes.
pub struct LinkGroup {
    group: Arc<Balance>,
    peers: Vec<String>,
    thresholds: Thresholds,
}

impl LinkGroup {
    /// The group `name`. Each member is `(SPEC, WEIGHT)`, a SPEC being
    /// `peer:FINGERPRINT/SERVICE`. A member exists while its link is down.
    pub fn new(
        name: &str,
        members: &[(String, u32)],
        policy: Policy,
        thresholds: Thresholds,
    ) -> Result<LinkGroup, Failure> {
        check_thresholds(&thresholds)?;
        let mut built = Vec::new();
        let mut peers = Vec::new();
        for (spec, weight) in members {
            let network = crate::networks::parse(spec)?;
            let Some((fingerprint, service)) = network.peer() else {
                return Err(usage(format!(
                    "{spec}: a link group takes peer:FINGERPRINT/SERVICE members"
                )));
            };
            if built.iter().any(|m: &Member| m.name == *spec) {
                return Err(usage(format!("{spec}: the member is there twice")));
            }
            built.push(Member::new(
                spec,
                crate::networks::peer_way(fingerprint, service),
                *weight,
            ));
            peers.push(fingerprint.to_string());
        }
        check_limits(&built)?;
        let group = Balance::peers(name, built)?.with_policy(policy);
        Ok(LinkGroup {
            group: Arc::new(group),
            peers,
            thresholds,
        })
    }

    /// The group of the router engine. `group().way()` is its peer way.
    pub fn group(&self) -> &Arc<Balance> {
        &self.group
    }

    /// One round of probes. A member that fails its control probe is
    /// `Down` at once. Else its data probe steps its health. The round
    /// ends with the move of the `minrtt` head.
    pub async fn probe_round(&self) {
        let t = self.thresholds;
        let target = crate::network::peer("probe");
        for (i, member) in self.group.members().iter().enumerate() {
            let control =
                tokio::time::timeout(t.timeout, crate::link_ctl::probe_peer(&self.peers[i]))
                    .await
                    .is_ok_and(|r| r.is_ok());
            if !control {
                self.group.drop_member(i);
                continue;
            }
            let result = probe(&member.way, &target, t.timeout).await;
            self.group.observe(i, result, true, Instant::now(), &t);
        }
        self.group.end_round();
    }

    /// Probe each interval, until the end of the task.
    pub async fn probe_forever(&self) {
        loop {
            self.probe_round().await;
            tokio::time::sleep(self.thresholds.interval).await;
        }
    }
}

/// The thresholds of a link group with the interval `seconds`.
pub fn thresholds(seconds: u64) -> Thresholds {
    Thresholds {
        interval: Duration::from_secs(seconds),
        ..Thresholds::default()
    }
}
