//! `Rules`: a way for each target (T-058). The first rule that matches
//! the target chooses its way, or refuses the target; a target that no
//! rule matches takes the default. The rules of `warp proxy -r`,
//! `pod -r` and `NO_PROXY` have this form.

use std::fmt;
use std::sync::Arc;

use tokio::time::Instant;

use super::{
    Addressing, BoxFuture, Class, Connected, Datagrams, Dial, Failure, Stage, Target, Way,
};

/// The match of a rule. The rules of the pod (`RuleMatch` of
/// `podnet-netns`: `domain:NAME`, `cidr:NET`) implement it in a crate that
/// uses both.
pub trait Matcher: Send + Sync {
    /// The rule in a log line and in an error (`domain:.example.com`).
    fn label(&self) -> String;

    /// True when the rule matches `target`. A name rule can match only a
    /// [`Target::Name`]: the caller sends the name ([`Rules`] says
    /// `names_inside`).
    fn matches(&self, target: &Target) -> bool;

    /// True when the rule matches `target` for a datagram flow. A rule of
    /// a port matches a TCP connection only (T-328): the default is
    /// [`Matcher::matches`], which a rule of a port overrides.
    fn matches_datagram(&self, target: &Target) -> bool {
        self.matches(target)
    }
}

/// A [`Matcher`] of a function.
pub struct MatchFn<F> {
    label: String,
    f: F,
}

impl<F: Fn(&Target) -> bool + Send + Sync> Matcher for MatchFn<F> {
    fn label(&self) -> String {
        self.label.clone()
    }

    fn matches(&self, target: &Target) -> bool {
        (self.f)(target)
    }
}

/// A matcher with the label `label` that runs `f`.
pub fn matcher<F>(label: &str, f: F) -> Arc<dyn Matcher>
where
    F: Fn(&Target) -> bool + Send + Sync + 'static,
{
    Arc::new(MatchFn {
        label: label.to_string(),
        f,
    })
}

/// One rule: a match and its way. With no way, the rule refuses the
/// target (the action `reject`).
#[derive(Clone)]
pub struct Rule {
    pub matcher: Arc<dyn Matcher>,
    pub way: Option<Way>,
}

impl fmt::Debug for Rule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Rule")
            .field("matcher", &self.matcher.label())
            .field("way", &self.way)
            .finish()
    }
}

/// A way for each target: the first rule that matches wins.
pub struct Rules {
    rules: Vec<Rule>,
    default: Option<Way>,
}

/// The choice of the rules for one target.
pub enum Choice<'a> {
    /// Dial through this way. `rule` is the label of the rule, or `None`
    /// for the default.
    Way { way: &'a Way, rule: Option<String> },
    /// Refuse the target. `rule` is as above.
    Refuse { rule: Option<String> },
}

impl Rules {
    /// The rules in their order, and the way of a target that no rule
    /// matches (`None`: refuse it). Each way must dial a destination.
    pub fn new(rules: Vec<Rule>, default: Option<Way>) -> Result<Rules, Failure> {
        let ways: Vec<Way> = rules
            .iter()
            .filter_map(|r| r.way.clone())
            .chain(default.clone())
            .collect();
        Addressing::Destination.admit(&ways)?;
        Ok(Rules { rules, default })
    }

    /// The choice for `target`.
    pub fn choose(&self, target: &Target) -> Choice<'_> {
        self.choose_with(|m| m.matches(target))
    }

    /// The choice for the datagrams of `target`: no rule of a port matches
    /// them (T-328).
    pub fn choose_datagram(&self, target: &Target) -> Choice<'_> {
        self.choose_with(|m| m.matches_datagram(target))
    }

    fn choose_with(&self, matches: impl Fn(&dyn Matcher) -> bool) -> Choice<'_> {
        let (way, rule) = match self.rules.iter().find(|r| matches(r.matcher.as_ref())) {
            Some(r) => (r.way.as_ref(), Some(r.matcher.label())),
            None => (self.default.as_ref(), None),
        };
        match way {
            Some(way) => Choice::Way { way, rule },
            None => Choice::Refuse { rule },
        }
    }

    fn way_for(&self, target: &Target) -> Result<&Way, Failure> {
        match self.choose(target) {
            Choice::Way { way, rule } => {
                tracing::debug!(%target, rule = rule.as_deref().unwrap_or("default"), way = %way.label(), "a rule chose a way");
                Ok(way)
            }
            Choice::Refuse { rule } => Err(refusal(target, rule)),
        }
    }
}

