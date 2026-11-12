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
use pod_link::{Carrier, CarrierError, CarrierErrorKind, Facts};

use crate::build_features;
use crate::exit::{self, Fault};
use crate::link::carrier_fault;
use crate::network::Addressing;

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
    /// The cost class: `free` (no network, no money) or `network`.
    pub cost: &'static str,
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

/// A carrier whose feature is on but whose code is not in the tree. The
/// entry that adds the carrier replaces the maker.
fn pending(name: &'static str, entry: &str) -> Result<Box<dyn Carrier>, CarrierError> {
    Err(CarrierError::new(
        name,
        CarrierErrorKind::Unavailable,
        format!("the feature is on, but the code of the carrier is not in this build ({entry})"),
    ))
}

fn make_quic(_: &Options) -> Result<Box<dyn Carrier>, CarrierError> {
    pending("quic", "T-273")
}

fn make_s3(_: &Options) -> Result<Box<dyn Carrier>, CarrierError> {
    pending("s3", "T-279")
}

fn make_icmp(_: &Options) -> Result<Box<dyn Carrier>, CarrierError> {
    pending("icmp", "T-282")
}

/// The carriers of this build, in the order of the lists and of the
/// first choice.
pub const ROWS: [Row; 5] = [
    Row {
        name: unix::NAME,
        about: "a stream over AF_UNIX, mode 0600, the uid of the peer checked",
        address: "PATH",
        cost: "free",
        needs_privilege: false,
        feature: None,
        make: make_unix,
    },
    Row {
        name: memory::NAME,
        about: "two ends in one process, a ring for each direction",
        address: "NAME",
        cost: "free",
        needs_privilege: false,
        feature: None,
        make: make_memory,
    },
    Row {
        name: "quic",
        about: "QUIC streams to a peer, TLS 1.3 with the key of the peer pinned",
        address: "HOST:PORT",
        cost: "network",
        needs_privilege: false,
        feature: Some("quic"),
        make: make_quic,
    },
    Row {
        name: "s3",
        about: "a simplex pair of objects in a bucket",
        address: "BUCKET/PREFIX",
        cost: "network",
        needs_privilege: false,
        feature: Some("s3"),
        make: make_s3,
    },
    Row {
        name: "icmp",
        about: "echo requests and replies, a datagram carrier",
        address: "HOST",
        cost: "network",
        needs_privilege: true,
        feature: Some("icmp"),
        make: make_icmp,
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
            "{name}: {framing}, {duplex}, {ordered}, mtu {mtu}, cost {cost}, address {address} ({addressing}): {state}",
            name = self.name,
            framing = self.framing(),
            duplex = if self.facts.duplex { "duplex" } else { "half duplex" },
            ordered = if self.facts.ordered { "ordered" } else { "unordered" },
            mtu = self.facts.mtu.map_or("none".to_string(), |m| m.to_string()),
            cost = self.cost,
            address = self.address,
            addressing = self.addressing.name(),
        )
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
            let (facts, available) = match made {
                Ok(c) => (c.facts(), c.available().map_err(|e| e.to_string())),
                Err(e) => (Facts::STREAM, Err(e.to_string())),
            };
            Report {
                name: row.name,
                about: row.about,
                address: row.address,
                cost: row.cost,
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
