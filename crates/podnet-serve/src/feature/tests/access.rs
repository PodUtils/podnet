use super::*;

#[test]
fn an_open_controller_allows_a_normal_request() {
    let c = AccessController::open();
    assert!(c.is_open());
    let ip: IpAddr = "203.0.113.9".parse().expect("parse ip");
    assert!(c.authorize(&req("GET", "/"), &ip).is_allowed());
}

#[test]
fn a_deny_rule_refuses_before_credentials_are_considered() {
    // Both conditions fail. The answer must be the IP refusal: a 401 here
    // would confirm to a stranger that a credential exists on this tunnel.
    let mut c = AccessController::open();
    c.deny_ip(IpRange::parse("203.0.113.0/24").expect("parse"))
        .require_basic_auth("podnet", &["u:p"])
        .expect("build");
    let ip: IpAddr = "203.0.113.9".parse().expect("parse");
    let decision = c.authorize(&req("GET", "/"), &ip);
    assert_eq!(status_line(&decision.to_bytes()), 403);
    let text = String::from_utf8(decision.to_bytes()).expect("utf8");
    assert!(
        !text.contains("WWW-Authenticate"),
        "leaked a challenge: {text}"
    );
}

#[test]
fn an_allow_list_refuses_addresses_outside_it() {
    let mut c = AccessController::open();
    c.allow_ip("office", IpRange::parse("198.51.100.0/24").expect("parse"));

    let inside: IpAddr = "198.51.100.7".parse().expect("parse");
    assert!(c.authorize(&req("GET", "/"), &inside).is_allowed());

    let outside: IpAddr = "192.0.2.7".parse().expect("parse");
    assert_eq!(
        status_line(&c.authorize(&req("GET", "/"), &outside).to_bytes()),
        403
    );
}

#[test]
fn an_empty_allow_list_permits_anything_not_denied() {
    // Documented semantics: an empty allow list is "allow all", not "deny
    // all". A typo in an allow rule must not look like a working policy.
    let c = AccessController::open();
    assert!(c.ip_filter().is_permissive());
    let ip: IpAddr = "192.0.2.1".parse().expect("parse");
    assert_eq!(c.ip_filter().check(&ip), IpVerdict::Permitted);
}

#[test]
fn cidr_prefixes_match_on_the_network_boundary_not_the_address() {
    let block = IpRange::parse("10.1.2.3/24").expect("parse");
    // Host bits in the given address must be ignored, or 10.1.2.3/24 and
    // 10.1.2.0/24 would be different networks.
    assert!(block.contains(&"10.1.2.0".parse().expect("parse")));
    assert!(block.contains(&"10.1.2.255".parse().expect("parse")));
    assert!(!block.contains(&"10.1.3.0".parse().expect("parse")));
    assert!(!block.contains(&"10.1.1.255".parse().expect("parse")));
}

#[test]
fn cidr_matching_handles_both_address_families_and_the_zero_prefix() {
    let v6 = IpRange::parse("2001:db8::/32").expect("parse");
    assert!(v6.contains(&"2001:db8:1234::1".parse().expect("parse")));
    assert!(!v6.contains(&"2001:db9::1".parse().expect("parse")));
    // An IPv4 rule must never match an IPv6 address.
    let v4 = IpRange::parse("0.0.0.0/0").expect("parse");
    assert!(v4.contains(&"8.8.8.8".parse().expect("parse")));
    assert!(!v4.contains(&"::1".parse().expect("parse")));
    // /0 is the whole family, which is what `AccessController::closed` uses.
    assert!(IpRange::parse("0.0.0.0/0")
        .expect("parse")
        .contains(&"1.2.3.4".parse().expect("parse")));
    assert!(IpRange::parse("2001:db8::1")
        .expect("parse")
        .contains(&"2001:db8::1".parse().expect("parse")));
}

#[test]
fn bad_address_specifications_are_rejected_rather_than_defaulting_open() {
    for bad in [
        "",
        "not-an-ip",
        "10.0.0.0/33",
        "10.0.0.0/-1",
        "10.0.0.0/x",
        "999.1.1.1",
    ] {
        assert!(
            IpRange::parse(bad).is_err(),
            "{bad:?} must not parse into an allow rule"
        );
    }
}

