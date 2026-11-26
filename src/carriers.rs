//! The registry of the carriers of the link (T-264).
//!
//! The trait [`Carrier`] and its `available()` are in `pod-link` (podcrates
//! call C117). The registry is here: the names, their order, the flags and
//! the lines of `podnet`. [`ROWS`] is a static table in the binary, as
//! [`crate::networks`] is.
//!
//! * [`open`] gives a carrier by its name. An unknown name is a usage
//!   error (exit 64) that lists the names. A carrier that the host refuses
//!   is an unavailable error (exit 69) that gives the reason of
//!   `available()`.
//! * [`report`] gives one [`Report`] for each carrier, with the facts and
//!   the reason, for `podnet link carriers` (T-265) and for `podnet doctor`.
//! * A carrier meets a peer: its address names a peer, not a destination.
//!   Its [`Addressing`] is `Peer`, so a consumer of destinations refuses it
//!   ([`Addressing::admit`], GitHub #377).

use anyhow::anyhow;
use pod_link::memory::{self, Memory};
use pod_link::unix::{self, Unix};
use pod_link::{Carrier, CarrierError, CarrierErrorKind, Cost, Facts};

use crate::build_features;
use crate::exit::{self, Fault};
use crate::link::carrier_fault;
use crate::link_query::Query;
use crate::network::Addressing;

mod address;

pub use address::{check_address, unix_path};

/// What the flags of a command give to the carriers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    /// The unix carrier accepts a peer of another uid
    /// (`--allow-other-uid`).
    pub allow_other_uid: bool,
    /// The bytes of each ring of the memory carrier (`--ring`).
    pub ring: usize,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            allow_other_uid: false,
            ring: memory::DEFAULT_RING,
        }
    }
}

/// One row of the registry.
#[derive(Clone, Copy)]
pub struct Row {
    /// The name, the word of `--carrier`.
    pub name: &'static str,
    /// What the carrier is, in one line.
    pub about: &'static str,
    /// The form of its address.
    pub address: &'static str,
    /// The cost class of the carrier: the class that its facts give
    /// (`Facts::cost`). A build that leaves the carrier out prints this
    /// class, because the facts are unknown then.
    pub cost: Cost,
    /// True when the carrier needs a privilege of the process.
    pub needs_privilege: bool,
    /// The build feature that holds the carrier (T-311), when it has one.
    pub feature: Option<&'static str>,
    /// Make the carrier from the options. A bad option is a usage error.
    pub make: fn(&Options) -> Result<Box<dyn Carrier>, CarrierError>,
}

impl Row {
    /// True when this binary holds the carrier: it has no feature, or its
    /// feature is on (T-311).
    pub fn built(&self) -> bool {
        self.feature.is_none_or(build_features::built)
    }

    /// The error of a carrier that this build leaves out, when it does.
    fn off(&self) -> Option<CarrierError> {
        let feature = self.feature.filter(|f| !build_features::built(f))?;
        Some(CarrierError::new(
            self.name,
            CarrierErrorKind::Unavailable,
            build_features::reason(feature),
        ))
    }
}

fn make_unix(options: &Options) -> Result<Box<dyn Carrier>, CarrierError> {
    Ok(Box::new(Unix {
        allow_other_uid: options.allow_other_uid,
    }))
}

fn make_memory(options: &Options) -> Result<Box<dyn Carrier>, CarrierError> {
    Ok(Box::new(Memory::new(options.ring)?))
}

/// The credential of the probe of a mail carrier. It signs no connection:
/// the registry shows the facts and the host check alone.
#[cfg(any(feature = "smtp", feature = "imap"))]
fn probe_credential() -> pod_link::smtp::Credential {
    pod_link::smtp::Credential::new("probe", "probe")
        .expect("a probe credential has no control character")
}

/// The NTPv4 carrier for the registry (GitHub #484): its facts (a poll
/// interval, a datagram unit, no reliability) and its host check. The
/// registry has no peer, so a dial or a listen through it is a usage error.
#[cfg(feature = "ntp")]
fn make_ntp(_: &Options) -> Result<Box<dyn Carrier>, CarrierError> {
    Ok(Box::new(crate::link_probe::Probe::new(
        pod_link::ntp::Ntp::new(),
    )))
}

