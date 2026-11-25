//! A refusal of the policy, as the error line reads it (GitHub #727): the
//! rule that refused, and the file that holds the rules. A refusal comes
//! before any connect, so the step of a failed hop (check the host, the
//! port, `--proxy`) is false for it.
//!
//! The typed [`super::Refused`] in the chain comes first. A layer that
//! keeps only the text of its cause (pod-ws, the first error of `bench`)
//! gives the second path: the words of [`super::Policy::classify`].

use super::{Action, BEFORE, NODE_VARIABLE, POLICY_VARIABLE};

/// A refusal of the policy in the chain of an error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stop {
    /// The name of the rule that refused, as the reason quotes it. None:
    /// no rule matched, and the default of the policy refused.
    pub rule: Option<String>,
}

/// The refusal of the policy that `err` holds, if any.
pub fn refusal_in(err: &anyhow::Error) -> Option<Stop> {
    err.chain()
        .filter_map(|e| e.downcast_ref::<super::Refused>())
        .find_map(|r| read(&r.reason))
        .or_else(|| err.chain().find_map(|e| read(&e.to_string())))
}

/// The refusal in `text`, by the words of the reason of a refusal.
fn read(text: &str) -> Option<Stop> {
    const LEAD: &str = "the rule \"";
    for action in [Action::Refuse, Action::Queue] {
        let tail = format!("\" {} the destination ", action.as_str());
        let mut from = 0;
        while let Some(end) = text[from..].find(&tail).map(|e| e + from) {
            let head = &text[..end];
            let after = &text[end..];
            if let Some(start) = head.rfind(LEAD) {
                if after.contains(BEFORE) {
                    let rule = head[start + LEAD.len()..].to_string();
                    return Some(Stop { rule: Some(rule) });
                }
            }
            from = end + tail.len();
        }
        if text.contains(&format!("so the policy {} it {BEFORE}", action.as_str())) {
            return Some(Stop { rule: None });
        }
    }
    None
}

impl Stop {
    /// The second line of the error: what refused, and where the rules
    /// are.
    pub fn hint(&self) -> String {
        self.hint_from(super::source())
    }

    /// [`Stop::hint`] for a policy that `source` gave.
    fn hint_from(&self, source: Option<(&str, &std::path::Path)>) -> String {
        let what = match &self.rule {
            Some(rule) => {
                format!("the rule \"{rule}\" of the egress policy refused this destination")
            }
            None => {
                "no rule of the egress policy matches this destination, and its default refuses it"
                    .to_string()
            }
        };
        let file = match source {
            Some((NODE_VARIABLE, path)) => format!(
                "{NODE_VARIABLE} names the node configuration {}; its egress object holds the rules",
                path.display()
            ),
            Some((_, path)) => format!(
                "{POLICY_VARIABLE} names the policy file {path}; podnet rules check {path} shows its rules",
                path = path.display()
            ),
            None => format!(
                "{POLICY_VARIABLE} names the policy file; podnet rules check FILE shows its rules"
            ),
        };
        format!("{what}; podnet made no connection; {file}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::{Destination, Policy};

    /// The reason of each kind of refusal reads back as its rule, also in
    /// the text of a layer that keeps only the words.
    #[test]
    fn each_refusal_reads_back_as_its_rule() {
        let policy = Policy::parse(
            r#"{"default":"queue","rules":[
                {"name":"no-example","action":"refuse","host":"example.com"},
                {"name":"hold \"q\"","action":"queue","host":"q.example"},
                {"name":"open","action":"direct","host":"open.example"}
            ]}"#,
        )
        .unwrap();
        let cases = [
            ("example.com", Some(Some("no-example"))),
            ("q.example", Some(Some("hold \\\"q\\\""))),
            ("other.example", Some(None)),
            ("open.example", None),
        ];
        for (host, want) in cases {
            let verdict = policy.classify(&Destination::new(host, 443));
            let text = format!(
                "GET https://{host}: could not connect to {host}:443: {}",
                verdict.reason.unwrap_or_default()
            );
            let got = read(&text).map(|s| s.rule);
            assert_eq!(got, want.map(|r| r.map(str::to_string)), "{text}");
        }
        assert_eq!(read("the rule \"x\" refuse the destination a:1"), None);
    }

    /// GitHub #727: each shape of a refused dial (typed, a failure of a
    /// crate, the text of a layer, an I/O error) gives the step of the
    /// policy as the second line, not the step of a hop.
    #[test]
    fn each_shape_of_a_refusal_gives_the_step_of_the_policy() {
        use crate::hop::{at, Hop};
        let rule = "the rule \"no-example\" refuse the destination example.com:443 (any family), before any connect";
        let default = "no rule matches api.example:443 (any family), so the policy refuse it before any connect (EGRESS-3)";
        let typed = || {
            let refused = crate::policy::Refused {
                reason: rule.into(),
            };
            crate::exit::fault(crate::exit::Fault::Unavailable, anyhow::Error::new(refused))
        };
        let failure = pod_core::Failure::caused(
            pod_core::Stage::Connect,
            pod_core::Class::Unavailable,
            rule,
            crate::policy::Refused {
                reason: rule.into(),
            },
        );
        let io = std::io::Error::new(std::io::ErrorKind::PermissionDenied, rule);
        let cases: [(anyhow::Error, &str); 6] = [
            (
                at(Hop::Connect, typed().context("GET https://example.com")),
                "the rule \"no-example\"",
            ),
            (
                anyhow::Error::new(failure).context("connecting"),
                "the rule \"no-example\"",
            ),
            (
                at(
                    Hop::Connect,
                    anyhow::anyhow!("bench: no request worked: GET https://example.com: {rule}"),
                ),
                "the rule \"no-example\"",
            ),
            (
                anyhow::anyhow!("TCP to example.com: could not connect to example.com:443: {rule}")
                    .context("the remote wss://example.com/ws does not answer"),
                "the rule \"no-example\"",
            ),
            (
                anyhow::Error::new(io).context("the cover origin"),
                "the rule \"no-example\"",
            ),
            (
                at(
                    Hop::Network,
                    anyhow::anyhow!("POST https://api.example/tunnel: {default}"),
                ),
                "its default refuses it",
            ),
        ];
        for (err, has) in cases {
            let line = crate::report::error_line(&err);
            let hint = line.lines().nth(1).unwrap_or_default();
            assert!(hint.contains(has), "{line}");
            assert!(hint.contains("podnet rules check"), "{line}");
            assert!(!hint.contains("hop failed"), "{line}");
            assert!(!hint.contains("--proxy"), "{line}");
            assert_eq!(line.lines().count(), 2, "{line}");
        }
    }

    /// The second line names the file of the variable that gave the policy.
    #[test]
    fn the_hint_names_the_file_of_the_rules() {
        let stop = Stop {
            rule: Some("no-example".into()),
        };
        let path = std::path::Path::new("/etc/pol.json");
        let cases = [
            (
                Some((POLICY_VARIABLE, path)),
                "podnet rules check /etc/pol.json",
            ),
            (
                Some((NODE_VARIABLE, path)),
                "PODNET_NODE_CONFIG names the node configuration /etc/pol.json",
            ),
            (None, "podnet rules check FILE"),
        ];
        for (source, has) in cases {
            let hint = stop.hint_from(source);
            assert!(hint.contains(has), "{hint}");
            assert!(hint.contains("the rule \"no-example\""), "{hint}");
            assert!(!hint.contains("--proxy"), "{hint}");
        }
    }
}
