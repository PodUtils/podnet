//! The probe of the router (T-097): each interval, one probe of each member
//! of each group, the egress check, and the renewal of a member that asks
//! for it.

use tokio::time::Instant;

use super::Router;
use crate::network::{probe, probe_egress};

impl Router {
    /// Probe each member of each group, each interval, until the end.
    pub async fn probe_forever(&self) {
        if self.health.is_none() {
            return std::future::pending().await;
        }
        loop {
            self.probe_round().await;
            self.control.pause().await;
        }
    }

    /// One round: probe each member of each group. The control of a group
    /// passes when one member or more reached the target: a failure of each
    /// member is an outage of the host. A member with an egress check
    /// (`health.egress`) also asks the echo endpoint for its address.
    pub async fn probe_round(&self) {
        let Some((target, thresholds)) = self.health.clone() else {
            return;
        };
        for (_, group) in &self.groups {
            let mut results = Vec::new();
            for member in group.members() {
                let mut result = probe(&member.way, &target, thresholds.timeout).await;
                if matches!(result, crate::network::ProbeResult::Ok(_)) {
                    // The check of the operator goes first; else the check
                    // of the network itself (a WARP member: its exit trace).
                    if let Some((echo, expect)) = self.egress.get(&member.name) {
                        result = probe_egress(&member.way, echo, *expect, thresholds.timeout).await;
                    } else if let Some(check) = self
                        .control
                        .network_of(&member.name)
                        .and_then(|n| n.egress_check())
                    {
                        result = check(member.way.clone(), thresholds.timeout).await;
                    }
                }
                results.push(result);
            }
            let control = results
                .iter()
                .any(|r| matches!(r, crate::network::ProbeResult::Ok(_)));
            let now = Instant::now();
            let mut renew = Vec::new();
            for (i, result) in results.into_iter().enumerate() {
                if group
                    .observe(i, result, control, now, &thresholds)
                    .is_some_and(|step| step.renew)
                {
                    renew.push(group.members()[i].name.clone());
                }
            }
            group.end_round();
            for name in renew {
                self.renew_member(&name).await;
            }
        }
    }

    /// A member whose probe asked for a renew (`Down` after K clean misses
    /// with a passed control, `WrongEgress`, or `AuthExpired`) gets a new
    /// link when its network renews (a WARP member, T-097 items 3 and 10):
    /// the same identity file, make before break, and a gap between two
    /// renewals. A control that failed gives no `Down`, so an outage of the
    /// host renews nothing (item 7).
    async fn renew_member(&self, name: &str) {
        if !self
            .control
            .network_of(name)
            .is_some_and(|network| network.renews())
        {
            return;
        }
        if let Err(answer) = self.control.renew_for_health(name).await {
            eprintln!("podnet: route  renew {name} failed: {}", answer.text);
        }
    }
}