#[cfg(not(feature = "ntp"))]
fn make_ntp(_: &Options) -> Result<Box<dyn Carrier>, CarrierError> {
    pending("ntp")
}

/// The rate of the SMTP row over the fixed message of the gate (GitHub
/// #484): 300 bytes for each submission, and the carrier flushes a partial
/// batch after `pod_link::smtp::FLUSH_AFTER` (250 ms). The local stand in
/// `tests/mail_stand` took 298 to 305 ms for the message in ten runs of
/// `tests/link_carriers_drive.rs` on 2026-11-21, so the rate is 983 to 1007
/// B/s; the row declares the rounded 950 B/s. SMTP is push-only, so the
/// row has no service poll interval.
pub const SMTP_BYTES_PER_SECOND: u64 = 950;

/// The SMTP submission carrier for the registry (GitHub #484): the facts of
/// a write-only stream with the cost class `medium`, and the rate of the
/// fixed 300-byte message that the gate uses. The credential of the probe is
/// a made-up value and signs nothing.
#[cfg(feature = "smtp")]
fn make_smtp(_: &Options) -> Result<Box<dyn Carrier>, CarrierError> {
    Ok(Box::new(
        crate::link_probe::Probe::new(pod_link::smtp::Smtp::new(
            probe_credential(),
            "probe@invalid",
            "probe@invalid",
        ))
        .with_rate(pod_link::Rate {
            bytes_per_second: Some(SMTP_BYTES_PER_SECOND),
            poll_interval: None,
            cost: Cost::Medium,
        }),
    ))
}

#[cfg(not(feature = "smtp"))]
fn make_smtp(_: &Options) -> Result<Box<dyn Carrier>, CarrierError> {
    pending("smtp")
}

/// The IMAP retrieval carrier for the registry (GitHub #484): the facts of a
/// duplex stream with the cost class `high`, and its host check.
#[cfg(feature = "imap")]
fn make_imap(_: &Options) -> Result<Box<dyn Carrier>, CarrierError> {
    Ok(Box::new(crate::link_probe::Probe::new(
        pod_link::imap::Imap::new(probe_credential(), "INBOX", "INBOX"),
    )))
}

#[cfg(not(feature = "imap"))]
fn make_imap(_: &Options) -> Result<Box<dyn Carrier>, CarrierError> {
    pending("imap")
}

/// A carrier whose feature is on but whose code is not in the tree. The
/// entry that adds the carrier replaces the maker. A build with each
/// carrier feature on has no such maker.
#[cfg(not(all(
    feature = "tcp",
    feature = "ws",
    feature = "h2",
    feature = "quic",
    feature = "udp",
    feature = "poll",
    feature = "dns",
    feature = "s3",
    feature = "icmp",
    feature = "ntp",
    feature = "smtp",
    feature = "imap"
)))]
fn pending(name: &'static str) -> Result<Box<dyn Carrier>, CarrierError> {
    Err(CarrierError::new(
        name,
        CarrierErrorKind::Unavailable,
        "the feature is on, but the code of the carrier is not in this build".to_string(),
    ))
}

/// The carrier for the registry. It knows the facts and the host check of
/// `tcp`. It has no key of this install, so a dial or a listen through it
/// is a usage error: `link listen` and `link dial` build the real one
/// (`podnet::link_tcp::carrier`).
#[cfg(feature = "tcp")]
fn make_tcp(_: &Options) -> Result<Box<dyn Carrier>, CarrierError> {
    Ok(Box::new(crate::link_tcp::Probe::new()))
}

#[cfg(not(feature = "tcp"))]
fn make_tcp(_: &Options) -> Result<Box<dyn Carrier>, CarrierError> {
    pending("tcp")
}

/// The carrier `ws` for the registry: the facts and the host check, no key
/// of this install (`podnet::link_ws::carrier` is the real one).
#[cfg(feature = "ws")]
fn make_ws(_: &Options) -> Result<Box<dyn Carrier>, CarrierError> {
    Ok(Box::new(crate::link_ws::Probe::new()))
}

