use super::*;

#[test]
fn a_valid_url_is_accepted_and_its_host_reported() {
    let link = ShareLink::new(
        "https://foo-bar.trycloudflare.com/",
        ShareSource::QuickTunnel,
    )
    .expect("valid");
    assert_eq!(link.url(), "https://foo-bar.trycloudflare.com/");
    assert_eq!(
        link.host(),
        "foo-bar.trycloudflare.com",
        "the host drops path and scheme"
    );
    assert_eq!(link.source(), ShareSource::QuickTunnel);
    assert_eq!(link.source().label(), "quick tunnel");
}

#[test]
fn a_url_with_credentials_is_refused_rather_than_shared() {
    // This is the case the type exists for: the browser accepts it, so
    // nothing else would ever complain, and pasting it shares the password.
    for bad in ["https://user:pass@example.com", "https://user@example.com"] {
        let err = ShareLink::new(bad, ShareSource::Manual).expect_err("credentials");
        match err {
            FeatureError::Invalid(m) => assert!(m.contains("credentials"), "{m}"),
            other => panic!("expected Invalid, got {other:?}"),
        }
    }
}

#[test]
fn malformed_and_non_http_urls_are_refused() {
    for bad in [
        "",
        "   ",
        "example.com",
        "ftp://example.com",
        "javascript:alert(1)",
        "file:///etc/passwd",
        "https://",
        "https:///path",
    ] {
        assert!(
            ShareLink::new(bad, ShareSource::Manual).is_err(),
            "{bad:?} must not become a shareable link"
        );
    }
}

#[test]
fn an_ipv6_literal_url_keeps_its_brackets_and_is_accepted() {
    // A colon-separated IPv6 host would be mistaken for a userinfo
    // separator unless the port is split off first.
    let link = ShareLink::new("http://[2001:db8::1]:8080/", ShareSource::Manual).expect("ipv6 url");
    assert_eq!(link.host(), "[2001:db8::1]:8080");
    assert_eq!(link.url(), "http://[2001:db8::1]:8080/");
}

#[test]
fn the_share_page_is_self_contained_and_escapes_the_note() {
    let link = ShareLink::new("https://foo.trycloudflare.com", ShareSource::QuickTunnel)
        .expect("valid")
        .with_note("<script>alert(1)</script> & co");

    let page = link.html().expect("html");
    assert!(page.starts_with("<!DOCTYPE html>"), "must be a document");
    assert!(
        page.contains("https://foo.trycloudflare.com"),
        "the link must be present"
    );
    // No external reference of any kind: nothing to fetch when offline.
    assert!(
        !page.contains("http://"),
        "the page must not fetch anything"
    );
    assert!(
        !page.contains("<script>"),
        "the note must be escaped: {page}"
    );
    assert!(page.contains("&lt;script&gt;"), "{page}");
    assert!(page.contains("&amp;"), "{page}");
}

#[test]
fn the_terminal_form_puts_the_url_first_and_labels_the_source() {
    let link = ShareLink::new("https://foo.trycloudflare.com", ShareSource::Relay)
        .expect("valid")
        .with_note("dev server");
    let text = link.terminal();
    let first = text.lines().next().expect("a line");
    assert_eq!(
        first, "https://foo.trycloudflare.com",
        "the URL must be copyable first"
    );
    assert!(text.contains("// dev server"));
    assert!(text.contains("ssh relay"));
}

#[test]
fn an_empty_note_is_dropped_rather_than_rendered_blank() {
    let link = ShareLink::new("https://foo.trycloudflare.com", ShareSource::Manual)
        .expect("valid")
        .with_note("   ");
    assert!(link.note().is_none(), "whitespace is not a note");
    // The source line stays; what must not appear is an empty "// " line.
    assert!(
        !link.terminal().lines().any(|l| l.trim_end() == "//"),
        "a blank note must not leave a dangling comment line: {}",
        link.terminal()
    );
    assert!(link.terminal().contains("// shared from podnet (manual)"));
}

#[test]
fn a_quick_tunnel_link_is_built_from_the_provisioned_url() {
    let tunnel = crate::quicktunnel::parse_quick_tunnel_response(
        200,
        r#"{"success":true,"result":{"id":"i","hostname":"foo.trycloudflare.com",
            "account_tag":"a","secret":"GlUwcZAOuFFaHRtTTFqbk1WVmpWMHFzOGdSSUthbG1K"},"errors":[]}"#,
    )
    .expect("parse response");

    let link = ShareLink::from_quick_tunnel(&tunnel.url()).expect("link");
    assert_eq!(link.url(), "https://foo.trycloudflare.com");
    assert_eq!(link.source(), ShareSource::QuickTunnel);
    assert!(link.qr().expect("qr").contains('█'));
}

#[test]
fn an_oversized_url_is_refused_before_it_reaches_the_encoder() {
    let long = format!("https://example.com/{}", "a".repeat(MAX_SHARE_URL_LEN));
    let err = ShareLink::new(&long, ShareSource::Manual).expect_err("too long");
    assert!(matches!(err, FeatureError::Invalid(_)));
}
