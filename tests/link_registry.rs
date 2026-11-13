//! The registry of the carriers of the link (T-264): the names, their
//! order, the facts, and `available()` with a reason. An unknown name
//! exits 64. A disabled carrier exits 69 with its reason. The tests bind
//! no TCP port.

use pod_link::{BoxIo, Carrier, CarrierError, CarrierErrorKind, Facts, Listener, Pending};
use podnet::carriers::{self, Options, Row};
use podnet::exit::{self, code_of};
use podnet::network::Addressing;

/// A carrier that the host refuses.
struct Refused;

impl Carrier for Refused {
    fn name(&self) -> &'static str {
        "refused"
    }
    fn facts(&self) -> Facts {
        Facts::STREAM
    }
    fn available(&self) -> Result<(), CarrierError> {
        Err(CarrierError::new(
            "refused",
            CarrierErrorKind::Unavailable,
            "the seccomp filter of this host denies the socket family",
        ))
    }
    fn dial<'a>(&'a self, _: &'a str) -> Pending<'a, BoxIo> {
        Box::pin(async {
            Err(CarrierError::new(
                "refused",
                CarrierErrorKind::Unavailable,
                "no",
            ))
        })
    }
    fn listen<'a>(&'a self, _: &'a str) -> Pending<'a, Box<dyn Listener>> {
        Box::pin(async {
            Err(CarrierError::new(
                "refused",
                CarrierErrorKind::Unavailable,
                "no",
            ))
        })
    }
}

fn make_refused(_: &Options) -> Result<Box<dyn Carrier>, CarrierError> {
    Ok(Box::new(Refused))
}

const DISABLED: [Row; 1] = [Row {
    name: "refused",
    about: "a carrier that this host refuses",
    address: "NAME",
    cost: "free",
    needs_privilege: false,
    feature: None,
    make: make_refused,
}];

#[test]
fn the_names_have_a_fixed_order() {
    assert_eq!(
        carriers::names(),
        [
            "unix", "memory", "tcp", "ws", "h2", "quic", "udp", "poll", "sse", "chunked",
            "httppoll", "dns", "s3", "icmp"
        ]
    );
}

#[test]
fn an_unknown_name_exits_64_and_lists_the_names() {
    let e = carriers::open("pigeon", &Options::default()).err().unwrap();
    assert_eq!(code_of(&e), exit::USAGE);
    let text = format!("{e:#}");
    assert!(text.contains("pigeon"), "{text}");
    assert!(
        text.contains(
            "unix, memory, tcp, ws, h2, quic, udp, poll, sse, chunked, httppoll, dns, s3, icmp"
        ),
        "{text}"
    );
    assert!(!text.contains("unknown carrier."), "{text}");
}

#[test]
fn a_carrier_that_the_host_allows_opens() {
    for name in carriers::ROWS
        .iter()
        .filter(|r| r.feature.is_none())
        .map(|r| r.name)
    {
        let c = carriers::open(name, &Options::default()).unwrap();
        assert_eq!(c.name(), name);
        assert!(c.available().is_ok());
    }
}

#[test]
fn a_disabled_carrier_exits_69_with_its_reason() {
    let e = carriers::open_in(&DISABLED, "refused", &Options::default())
        .err()
        .unwrap();
    assert_eq!(code_of(&e), exit::UNAVAILABLE);
    let text = format!("{e:#}");
    assert!(text.contains("refused"), "{text}");
    assert!(
        text.contains("the seccomp filter of this host denies the socket family"),
        "{text}"
    );
}

#[test]
fn a_bad_option_exits_64() {
    let options = Options {
        ring: 10,
        ..Options::default()
    };
    let e = carriers::open("memory", &options).err().unwrap();
    assert_eq!(code_of(&e), exit::USAGE);
}

#[test]
fn the_report_gives_the_reason_of_a_disabled_carrier() {
    let report = carriers::report_in(&DISABLED, &Options::default());
    assert_eq!(report.len(), 1);
    assert_eq!(report[0].name, "refused");
    let reason = report[0].available.as_ref().err().unwrap();
    assert!(reason.contains("denies the socket family"), "{reason}");
    assert!(
        report[0].line().contains("unavailable"),
        "{}",
        report[0].line()
    );
}

#[test]
fn the_report_of_the_built_in_carriers_has_the_facts() {
    let report = carriers::report(&Options::default());
    assert_eq!(report.len(), carriers::ROWS.len());
    // The carriers of the default build: the others sit behind a feature.
    for r in report.iter().filter(|r| r.feature.is_none()) {
        assert!(r.available.is_ok(), "{}: {:?}", r.name, r.available);
        assert!(r.facts.ordered && r.facts.reliable && r.facts.duplex);
        assert_eq!(r.facts.mtu, None);
        assert_eq!(r.framing(), "stream");
        // A carrier meets a peer: it takes no arbitrary destination.
        assert_eq!(r.addressing, Addressing::Peer);
        assert!(r.line().contains("available"), "{}", r.line());
    }
}