#[cfg(not(feature = "ws"))]
fn make_ws(_: &Options) -> Result<Box<dyn Carrier>, CarrierError> {
    pending("ws")
}

/// The carrier `h2` for the registry: the facts and the host check, no key
/// of this install (`podnet::link_h2::carrier` is the real one).
#[cfg(feature = "h2")]
fn make_h2(_: &Options) -> Result<Box<dyn Carrier>, CarrierError> {
    Ok(Box::new(crate::link_h2::Probe::new()))
}

#[cfg(not(feature = "h2"))]
fn make_h2(_: &Options) -> Result<Box<dyn Carrier>, CarrierError> {
    pending("h2")
}

/// The carrier `quic` for the registry: the facts and the host check, no
/// key of this install (`podnet::link_quic::carrier` is the real one).
#[cfg(feature = "quic")]
fn make_quic(_: &Options) -> Result<Box<dyn Carrier>, CarrierError> {
    Ok(Box::new(crate::link_quic::probe()))
}

#[cfg(not(feature = "quic"))]
fn make_quic(_: &Options) -> Result<Box<dyn Carrier>, CarrierError> {
    pending("quic")
}

/// The carrier `udp` for the registry: the facts and the host check, no
/// key of this install (`podnet::link_udp::carrier` is the real one).
#[cfg(feature = "udp")]
fn make_udp(_: &Options) -> Result<Box<dyn Carrier>, CarrierError> {
    Ok(Box::new(crate::link_udp::probe()))
}

#[cfg(not(feature = "udp"))]
fn make_udp(_: &Options) -> Result<Box<dyn Carrier>, CarrierError> {
    pending("udp")
}

/// The poll carriers for the registry: the facts (the cost class of each
/// form) and the host check, no key of this install
/// (`podnet::link_poll::carrier` is the real one).
#[cfg(feature = "poll")]
fn make_poll_form(name: &'static str) -> Result<Box<dyn Carrier>, CarrierError> {
    let form = crate::link_poll::form_of(name).expect("a poll form");
    Ok(Box::new(crate::link_poll::probe(form)))
}

#[cfg(not(feature = "poll"))]
fn make_poll_form(name: &'static str) -> Result<Box<dyn Carrier>, CarrierError> {
    pending(name)
}

fn make_poll(_: &Options) -> Result<Box<dyn Carrier>, CarrierError> {
    make_poll_form("poll")
}

fn make_sse(_: &Options) -> Result<Box<dyn Carrier>, CarrierError> {
    make_poll_form("sse")
}

fn make_chunked(_: &Options) -> Result<Box<dyn Carrier>, CarrierError> {
    make_poll_form("chunked")
}

fn make_httppoll(_: &Options) -> Result<Box<dyn Carrier>, CarrierError> {
    make_poll_form("httppoll")
}

/// The carrier `dns` for the registry: the facts (the cost class
/// `highest`) and the host check, no key of this install
/// (`podnet::link_dns::carrier` is the real one).
#[cfg(feature = "dns")]
fn make_dns(_: &Options) -> Result<Box<dyn Carrier>, CarrierError> {
    Ok(Box::new(crate::link_dns::probe()))
}

#[cfg(not(feature = "dns"))]
fn make_dns(_: &Options) -> Result<Box<dyn Carrier>, CarrierError> {
    pending("dns")
}

/// The carrier `s3` for the registry: the facts (the cost class
/// `highest`) and the host check, no key of this install and no endpoint
/// (`podnet::link_s3::carrier` is the real one).
#[cfg(feature = "s3")]
fn make_s3(_: &Options) -> Result<Box<dyn Carrier>, CarrierError> {
    Ok(Box::new(crate::link_s3::probe()))
}

#[cfg(not(feature = "s3"))]
fn make_s3(_: &Options) -> Result<Box<dyn Carrier>, CarrierError> {
    pending("s3")
}

/// The carrier `icmp` for the registry: the facts and the host check (a
/// raw ICMP socket opens), no key of this install
/// (`podnet::link_icmp::carrier` is the real one).
#[cfg(feature = "icmp")]
fn make_icmp(_: &Options) -> Result<Box<dyn Carrier>, CarrierError> {
    Ok(Box::new(crate::link_icmp::probe()))
}

