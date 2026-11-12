//! The carriers of the link (T-264, T-265): `podnet link` needs one that
//! this host can use. The doctor reports each carrier of the registry
//! ([`crate::carriers`]) with the reason of `available()`.

use super::*;
use crate::carriers::{self, Options, Report};

/// One carrier of the registry.
#[derive(Clone, Debug)]
pub struct CarrierRow {
    pub name: &'static str,
    /// `ok` when the host can use the carrier, else `fail`.
    pub verdict: Verdict,
    /// The line of `podnet link carriers`, or the reason of the refusal.
    pub detail: String,
}

/// The carriers of the link.
#[derive(Clone, Debug)]
pub struct Carriers(pub Vec<CarrierRow>);

impl Carriers {
    /// One row for each report.
    pub fn from_reports(reports: &[Report]) -> Carriers {
        Carriers(
            reports
                .iter()
                .map(|r| match &r.available {
                    // A carrier that the build leaves out is not judged.
                    Err(reason) if !r.built => CarrierRow {
                        name: r.name,
                        verdict: Verdict::Unknown,
                        detail: reason.clone(),
                    },
                    Ok(()) => CarrierRow {
                        name: r.name,
                        verdict: Verdict::Ok,
                        detail: format!("{}: podnet link can use it", r.about),
                    },
                    Err(reason) => CarrierRow {
                        name: r.name,
                        verdict: Verdict::Refused,
                        detail: reason.clone(),
                    },
                })
                .collect(),
        )
    }

    pub(super) fn to_json(&self) -> Value {
        Value::Array(
            self.0
                .iter()
                .map(|c| {
                    json!({
                        "carrier": c.name,
                        "verdict": c.verdict.name(),
                        "status": c.verdict.name(),
                        "detail": c.detail,
                    })
                })
                .collect(),
        )
    }

    pub(super) fn text(&self) -> String {
        let mut out = String::from("\ncarriers\n");
        for c in &self.0 {
            out.push_str(&format!(
                "  {:<10} {:<8} {}\n",
                c.name,
                c.verdict.name(),
                c.detail
            ));
        }
        out
    }
}

/// Each carrier of this build, tested on this host.
pub(super) fn carriers() -> Carriers {
    Carriers::from_reports(&carriers::report(&Options::default()))
}
