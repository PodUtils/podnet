//! The two rule engines (T-156): `podnet pod -r` and the WARP rules of
//! `podnet warp proxy` read the same words for a refusal.

use podnet::warp::rules::{Action, Rule as WarpRule};
use podnet_netns::{ProxyKind, Rule, RuleAction};

#[test]
fn reject_and_block_mean_the_same_in_both_rule_engines() {
    for word in ["reject", "block", "REJECT", "Block"] {
        assert_eq!(Action::parse(word).unwrap(), Action::Reject, "{word}");
        assert!(
            matches!(RuleAction::parse(word).unwrap(), RuleAction::Reject),
            "{word}"
        );
    }
    assert_eq!(Action::parse("direct").unwrap(), Action::Direct);
    match RuleAction::parse("direct").unwrap() {
        RuleAction::Proxy(proxy) => assert_eq!(proxy.kind, ProxyKind::Direct),
        RuleAction::Reject => panic!("direct is not a refusal"),
    }
    // One rule text means the same in both engines.
    for spec in ["cidr:10.0.0.0/8=block", "domain:.ads.example=reject"] {
        let warp = WarpRule::parse(spec, "-r 1").unwrap();
        let pod = Rule::parse(spec).unwrap();
        assert_eq!(warp.action, Action::Reject);
        assert!(matches!(pod.action, RuleAction::Reject));
        for (host, ip) in [
            ("10.1.2.3", Some("10.1.2.3".parse().unwrap())),
            ("x.ads.example", None),
            ("example.com", None),
        ] {
            assert_eq!(
                warp.matcher.matches(host, ip),
                pod.matcher.matches(host, ip),
                "{spec} {host}"
            );
        }
    }
}
