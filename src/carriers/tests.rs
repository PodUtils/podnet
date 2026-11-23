use super::*;
use crate::link_query::parse;

#[test]
fn mtu_needs_a_datagram_carrier_and_fits_its_unit() {
    let q = parse("mtu=1200").unwrap();
    let datagram = Facts {
        mtu: Some(1400),
        ..Facts::STREAM
    };
    assert!(check_mtu("udp", datagram, &q).is_ok());
    let big = parse("mtu=1500").unwrap();
    let m = format!("{:#}", check_mtu("udp", datagram, &big).unwrap_err());
    assert!(m.contains("1400") && !m.contains("1500"), "{m}");
    let m = format!("{:#}", check_mtu("unix", Facts::STREAM, &q).unwrap_err());
    assert!(m.contains("stream carrier") && !m.contains("1200"), "{m}");
    assert!(check_mtu("unix", Facts::STREAM, &Query::default()).is_ok());
}

/// GitHub #622 item 8: a network address on the unix carrier exits 64
/// and names the carrier to give. A path stays good.
#[test]
fn a_network_address_on_the_unix_carrier_names_the_carrier() {
    for (address, carrier) in [
        ("tcp://127.0.0.1:18084", "--carrier tcp"),
        ("TCP://127.0.0.1:18084", "--carrier tcp"),
        ("wss://example.org/link", "--carrier ws"),
        ("ws://127.0.0.1:80/", "--carrier ws"),
        ("https://example.org/link", "--carrier h2"),
        ("quic://127.0.0.1:443", "--carrier quic"),
        ("udp://127.0.0.1:443", "--carrier udp"),
        ("KEY@tcp://127.0.0.1:1", "--carrier tcp"),
        ("gopher://x", "--carrier NAME"),
        ("127.0.0.1:18084", "--carrier tcp"),
        ("[::1]:18084", "--carrier tcp"),
        ("localhost:8080", "--carrier tcp"),
        ("KEY@127.0.0.1:18084", "--carrier tcp"),
    ] {
        let err = check_address("unix", address).unwrap_err();
        assert_eq!(exit::code_of(&err), exit::USAGE, "{address}: {err:#}");
        let m = format!("{err:#}");
        assert!(m.contains(carrier), "{address}: {m}");
        assert!(!m.contains("18084"), "the message shows no address: {m}");
    }
    for path in [
        "/tmp/link.sock",
        "./link.sock",
        "link.sock",
        "/run/a@b/link.sock",
        "/tmp/x:1",
        "dir/127.0.0.1:80",
    ] {
        assert!(check_address("unix", path).is_ok(), "{path}");
    }
    assert!(check_address("tcp", "127.0.0.1:18084").is_ok());
    assert!(check_address("memory", "tcp://x").is_ok());
}

/// GitHub #634: `unix:PATH` is the path on the unix carrier. No socket
/// gets a name that starts with `unix:`.
#[test]
fn the_unix_carrier_takes_the_unix_prefix() {
    for (address, path) in [
        ("unix:/tmp/l1", "/tmp/l1"),
        ("unix:qa-literal", "qa-literal"),
        ("unix:///tmp/l1", "/tmp/l1"),
        ("KEY@unix:/tmp/l1", "KEY@/tmp/l1"),
        ("/tmp/l1", "/tmp/l1"),
        ("dir/unix:x", "dir/unix:x"),
    ] {
        assert_eq!(unix_path("unix", address), path, "{address}");
        assert!(check_address("unix", &unix_path("unix", address)).is_ok());
    }
    assert_eq!(unix_path("tcp", "unix:/x"), "unix:/x");
}

#[test]
fn check_query_knows_the_names() {
    let q = parse("mtu=1200").unwrap();
    assert!(check_query("unix", &q).is_err());
    assert!(check_query("nope", &Query::default()).is_err());
    assert!(check_query("unix", &Query::default()).is_ok());
}
