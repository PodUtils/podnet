use super::*;
use crate::rules::{select_chain, Rule};
use crate::upstream::Proxy;

fn items(rules: &[Rule]) -> Vec<LintItem<'_>> {
    rules
        .iter()
        .enumerate()
        .map(|(n, r)| LintItem {
            matcher: r.matcher.as_ref().unwrap(),
            action: action_key(r.spec.split_once('=').unwrap().1),
            place: format!("-r {}", n + 1),
            spec: r.spec.clone(),
        })
        .collect()
}

fn parse(specs: &[&str]) -> Vec<Rule> {
    specs.iter().map(|s| Rule::parse(s).unwrap()).collect()
}

#[test]
fn shadowed_rule_is_reported() {
    let rules = parse(&[
        "domain:.example.com=direct",
        "domain:admin.example.com=socks5://127.0.0.1:1",
    ]);
    assert_eq!(shadowed(&items(&rules)), vec![(1, 0)]);
    let warnings = check(&items(&rules), false).unwrap();
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("-r 2") && warnings[0].contains("-r 1"));
    let cidr = parse(&[
        "cidr:10.0.0.0/8=direct",
        "cidr:10.1.0.0/16=socks5://127.0.0.1:1",
    ]);
    assert_eq!(shadowed(&items(&cidr)), vec![(1, 0)]);
    let v6 = parse(&[
        "cidr:10.0.0.0/8=direct",
        "cidr:fd00::/8=direct",
        "cidr:fd00:1::/32=reject",
    ]);
    assert_eq!(shadowed(&items(&v6)), vec![(2, 1)]);
}

#[test]
fn shadowed_reject_is_an_error() {
    let rules = parse(&[
        "domain:.example.com=direct",
        "domain:admin.example.com=reject",
    ]);
    let err = check(&items(&rules), false).unwrap_err().to_string();
    assert!(err.contains("-r 2") && err.contains("-r 1"), "{err}");
    assert!(err.contains("--allow-shadowed-rules"), "{err}");
    // The flag turns the error into a warning.
    assert_eq!(check(&items(&rules), true).unwrap().len(), 1);
    let cidr = parse(&["cidr:10.0.0.0/8=direct", "cidr:10.1.0.0/16=block"]);
    assert!(check(&items(&cidr), false).is_err());
}

#[test]
fn reversed_order_gives_no_report() {
    let rules = parse(&[
        "domain:admin.example.com=reject",
        "domain:.example.com=direct",
    ]);
    assert!(shadowed(&items(&rules)).is_empty());
    assert!(check(&items(&rules), false).unwrap().is_empty());
    let cidr = parse(&["cidr:10.1.0.0/16=reject", "cidr:10.0.0.0/8=direct"]);
    assert!(shadowed(&items(&cidr)).is_empty());
}

#[test]
fn same_action_gives_no_report() {
    let rules = parse(&[
        "domain:.example.com=reject",
        "domain:admin.example.com=BLOCK",
        "cidr:10.0.0.0/8=direct",
        "cidr:10.1.0.0/16=direct",
    ]);
    assert!(shadowed(&items(&rules)).is_empty());
}

#[test]
fn exact_rule_does_not_cover_a_subdomain() {
    let rules = parse(&[
        "domain:example.com=direct",
        "domain:www.example.com=reject",
        "domain:.example.com=reject",
        "domain:a.b=direct",
        "cidr:10.0.0.0/8=direct",
        "domain:10.1.1.1=reject",
    ]);
    assert!(shadowed(&items(&rules)).is_empty());
    assert!(check(&items(&rules), false).unwrap().is_empty());
}

// The lint and the lookup agree: a rule is reported when the lookup never
// picks it for a target that it matches.
#[test]
fn rule_lint_agrees_with_the_lookup() {
    let default = vec![Proxy::parse("socks5://127.0.0.1:9").unwrap()];
    let layouts: [&[&str]; 4] = [
        &[
            "domain:.example.com=direct",
            "domain:admin.example.com=reject",
        ],
        &[
            "domain:admin.example.com=reject",
            "domain:.example.com=direct",
        ],
        &["cidr:10.0.0.0/8=direct", "cidr:10.1.0.0/16=reject"],
        &["cidr:10.1.0.0/16=reject", "cidr:10.0.0.0/8=direct"],
    ];
    let probes = [
        "admin.example.com:443",
        "a.example.com:1",
        "example.com:1",
        "10.1.2.3:80",
        "10.9.9.9:80",
    ];
    for specs in layouts {
        let rules = parse(specs);
        let reported: Vec<usize> = shadowed(&items(&rules)).iter().map(|p| p.0).collect();
        for (n, rule) in rules.iter().enumerate() {
            let picked = probes.iter().any(|t| {
                let sel = select_chain(&rules, &default, t);
                sel.rule.is_some_and(|r| std::ptr::eq(r, rule))
            });
            assert_eq!(reported.contains(&n), !picked, "{specs:?} rule {n}");
        }
    }
}

