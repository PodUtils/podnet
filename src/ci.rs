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
//! {"schema":1,"kind":"podnet.ci-egress/1","hosts":[
//!   {"host":"api.example","port":443,"udp":false}
//! ]}
//! ```
//!
//! The older form `"schema":"podnet.ci-egress/1"` (with no `kind`) is read
//! too. Another value of `schema` or `kind` is refused (GitHub #730).
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
    /// The stamp of the document: 1 (with `kind`), or the older form, the
    /// name of the form.
    #[serde(default)]
    pub schema: serde_json::Value,
    /// The name of the form: `podnet.ci-egress/1`.
    #[serde(default)]
    pub kind: Option<String>,
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
    form_of(&declaration)?;
    Ok(check(&declaration, policy))
}

/// The name of the form of a declaration.
pub const FORM: &str = "podnet.ci-egress/1";

/// Check the stamp of `declaration` (GitHub #730): `"schema": 1` with no
/// `kind` or the `kind` [`FORM`], or the older form: the name of the form in `schema`.
fn form_of(declaration: &Declaration) -> anyhow::Result<()> {
    let kind = declaration.kind.as_deref();
    let good = match &declaration.schema {
        serde_json::Value::Number(n) => n.as_u64() == Some(1) && kind.is_none_or(|k| k == FORM),
        serde_json::Value::String(s) => s == FORM && kind.is_none_or(|k| k == FORM),
        serde_json::Value::Null => kind.is_none_or(|k| k == FORM),
        _ => false,
    };
    if good {
        return Ok(());
    }
    anyhow::bail!(
        "the declaration has schema {} and kind {:?}: give \"schema\": 1 and \"kind\": \"{FORM}\"",
        declaration.schema,
        kind.unwrap_or("none")
    )
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
        schema: FORM.to_string(),
        allowed,
        refused,
    }
}
