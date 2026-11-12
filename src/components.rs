//! The inventory of the components of this build (T-310, GitHub #476, row
//! E10-5): one [`Component`] for each carrier and each service, for
//! `podnet components`.
//!
//! * The carriers come from the registry ([`crate::carriers`]). The
//!   services ([`SERVICES`]) are the optional services of the link.
//! * The field names of a row are stable ([`FIELDS`]). A list holds at
//!   most [`MAX_ROWS`] rows, and a row is at most [`MAX_ROW_BYTES`] bytes
//!   in JSON.
//! * A row of a feature that this build leaves out says `available: false`
//!   and gives the reason of [`crate::build_features::reason`]. Its facts
//!   (`framing`, `duplex`) are `null`: nothing can report them.
//! * [`excluded`] lists the optional features that this build leaves out,
//!   with the reason of each.

use serde::Serialize;
use serde_json::{json, Value};

use crate::build_features;
use crate::carriers::{self, Options};

/// The most rows of a list.
pub const MAX_ROWS: usize = 64;

/// The most bytes of one row in JSON.
pub const MAX_ROW_BYTES: usize = 4096;

/// The field names of a row, in order. A name stays; a new field is added
/// at the end.
pub const FIELDS: [&str; 12] = [
    "name",
    "kind",
    "framing",
    "duplex",
    "needs_privilege",
    "available",
    "reason",
    "cost",
    "feature",
    "built",
    "about",
    "address",
];

/// One row of the inventory.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Component {
    /// The name: the word of `--carrier` for a carrier.
    pub name: String,
    /// `carrier` or `service`.
    pub kind: &'static str,
    /// `stream` or `datagram`; `null` when this build cannot say.
    pub framing: Option<&'static str>,
    /// `null` when this build cannot say.
    pub duplex: Option<bool>,
    pub needs_privilege: bool,
    /// True when this build holds the component and this host can use it.
    pub available: bool,
    /// Why the component is not available; `null` when it is.
    pub reason: Option<String>,
    /// `free` or `network`.
    pub cost: &'static str,
    /// The Cargo feature that holds the component; `null` for a component
    /// of every build.
    pub feature: Option<&'static str>,
    /// True when this binary holds the code (the feature is on).
    pub built: bool,
    pub about: &'static str,
    /// The form of the address; `null` for a service.
    pub address: Option<&'static str>,
}

impl Component {
    /// One text line: the name, the kind and the state.
    pub fn line(&self) -> String {
        let state = match &self.reason {
            None => "available".to_string(),
            Some(reason) => format!("unavailable: {reason}"),
        };
        format!(
            "{name}: {kind}, {framing}, {duplex}, cost {cost}, feature {feature}: {state}",
            name = self.name,
            kind = self.kind,
            framing = self.framing.unwrap_or("unknown framing"),
            duplex = match self.duplex {
                Some(true) => "duplex",
                Some(false) => "half duplex",
                None => "unknown duplex",
            },
            cost = self.cost,
            feature = self.feature.unwrap_or("none"),
        )
    }
}

/// A service of the link that sits behind a build feature.
#[derive(Clone, Copy)]
pub struct Service {
    pub name: &'static str,
    pub about: &'static str,
    pub cost: &'static str,
    pub feature: &'static str,
    /// The entry of the record that adds the code.
    pub entry: &'static str,
}

/// The services of this build, in order.
pub const SERVICES: [Service; 2] = [
    Service {
        name: "ss",
        about: "the Shadowsocks service over a stream of the link",
        cost: "network",
        feature: "ss",
        entry: "T-294",
    },
    Service {
        name: "trojan",
        about: "the Trojan service over a stream of the link",
        cost: "network",
        feature: "trojan",
        entry: "T-295",
    },
];