#[cfg(not(feature = "icmp"))]
fn make_icmp(_: &Options) -> Result<Box<dyn Carrier>, CarrierError> {
    pending("icmp")
}

/// The carrier `serial` for the registry (PodUtils/podnet#499): the AX.25
/// frames in KISS framing, the window and the timeout of the baud rate.
/// The line opens at the address of the dial or the listen, which is
/// `serial:PATH[,BAUD]`, or from the flag `--carrier serial:PATH[,BAUD]`.
fn make_serial(_: &Options) -> Result<Box<dyn Carrier>, CarrierError> {
    Ok(Box::new(crate::offline::serial::Serial::default()))
}

/// The carrier `hf` for the registry (PodUtils/podnet#499): an ARDOP or
/// VARA modem on a serial line, with the deferring ARQ and the degraded
/// profile of `pod-offline`.
fn make_hf(_: &Options) -> Result<Box<dyn Carrier>, CarrierError> {
    Ok(Box::new(crate::offline::hf::Hf::default()))
}

/// The carriers of this build, in the order of the lists and of the
/// first choice.
pub const ROWS: [Row; 19] = [
    Row {
        name: unix::NAME,
        about: "a stream over AF_UNIX, mode 0600, the uid of the peer checked",
        address: "PATH",
        cost: Cost::Low,
        needs_privilege: false,
        feature: None,
        make: make_unix,
    },
    Row {
        name: memory::NAME,
        about: "two ends in one process, a ring for each direction",
        address: "NAME",
        cost: Cost::Low,
        needs_privilege: false,
        feature: None,
        make: make_memory,
    },
    Row {
        name: "tcp",
        about: "TCP with TLS 1.3, the Ed25519 key of each end pinned in the handshake",
        address: "KEY@HOST:PORT to dial, IP:PORT to listen",
        cost: Cost::Low,
        needs_privilege: false,
        feature: Some("tcp"),
        make: make_tcp,
    },
    Row {
        name: "ws",
        about: "a WebSocket over TLS 1.3, the Ed25519 key of each end pinned in the handshake",
        address: "KEY@wss://HOST[:PORT]/PATH to dial, IP:PORT to listen",
        cost: Cost::Low,
        needs_privilege: false,
        feature: Some("ws"),
        make: make_ws,
    },
    Row {
        name: "h2",
        about:
            "one POST of HTTP/2 over TLS 1.3, the Ed25519 key of each end pinned in the handshake",
        address: "KEY@https://HOST[:PORT]/PATH to dial, IP:PORT to listen",
        cost: Cost::Low,
        needs_privilege: false,
        feature: Some("h2"),
        make: make_h2,
    },
    Row {
        name: "quic",
        about: "one stream of a QUIC connection, the Ed25519 key of each end pinned in the TLS 1.3 handshake",
        address: "KEY@HOST:PORT to dial, IP:PORT to listen",
        cost: Cost::Low,
        needs_privilege: false,
        feature: Some("quic"),
        make: make_quic,
    },
    Row {
        name: "udp",
        about: "KCP over UDP, the Ed25519 key of each end pinned in the TLS 1.3 handshake above it",
        address: "KEY@HOST:PORT to dial, IP:PORT to listen",
        cost: Cost::Low,
        needs_privilege: false,
        feature: Some("udp"),
        make: make_udp,
    },
    Row {
        name: "poll",
        about: "a long poll of HTTP/1.1 over TLS 1.3, the Ed25519 key of each end pinned in the handshake",
        address: "KEY@https://HOST[:PORT]/PATH to dial, IP:PORT to listen",
        cost: Cost::Medium,
        needs_privilege: false,
        feature: Some("poll"),
        make: make_poll,
    },
    Row {
        name: "sse",
        about: "POSTs up and one answer of SSE events down, HTTP/1.1 over TLS 1.3, the keys pinned",
        address: "KEY@https://HOST[:PORT]/PATH to dial, IP:PORT to listen",
        cost: Cost::High,
        needs_privilege: false,
        feature: Some("poll"),
        make: make_sse,
    },
    Row {
        name: "chunked",
        about: "POSTs up and one chunked answer down, HTTP/1.1 over TLS 1.3, the keys pinned",
        address: "KEY@https://HOST[:PORT]/PATH to dial, IP:PORT to listen",
        cost: Cost::Medium,
        needs_privilege: false,
        feature: Some("poll"),
        make: make_chunked,
    },
    Row {
        name: "httppoll",
        about: "POSTs up and a GET poll down, HTTP/1.1 over TLS 1.3, the keys pinned",
        address: "KEY@https://HOST[:PORT]/PATH to dial, IP:PORT to listen",
        cost: Cost::Highest,
        needs_privilege: false,
        feature: Some("poll"),
        make: make_httppoll,
    },
    Row {
        name: "dns",
        about: "query names up and TXT answers down, an ARQ and TLS 1.3 above, the Ed25519 key of each end pinned",
        address: "KEY@ZONE to dial (--dns-server), IP:PORT to listen (--dns-zone)",
        cost: Cost::Highest,
        needs_privilege: false,
        feature: Some("dns"),
        make: make_dns,
    },
    Row {
        name: "s3",
        about: "objects of a bucket of the S3 API each way, TLS 1.3 above, the Ed25519 key of each end pinned",
        address: "KEY@BUCKET[/PREFIX] to dial, BUCKET[/PREFIX] to listen (--s3-endpoint)",
        cost: Cost::Highest,
        needs_privilege: false,
        feature: Some("s3"),
        make: make_s3,
    },
    Row {
        name: "icmp",
        about: "the data of ICMP echo messages on a raw socket (CAP_NET_RAW, --allow-raw icmp), an ARQ and TLS 1.3 above, the keys pinned",
        address: "KEY@HOST to dial, an IPv4 address to listen",
        cost: Cost::High,
        needs_privilege: true,
        feature: Some("icmp"),
        make: make_icmp,
    },
    Row {
        name: "ntp",
        about: "the bytes in the extension fields of NTPv4 packets over UDP 123, a new field type for each packet, a poll of 16 s to 64 s",
        address: "HOST[:PORT] to dial (port 123), IP:PORT to listen",
        cost: Cost::Highest,
        needs_privilege: false,
        feature: Some("ntp"),
        make: make_ntp,
    },
    Row {
        name: "smtp",
        about: "one message for each batch over submission, 587 with STARTTLS or 465 with TLS; the bytes up only, the bytes down come from imap",
        address: "HOST:PORT to dial (587 or 465)",
        cost: Cost::Medium,
        needs_privilege: false,
        feature: Some("smtp"),
        make: make_smtp,
    },
    Row {
        name: "imap",
        about: "the bytes up by APPEND and down by UID FETCH over 143 with STARTTLS or 993 with TLS",
        address: "HOST:PORT to dial (143 or 993)",
        cost: Cost::High,
        needs_privilege: false,
        feature: Some("imap"),
        make: make_imap,
    },
    Row {
        name: "serial",
        about: "AX.25 frames in KISS framing on a serial line, 256 bytes at most, a window of 4 and a retransmission timeout from the baud rate",
        address: "serial:PATH[,BAUD] (115200 baud without one)",
        cost: Cost::High,
        needs_privilege: false,
        feature: None,
        make: make_serial,
    },
    Row {
        name: "hf",
        about: "an ARDOP or VARA modem on a serial line, 300 to 3000 bit/s, half duplex with a busy signal and a turnaround, the ARQ of the modem and the degraded profile",
        address: "hf:PATH[,BPS] (1200 bit/s without one)",
        cost: Cost::High,
        needs_privilege: false,
        feature: None,
        make: make_hf,
    },
];

