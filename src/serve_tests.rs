//! The tests of the service modules (crates/podnet-serve, T-051) that read
//! a module of a higher layer. They stay in the root crate: the crate of
//! the services cannot use the Cloudflare adapter.

mod feature {
    use crate::feature::{ShareLink, ShareSource};

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
}