// The command line: a shadowed reject is a usage error (exit 64); the
// flag allows it.
#[test]
fn pod_args_shadowed_reject_is_an_error() {
    let argv = |extra: &[&str]| -> Vec<String> {
        let mut v: Vec<String> = ["-r", "domain:.example.com=direct", "-r"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        v.push("domain:admin.example.com=reject".into());
        v.extend(extra.iter().map(|s| s.to_string()));
        v.extend(["--".into(), "true".into()]);
        v
    };
    let err = crate::cli::parse_args(&argv(&[])).err().unwrap();
    assert!(format!("{err:#}").contains("-r 2"), "{err:#}");
    assert!(crate::cli::parse_args(&argv(&["--allow-shadowed-rules"])).is_ok());
}

// The time of the pair loop for a file of 10 000 rules. Run by hand:
// `cargo test -p podnet-netns --lib -- measure_ten_thousand --ignored --nocapture`.
#[test]
#[ignore]
fn measure_ten_thousand_rules() {
    let domains: Vec<String> = (0..10_000)
        .map(|n| format!("domain:h{n}.example.com=direct"))
        .collect();
    let cidrs: Vec<String> = (0..10_000)
        .map(|n| format!("cidr:10.{}.{}.0/24=direct", n / 256, n % 256))
        .collect();
    for (name, specs) in [("domain", domains), ("cidr", cidrs)] {
        let rules: Vec<Rule> = specs.iter().map(|s| Rule::parse(s).unwrap()).collect();
        let start = std::time::Instant::now();
        let found = shadowed(&items(&rules));
        eprintln!(
            "{name}: 10000 rules, {:?}, {} found",
            start.elapsed(),
            found.len()
        );
    }
}

// T-328: the kinds `port:` and `tcp:`.
#[test]
fn rules_port_shadow_check_covers_port_and_tcp_rules() {
    let shadow = |specs: &[&str]| shadowed(&items(&parse(specs)));
    // A prefix rule matches each port: it covers a tcp rule of an address.
    assert_eq!(
        shadow(&[
            "cidr:127.0.0.1/32=direct",
            "tcp:127.0.0.1:5432=socks5://127.0.0.1:1"
        ]),
        vec![(1, 0)]
    );
    // A range covers a single port in it, and a smaller range.
    assert_eq!(
        shadow(&[
            "port:8000-8100=direct",
            "port:8080=reject",
            "port:8000-8010=direct"
        ]),
        vec![(1, 0)]
    );
    assert_eq!(
        shadow(&[
            "port:8000-8100=direct",
            "port:8000-8010=socks5://127.0.0.1:1"
        ]),
        vec![(1, 0)]
    );
    assert_eq!(
        shadow(&["port:5432=direct", "tcp:db.example:5432=reject"]),
        vec![(1, 0)]
    );
    assert_eq!(
        shadow(&["domain:.example=direct", "tcp:db.example:5432=reject"]),
        vec![(1, 0)]
    );
    assert_eq!(
        shadow(&["tcp:[::1]:80=direct", "tcp:[::1]:80=reject"]),
        vec![(1, 0)]
    );
    // The narrow rule first, no shadow; other port, other host, same action.
    assert!(shadow(&["tcp:127.0.0.1:5432=direct", "cidr:127.0.0.1/32=reject"]).is_empty());
    assert!(shadow(&["port:8080=direct", "port:8000-8100=reject"]).is_empty());
    assert!(shadow(&["port:8080=direct", "tcp:a.example:8081=reject"]).is_empty());
    assert!(shadow(&["cidr:10.0.0.0/8=direct", "tcp:127.0.0.1:80=reject"]).is_empty());
    assert!(shadow(&["domain:a.example=direct", "tcp:10.0.0.1:80=reject"]).is_empty());
    assert!(shadow(&["port:80=direct", "tcp:a.example:80=direct"]).is_empty());
    let rules = parse(&["port:8000-8100=direct", "port:8080=reject"]);
    let err = check(&items(&rules), false).unwrap_err().to_string();
    assert!(err.contains("-r 2") && err.contains("-r 1"), "{err}");
}