fn from_report(r: &carriers::Report) -> Component {
    let reason = r.available.as_ref().err().cloned();
    Component {
        name: r.name.to_string(),
        kind: "carrier",
        framing: r.built.then(|| r.framing()),
        duplex: r.built.then_some(r.facts.duplex),
        needs_privilege: r.needs_privilege,
        available: r.available.is_ok(),
        reason,
        cost: r.cost,
        feature: r.feature,
        built: r.built,
        about: r.about,
        address: Some(r.address),
    }
}

fn from_service(s: &Service) -> Component {
    let built = build_features::built(s.feature);
    let reason = if built {
        format!(
            "the feature is on, but the code of the service is not in this build ({})",
            s.entry
        )
    } else {
        build_features::reason(s.feature)
    };
    Component {
        name: s.name.to_string(),
        kind: "service",
        framing: None,
        duplex: None,
        needs_privilege: false,
        available: false,
        reason: Some(reason),
        cost: s.cost,
        feature: Some(s.feature),
        built,
        about: s.about,
        address: None,
    }
}

/// One [`Component`] for each carrier, then one for each service.
pub fn rows(options: &Options) -> Vec<Component> {
    let mut rows: Vec<Component> = carriers::report(options).iter().map(from_report).collect();
    rows.extend(SERVICES.iter().map(from_service));
    rows
}

/// The optional features that this build leaves out, each with its reason.
pub fn excluded() -> Vec<(&'static str, String)> {
    build_features::OPTIONAL
        .iter()
        .filter(|f| !build_features::built(f))
        .map(|f| (*f, build_features::reason(f)))
        .collect()
}

/// The JSON document: `components`, `features` (each optional feature and
/// whether it is built) and `excluded` (the reasons).
pub fn document(rows: &[Component]) -> Value {
    json!({
        "components": rows,
        "features": build_features::OPTIONAL
            .iter()
            .map(|f| json!({ "name": f, "built": build_features::built(f) }))
            .collect::<Vec<_>>(),
        "excluded": excluded()
            .into_iter()
            .map(|(name, reason)| json!({ "name": name, "reason": reason }))
            .collect::<Vec<_>>(),
        "portable": build_features::built("portable"),
    })
}

/// The check of the limits: at most [`MAX_ROWS`] rows, each at most
/// [`MAX_ROW_BYTES`] bytes in JSON.
pub fn check_limits(rows: &[Component]) -> Result<(), String> {
    if rows.len() > MAX_ROWS {
        return Err(format!("{} rows; the limit is {MAX_ROWS}", rows.len()));
    }
    for row in rows {
        let bytes = serde_json::to_string(row).map_or(usize::MAX, |s| s.len());
        if bytes > MAX_ROW_BYTES {
            return Err(format!(
                "the row {} has {bytes} bytes; the limit is {MAX_ROW_BYTES}",
                row.name
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row_has_the_stable_fields_and_the_limits() {
        let rows = rows(&Options::default());
        assert!(rows.len() >= carriers::ROWS.len() + SERVICES.len());
        check_limits(&rows).unwrap();
        for row in &rows {
            let value = serde_json::to_value(row).unwrap();
            let keys: Vec<&str> = value
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect();
            let mut want = FIELDS.to_vec();
            want.sort_unstable();
            let mut got = keys.clone();
            got.sort_unstable();
            assert_eq!(got, want, "{}", row.name);
        }
    }

    #[test]
    fn a_feature_that_is_off_is_unavailable_with_its_reason() {
        for row in rows(&Options::default()) {
            if let Some(feature) = row.feature.filter(|f| !build_features::built(f)) {
                assert!(!row.built && !row.available, "{}", row.name);
                assert!(row
                    .reason
                    .unwrap()
                    .contains(&build_features::reason(feature)));
                assert_eq!((row.framing, row.duplex), (None, None));
            }
        }
    }

    #[test]
    fn the_manual_has_an_entry_for_each_row() {
        for row in rows(&Options::default()) {
            assert!(
                crate::manual::inventory::COMPONENTS
                    .iter()
                    .any(|c| c.name == row.name && c.kind == row.kind),
                "no entry in the manual for {}",
                row.name
            );
        }
    }
}