/// The names of the carriers, in order.
pub fn names() -> Vec<&'static str> {
    ROWS.iter().map(|r| r.name).collect()
}

fn find<'a>(rows: &'a [Row], name: &str) -> anyhow::Result<&'a Row> {
    rows.iter().find(|r| r.name == name).ok_or_else(|| {
        let list: Vec<&str> = rows.iter().map(|r| r.name).collect();
        exit::fault(
            Fault::Usage,
            anyhow!(
                "no carrier is named {name:?}: give one of {}",
                list.join(", ")
            ),
        )
    })
}

/// The carrier `name` of this build, ready to use. Exit 64 for an unknown
/// name or a bad option. Exit 69, with the reason, when the host refuses
/// the carrier.
pub fn open(name: &str, options: &Options) -> anyhow::Result<Box<dyn Carrier>> {
    open_in(&ROWS, name, options)
}

/// [`open`] over the rows `rows`.
pub fn open_in(rows: &[Row], name: &str, options: &Options) -> anyhow::Result<Box<dyn Carrier>> {
    let row = find(rows, name)?;
    if let Some(off) = row.off() {
        return Err(carrier_fault(off));
    }
    let carrier = (row.make)(options).map_err(carrier_fault)?;
    carrier.available().map_err(carrier_fault)?;
    Ok(carrier)
}

