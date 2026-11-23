//! CI-1: a pipeline declares its egress needs before its first step.
//!
//! A declaration names each host, its port and whether the step needs UDP.
//! [`check`] classifies each of them with the installed egress policy and
//! names each destination that the policy does not cover, so the runner
//! refuses before any step runs.
//!
//! The declaration is JSON:
//!
//! ```json
//! {"schema":"podnet.ci-egress/1","hosts":[
//!   {"host":"api.example","port":443,"udp":false}
//! ]}
//! ```
//!
//! A `proxy` rule carries TCP in this tree. A step that needs UDP with only
//! a proxy rule is refused, because the rule does not cover the need.

use serde::Deserialize;

use crate::policy::{Action, Policy};

/// One destination of a pipeline declaration.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Need {
    /// The host name or address.
    pub host: String,
    /// The port.
    pub port: u16,
    /// True when the step sends datagrams.
    #[serde(default)]
    pub udp: bool,
}

/// A pipeline declaration.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Declaration {
    /// The schema of the document.
    #[serde(default)]
    pub schema: String,
    /// The destinations of the pipeline.
    pub hosts: Vec<Need>,
}

/// A need that the policy does not cover, with the reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    /// The host of the need.
    pub host: String,
    /// The port of the need.
    pub port: u16,
    /// True when the step needs UDP.
    pub udp: bool,
    /// The policy's reason, or the reason that a proxy does not carry UDP.
    pub reason: String,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let transport = if self.udp { "UDP" } else { "TCP" };
        write!(
            f,
            "{}:{} ({transport}): {}",
            self.host, self.port, self.reason
        )
    }
}

/// The result of a check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    /// The schema that the declaration named.
    pub schema: String,
    /// The needs that the policy covers.
    pub allowed: usize,
    /// Each need that the policy does not cover.
    pub refused: Vec<Refusal>,
}

impl Report {
    /// True when every need is covered.
    pub fn covered(&self) -> bool {
        self.refused.is_empty()
    }
}

/// Read and check a declaration against `policy`.
pub fn check_text(text: &str, policy: &Policy) -> anyhow::Result<Report> {
    let declaration: Declaration = serde_json::from_str(text)
        .map_err(|e| anyhow::anyhow!("the declaration is not JSON of the CI egress schema: {e}"))?;
    Ok(check(&declaration, policy))
}

/// Check `declaration` against `policy`.
pub fn check(declaration: &Declaration, policy: &Policy) -> Report {
    let mut allowed = 0;
    let mut refused = Vec::new();
    for need in &declaration.hosts {
        let destination = crate::policy::Destination::new(&need.host, need.port);
        let verdict = policy.classify(&destination);
        if let Some(reason) = verdict.reason {
            refused.push(Refusal {
                host: need.host.clone(),
                port: need.port,
                udp: need.udp,
                reason,
            });
            continue;
        }
        if need.udp && verdict.action == Action::Proxy {
            refused.push(Refusal {
                host: need.host.clone(),
                port: need.port,
                udp: true,
                reason: format!(
                    "the rule {:?} sends the destination to a proxy, which carries TCP, not the UDP that the step needs",
                    verdict.rule.as_deref().unwrap_or("default")
                ),
            });
            continue;
        }
        allowed += 1;
    }
    Report {
        schema: declaration.schema.clone(),
        allowed,
        refused,
    }
}
