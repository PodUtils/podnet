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
    cost: pod_link::Cost::Low,
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
            "httppoll", "dns", "s3", "icmp", "ntp", "smtp", "imap", "serial", "hf"
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
            "unix, memory, tcp, ws, h2, quic, udp, poll, sse, chunked, httppoll, dns, s3, icmp, ntp, smtp, imap"
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
        assert!(
            r.facts.ordered && r.facts.reliable,
            "{}: the facts of a stream",
            r.name
        );
        // Every carrier of the default build is duplex but the HF modem,
        // whose radio is half duplex and holds each transmission.
        assert!(
            r.facts.duplex || r.name == "hf",
            "{}: half duplex is only the HF modem",
            r.name
        );
        assert_eq!(r.facts.mtu, None);
        assert_eq!(r.framing(), "stream");
        // A carrier meets a peer: it takes no arbitrary destination.
        assert_eq!(r.addressing, Addressing::Peer);
        assert!(r.line().contains("available"), "{}", r.line());
    }
}

/// GitHub #484, the cost disagreement: `link carriers` printed the row's
/// `free` for serial and hf while the facts said `high`. The text and the
/// JSON now take the cost class from the same facts, and they agree for
/// every row.
#[test]
fn the_text_and_the_json_agree_on_the_cost_class_of_every_row() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(["link", "carriers", "--json"])
        .env_remove("RUST_LOG")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let rows = doc["carriers"].as_array().unwrap();
    assert_eq!(rows.len(), carriers::ROWS.len());

    let text = std::process::Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(["link", "carriers"])
        .env_remove("RUST_LOG")
        .output()
        .unwrap();
    assert_eq!(text.status.code(), Some(0), "{text:?}");
    let text = String::from_utf8(text.stdout).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), rows.len());

    for (row, line) in rows.iter().zip(&lines) {
        let name = row["name"].as_str().unwrap();
        let cost = row["cost"].as_str().unwrap();
        assert!(line.starts_with(&format!("{name}: ")), "{line}");
        assert!(
            line.contains(&format!("cost {cost},")),
            "{name}: the text says another cost: {line}"
        );
        if row["built"].as_bool().unwrap() {
            assert_eq!(row["overhead"], cost, "{name}: {row}");
            assert_eq!(row["rate"]["cost"], cost, "{name}: {row}");
        }
    }

    // The two rows of the offline agent's report, and the classes that
    // their facts give.
    for (name, class) in [("serial", "high"), ("hf", "high")] {
        let row = rows.iter().find(|r| r["name"] == name).unwrap();
        assert_eq!(row["cost"], class, "{row}");
        assert!(
            lines
                .iter()
                .any(|l| l.starts_with(&format!("{name}: "))
                    && l.contains(&format!("cost {class},"))),
            "no text line of {name} with cost {class}"
        );
    }
}

/// GitHub #484: `link carriers` lists the NTP, SMTP and IMAP carriers with a
/// cost class and a rate. NTP and IMAP declare a rate; SMTP declares its
/// cost class and reports no rate until a submission measures one. A build
/// without the feature lists the carrier as not built, as each other gated
/// carrier.
#[test]
fn the_three_network_service_carriers_carry_a_cost_class_and_a_rate() {
    let report = carriers::report(&Options::default());
    let row = |name: &str| {
        report
            .iter()
            .find(|r| r.name == name)
            .unwrap_or_else(|| panic!("no row {name}"))
    };
    // NTP: a poll each 32 s by default, a datagram unit, no reliability.
    let ntp = row("ntp");
    assert_eq!(ntp.feature, Some("ntp"));
    assert_eq!(ntp.cost, "highest");
    if ntp.built {
        assert_eq!(ntp.overhead(), "highest");
        assert_eq!(ntp.bytes_per_second(), Some(9));
        assert_eq!(
            ntp.poll_interval(),
            Some(std::time::Duration::from_secs(32))
        );
        assert!(ntp.facts.ordered && ntp.facts.duplex && !ntp.facts.reliable);
        assert_eq!(ntp.facts.mtu, Some(300));
        assert!(
            ntp.line().contains("rate 9 B/s, poll 32 s"),
            "{}",
            ntp.line()
        );
    }
    // SMTP: one direction, the cost class `medium`, and the measured rate of
    // a fixed 300-byte message. It is push-only: no poll interval.
    let smtp = row("smtp");
    assert_eq!(smtp.feature, Some("smtp"));
    assert_eq!(smtp.cost, "medium");
    if smtp.built {
        assert_eq!(smtp.overhead(), "medium");
        assert!(smtp.facts.ordered && smtp.facts.reliable && !smtp.facts.duplex);
        assert_eq!(
            smtp.bytes_per_second(),
            Some(carriers::SMTP_BYTES_PER_SECOND)
        );
        assert_eq!(smtp.poll_interval(), None);
        assert!(
            smtp.rate().contains("950 B/s, poll none, cost medium"),
            "{}",
            smtp.rate()
        );
        assert!(
            smtp.line()
                .contains("cost medium, rate 950 B/s, poll none, cost medium"),
            "{}",
            smtp.line()
        );
    }
    // IMAP: both directions, a poll each 5 s, the cost class `high`.
    let imap = row("imap");
    assert_eq!(imap.feature, Some("imap"));
    assert_eq!(imap.cost, "high");
    if imap.built {
        assert_eq!(imap.overhead(), "high");
        assert!(imap.bytes_per_second().is_some());
        assert_eq!(
            imap.poll_interval(),
            Some(std::time::Duration::from_secs(5))
        );
        assert!(imap.facts.ordered && imap.facts.reliable && imap.facts.duplex);
        assert!(imap.line().contains("poll 5 s"), "{}", imap.line());
    }
    // The JSON document carries the rate of each row, built or not.
    for name in ["ntp", "smtp", "imap"] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_podnet"))
            .args(["link", "carriers", "--json"])
            .env_remove("RUST_LOG")
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        let row = doc["carriers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["name"] == name)
            .unwrap_or_else(|| panic!("no JSON row {name}"));
        assert!(row["rate"].is_object(), "{row}");
        assert!(row["rate"]["cost"].is_string() || row["rate"]["cost"].is_null());
        if name == "smtp" {
            assert_eq!(row["cost"], "medium", "{row}");
            if smtp.built {
                assert_eq!(
                    row["rate"]["bytes_per_second"],
                    carriers::SMTP_BYTES_PER_SECOND,
                    "{row}"
                );
                assert!(row["rate"]["poll_interval_ms"].is_null(), "{row}");
                assert_eq!(row["rate"]["poll"], "none", "{row}");
            } else {
                // The facts of a carrier that the build leaves out are
                // unknown, so it declares its cost class and no rate.
                assert!(row["rate"]["bytes_per_second"].is_null(), "{row}");
            }
        }
    }
    // An unknown name still exits 64 and lists the new names.
    let dir = tempfile::tempdir().unwrap();
    let allow = dir.path().join("allow.txt");
    std::fs::write(&allow, "").unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args([
            "link",
            "listen",
            "/tmp/podnet-test.sock",
            "--carrier",
            "pigeon",
            "--allow",
            allow.to_str().unwrap(),
        ])
        .env("PODNET_STATE_DIR", dir.path().join("state"))
        .env("PODNET_RUN_DIR", dir.path().join("run"))
        .env_remove("RUST_LOG")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(64), "{out:?}");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("pigeon") && err.contains("imap"), "{err}");
}
