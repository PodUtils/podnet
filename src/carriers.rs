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
use pod_link::{Carrier, CarrierError, Facts};

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

fn make_unix(options: &Options) -> Result<Box<dyn Carrier>, CarrierError> {
    Ok(Box::new(Unix {
        allow_other_uid: options.allow_other_uid,
    }))
}

fn make_memory(options: &Options) -> Result<Box<dyn Carrier>, CarrierError> {
    Ok(Box::new(Memory::new(options.ring)?))
}

/// The carriers of this build, in the order of the lists and of the
/// first choice.
pub const ROWS: [Row; 2] = [
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
            let (facts, available) = match (row.make)(options) {
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
                facts,
                addressing: Addressing::Peer,
                available,
            }
        })
        .collect()
}