/// The carrier `name` with its address in the flag (`--carrier
/// serial:PATH[,BAUD]`, `--carrier hf:PATH[,BPS]`), or [`open`] for another
/// name. The address that the carrier holds is checked here, so a bad
/// spec exits before the command opens a listener.
pub fn open_spec(name: &str, spec: &str, options: &Options) -> anyhow::Result<Box<dyn Carrier>> {
    let carrier: Box<dyn Carrier> = match name {
        crate::offline::serial::NAME => {
            Box::new(crate::offline::serial::Serial::with_spec(spec.to_string()))
        }
        crate::offline::hf::NAME => Box::new(crate::offline::hf::Hf::with_spec(spec.to_string())),
        _ => return open(name, options),
    };
    carrier.available().map_err(carrier_fault)?;
    Ok(carrier)
}

/// Check the keys of `query` that depend on the carrier `name`: `mtu`
/// needs a datagram carrier and gives at most its unit (T-306). Exit 64
/// for an unknown carrier or a bad key. A carrier that this build leaves
/// out is checked when it opens (exit 69), not here.
pub fn check_query(name: &str, query: &Query) -> anyhow::Result<()> {
    check_query_in(&ROWS, name, query)
}

/// [`check_query`] over the rows `rows`.
pub fn check_query_in(rows: &[Row], name: &str, query: &Query) -> anyhow::Result<()> {
    let row = find(rows, name)?;
    // The `mtu` of the udp carrier is the largest datagram of KCP below its
    // stream: `podnet::link_keyed::Tune::of_query` checks it (T-276).
    if query.mtu.is_none() || row.off().is_some() || name == "udp" {
        return Ok(());
    }
    match (row.make)(&Options::default()) {
        Ok(carrier) => check_mtu(name, carrier.facts(), query),
        Err(_) => Ok(()),
    }
}

/// The `mtu` of `query` against the facts of the carrier `name`.
fn check_mtu(name: &str, facts: Facts, query: &Query) -> anyhow::Result<()> {
    let usage = |text: String| exit::fault(Fault::Usage, anyhow!(text));
    match (query.mtu, facts.mtu) {
        (Some(_), None) => Err(usage(format!(
            "the query key mtu: the carrier {name} is a stream carrier and has no unit"
        ))),
        (Some(mtu), Some(unit)) if mtu > unit => Err(usage(format!(
            "the query key mtu: the unit of the carrier {name} is {unit} bytes at most"
        ))),
        _ => Ok(()),
    }
}

/// What `link carriers` and `doctor` say about one carrier.
#[derive(Clone, Debug)]
pub struct Report {
    pub name: &'static str,
    pub about: &'static str,
    pub address: &'static str,
    pub cost: &'static str,
    pub needs_privilege: bool,
    pub feature: Option<&'static str>,
    /// False when the feature of the carrier is off: the facts are then
    /// unknown (the default ones), and `available` holds "not built".
    pub built: bool,
    pub facts: Facts,
    /// How the address names the other end.
    pub addressing: Addressing,
    /// `Ok`, or the reason that the host cannot use the carrier.
    pub available: Result<(), String>,
}

