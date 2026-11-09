use crate::cli::cmd::tunnel::split_host_port;

/// warp probe targets (issue 67): a name, an IPv4 and an IPv6 address
/// with a port; no port, port 0 or an empty host is refused.
#[test]
fn probe_targets_take_a_name_or_an_address() {
    assert_eq!(
        split_host_port("example.com:443").unwrap(),
        ("example.com".into(), 443)
    );
    assert_eq!(
        split_host_port("1.1.1.1:443").unwrap(),
        ("1.1.1.1".into(), 443)
    );
    assert_eq!(
        split_host_port("[2606:4700::1]:80").unwrap(),
        ("2606:4700::1".into(), 80)
    );
    for bad in ["example.com", "example.com:0", ":443", "a b:1", "host:http"] {
        assert!(split_host_port(bad).is_err(), "{bad}");
    }
}