#[test]
fn basic_auth_challenges_then_accepts_the_right_credential() {
    let mut c = AccessController::open();
    c.require_basic_auth("podnet", &["alice:s3cret"])
        .expect("build");
    let ip: IpAddr = "203.0.113.1".parse().expect("parse");

    let refused = c.authorize(&req("GET", "/"), &ip);
    assert_eq!(status_line(&refused.to_bytes()), 401);
    assert!(String::from_utf8(refused.to_bytes())
        .expect("utf8")
        .contains("WWW-Authenticate: Basic realm=\"podnet\""));

    // base64("alice:s3cret")
    let good = req_with_auth("/", "Basic YWxpY2U6czNjcmV0");
    assert!(c.authorize(&good, &ip).is_allowed());
}

#[test]
fn wrong_and_malformed_credentials_are_403_not_401() {
    let mut c = AccessController::open();
    c.require_basic_auth("podnet", &["alice:s3cret"])
        .expect("build");
    let ip: IpAddr = "203.0.113.1".parse().expect("parse");

    for header in [
        "Basic YWxpY2U6d3Jvbmc=", // alice:wrong
        "Basic !!!not base64!!!",
        "Bearer sometoken",
        "Basic ", // nothing after the scheme
    ] {
        let decision = c.authorize(&req_with_auth("/", header), &ip);
        assert_eq!(
            status_line(&decision.to_bytes()),
            403,
            "{header:?} should be forbidden, not challenged"
        );
    }
}

#[test]
fn a_refusal_never_names_the_accepted_credentials() {
    // The message is the only thing an attacker reads. It must not confirm
    // which user or password would be accepted.
    let mut c = AccessController::open();
    c.require_basic_auth("podnet", &["alice:s3cret"])
        .expect("build");
    let ip: IpAddr = "203.0.113.1".parse().expect("parse");
    let text = String::from_utf8(c.authorize(&req("GET", "/"), &ip).to_bytes()).expect("utf8");
    assert!(!text.contains("alice"));
    assert!(!text.contains("s3cret"));
}

#[test]
fn an_empty_credential_table_is_a_configuration_error_not_an_open_gate() {
    // Failing open here would be the worst possible default: a flag that
    // names no users would publish the tunnel to everyone.
    let mut empty = AccessController::open();
    assert!(empty.require_basic_auth("podnet", &[]).is_err());

    let too_many: Vec<String> = (0..=MAX_ALLOWED_IDENTITIES)
        .map(|i| format!("u{i}:p{i}"))
        .collect();
    let refs: Vec<&str> = too_many.iter().map(String::as_str).collect();
    let mut too_many_controller = AccessController::open();
    assert!(too_many_controller
        .require_basic_auth("podnet", &refs)
        .is_err());
}

#[test]
fn an_unknown_method_is_405_and_a_known_one_is_not() {
    let c = AccessController::open();
    let ip: IpAddr = "203.0.113.1".parse().expect("parse");
    assert_eq!(
        status_line(&c.authorize(&req("TRACE", "/"), &ip).to_bytes()),
        405
    );
    for method in ["GET", "HEAD", "POST", "PUT", "DELETE", "OPTIONS", "PATCH"] {
        assert!(
            c.authorize(&req(method, "/"), &ip).is_allowed(),
            "{method} should reach the origin"
        );
    }
}

#[test]
fn ip_rules_round_trip_through_their_text_form() {
    let mut f = IpFilter::allow_all();
    f.deny_str("203.0.113.5").expect("deny single");
    f.allow_str("office", "198.51.100.0/24")
        .expect("allow block");
    assert_eq!(f.len(), 2);
    let described = f.describe();
    assert!(
        described.contains("allow office 198.51.100.0/24"),
        "{described}"
    );
    assert!(described.contains("deny 203.0.113.5"), "{described}");

    // Deny wins over allow for the same address.
    f.deny_str("198.51.100.7").expect("deny inside allowed");
    let hit: IpAddr = "198.51.100.7".parse().expect("parse");
    assert!(matches!(f.check(&hit), IpVerdict::DeniedBy(_)));
}

#[test]
fn a_closed_controller_refuses_an_ordinary_visitor() {
    let c = AccessController::closed();
    let ip: IpAddr = "203.0.113.9".parse().expect("parse");
    assert!(!c.is_open());
    assert_eq!(
        status_line(&c.authorize(&req("GET", "/"), &ip).to_bytes()),
        403
    );
}