impl Report {
    /// The cost class of the carrier on the wire (`Facts::cost`):
    /// `low`, `medium`, `high` or `highest`.
    pub fn overhead(&self) -> &'static str {
        cost_name(self.facts.cost)
    }

    /// The payload bytes that the carrier moves in one direction for each
    /// second, when it declares a rate (`Facts::rate`, GitHub #484).
    pub fn bytes_per_second(&self) -> Option<u64> {
        self.facts.rate.bytes_per_second
    }

    /// The time between two polls of the carrier's service, when it polls
    /// (`Facts::rate`). It is the floor of the latency of the carrier.
    pub fn poll_interval(&self) -> Option<std::time::Duration> {
        self.facts.rate.poll_interval
    }

    /// The rate in one phrase: the payload rate and the poll interval, or
    /// the cost class alone when the carrier has not measured its rate. A
    /// carrier with a rate and no poll is push-only or continuous, and says
    /// `poll none` (GitHub #484: SMTP has no interval to poll).
    pub fn rate(&self) -> String {
        let mut parts = Vec::new();
        if let Some(bytes) = self.bytes_per_second() {
            parts.push(format!("{bytes} B/s"));
        }
        if let Some(poll) = self.poll_interval() {
            parts.push(format!("poll {} s", poll.as_secs()));
        } else if self.bytes_per_second().is_some() {
            parts.push("poll none".to_string());
        }
        if parts.is_empty() {
            format!("not measured, cost {}", self.overhead())
        } else {
            format!("{}, cost {}", parts.join(", "), self.overhead())
        }
    }

    /// `stream` for a carrier with no unit, else `datagram`.
    pub fn framing(&self) -> &'static str {
        if self.facts.mtu.is_none() {
            "stream"
        } else {
            "datagram"
        }
    }

    /// One line: the name, the flags and the diagnosis.
    pub fn line(&self) -> String {
        if !self.built {
            let reason = self.available.as_ref().err().map_or("", String::as_str);
            return format!(
                "{name}: cost {cost}, address {address} ({addressing}): unavailable: {reason}",
                name = self.name,
                cost = self.cost,
                address = self.address,
                addressing = self.addressing.name(),
            );
        }
        let state = match &self.available {
            Ok(()) => "available".to_string(),
            Err(reason) => format!("unavailable: {reason}"),
        };
        format!(
            "{name}: {framing}, {duplex}, {ordered}, mtu {mtu}, rate {rate}, address {address} ({addressing}): {state}",
            name = self.name,
            framing = self.framing(),
            duplex = if self.facts.duplex { "duplex" } else { "half duplex" },
            ordered = if self.facts.ordered { "ordered" } else { "unordered" },
            mtu = self.facts.mtu.map_or("none".to_string(), |m| m.to_string()),
            rate = self.rate(),
            address = self.address,
            addressing = self.addressing.name(),
        )
    }
}

/// The name of a cost class, the one source of truth for the row of the
/// registry and the facts of the carrier (GitHub #484).
pub fn cost_name(cost: Cost) -> &'static str {
    match cost {
        Cost::Low => "low",
        Cost::Medium => "medium",
        Cost::High => "high",
        Cost::Highest => "highest",
    }
}

/// One [`Report`] for each carrier of this build, in order.
pub fn report(options: &Options) -> Vec<Report> {
    report_in(&ROWS, options)
}

/// [`report`] over the rows `rows`.
pub fn report_in(rows: &[Row], options: &Options) -> Vec<Report> {
    rows.iter()
        .map(|row| {
            let made = match row.off() {
                Some(off) => Err(off),
                None => (row.make)(options),
            };
            let (facts, available, cost) = match made {
                Ok(c) => {
                    let facts = c.facts();
                    // GitHub #484: the facts are the one source of truth
                    // for the cost class of a carrier this build holds. The
                    // row's declaration is for a carrier that is not built.
                    (
                        facts,
                        c.available().map_err(|e| e.to_string()),
                        cost_name(facts.cost),
                    )
                }
                Err(e) => (Facts::STREAM, Err(e.to_string()), cost_name(row.cost)),
            };
            Report {
                name: row.name,
                about: row.about,
                address: row.address,
                cost,
                needs_privilege: row.needs_privilege,
                feature: row.feature,
                built: row.built(),
                facts,
                addressing: Addressing::Peer,
                available,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests;