fn refusal(target: &Target, rule: Option<String>) -> Failure {
    let by = match rule {
        Some(rule) => format!("the rule {rule}"),
        None => "the default of the rules".to_string(),
    };
    Failure::new(
        Stage::Connect,
        Class::Policy,
        format!("{by} refuses {target}"),
    )
}

impl Dial for Rules {
    fn label(&self) -> String {
        let default = self
            .default
            .as_ref()
            .map_or("refuse".to_string(), Way::label);
        format!("rules({} rules, default {default})", self.rules.len())
    }

    fn dial<'a>(
        &'a self,
        target: &'a Target,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(async move { self.way_for(target)?.dial.dial(target, deadline).await })
    }

    fn dial_http<'a>(
        &'a self,
        target: &'a Target,
        deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(async move { self.way_for(target)?.dial.dial_http(target, deadline).await })
    }

    /// True: a name rule matches only a name, so the caller sends the
    /// name. A way that does not send the name resolves it itself.
    fn names_inside(&self) -> bool {
        true
    }

    /// The datagrams of the default way. A datagram socket has no target
    /// when it opens, so the rules do not choose its way.
    fn datagrams<'a>(&'a self) -> BoxFuture<'a, Result<Arc<dyn Datagrams>, Failure>> {
        Box::pin(async move {
            match &self.default {
                Some(way) => way.dial.datagrams().await,
                None => Err(Failure::new(
                    Stage::Connect,
                    Class::Policy,
                    "the default of the rules refuses UDP",
                )),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::fakes::{carrier, log, seen, soon, Fake};
    use super::*;

    fn domain(suffix: &'static str) -> Arc<dyn Matcher> {
        matcher(
            &format!("domain:{suffix}"),
            move |t| matches!(t, Target::Name { host, .. } if host.ends_with(suffix)),
        )
    }

    #[tokio::test]
    async fn the_first_rule_that_matches_chooses_the_way() {
        let log = log();
        let rules = Rules::new(
            vec![
                Rule {
                    matcher: domain(".corp.test"),
                    way: Some(Fake::up("relay", &log).way()),
                },
                Rule {
                    matcher: domain(".test"),
                    way: Some(Fake::up("warp", &log).way()),
                },
            ],
            Some(Fake::up("direct", &log).way()),
        )
        .unwrap();
        for (host, way) in [
            ("git.corp.test", "relay"),
            ("www.test", "warp"),
            ("example.org", "direct"),
            ("10.0.0.1", "direct"),
        ] {
            let conn = rules.dial(&Target::new(host, 443), soon()).await.unwrap();
            assert_eq!(carrier(conn).await, way, "{host}");
        }
        assert_eq!(seen(&log), ["relay", "warp", "direct", "direct"]);
        assert!(rules.names_inside());
        assert_eq!(rules.label(), "rules(2 rules, default direct)");
    }

    #[tokio::test]
    async fn a_rule_with_no_way_and_a_default_with_no_way_refuse() {
        let log = log();
        let rules = Rules::new(
            vec![Rule {
                matcher: domain(".ads.test"),
                way: None,
            }],
            None,
        )
        .unwrap();
        let err = rules
            .dial(&Target::new("x.ads.test", 80), soon())
            .await
            .expect_err("refused");
        assert_eq!((err.stage, err.class), (Stage::Connect, Class::Policy));
        assert_eq!(
            err.text(),
            "the rule domain:.ads.test refuses x.ads.test:80"
        );
        let err = rules
            .dial(&Target::new("example.org", 80), soon())
            .await
            .expect_err("refused");
        assert_eq!(
            err.text(),
            "the default of the rules refuses example.org:80"
        );
        let err = rules.datagrams().await.expect_err("no UDP");
        assert_eq!(err.class, Class::Policy);
        assert!(seen(&log).is_empty());
        assert_eq!(rules.label(), "rules(1 rules, default refuse)");
    }

    #[tokio::test]
    async fn udp_takes_the_default_way() {
        let log = log();
        let rules = Rules::new(Vec::new(), Some(Fake::up("direct", &log).way())).unwrap();
        let err = rules.datagrams().await.expect_err("the fake has no UDP");
        assert_eq!(err.text(), "direct carries no UDP");
        assert_eq!(seen(&log), ["direct udp"]);
    }

    #[test]
    fn rules_refuse_a_peer_way() {
        let log = log();
        let peer = Way::peer(Arc::new(Fake::up("wormhole", &log)));
        let err = Rules::new(
            vec![Rule {
                matcher: domain(".test"),
                way: Some(peer),
            }],
            None,
        )
        .err()
        .expect("a peer way");
        assert_eq!(err.class, Class::Usage);
        assert!(err.text().starts_with("wormhole meets a peer"), "{err}");
    }
}
