//! The metrics of the router (T-097): rows in the Prometheus text format,
//! one for each member and metric, built from the status document. The
//! control socket serves them at `GET /route/metrics`.

use std::fmt::Write as _;
use std::time::Duration;

use serde_json::Value;

use super::Router;

/// One metric of a member: its name, its type, its help, and how it reads
/// the member document (`None`: the member has no row for it).
type Reader = fn(&Value) -> Option<String>;

const MEMBER_METRICS: [(&str, &str, &str, Reader); 8] = [
    (
        "podnet_route_member_up",
        "gauge",
        "1 when the member is up, 0 in cooldown, on probation or down.",
        |m| Some(flag(m["health"] == "up")),
    ),
    (
        "podnet_route_member_takes_flows",
        "gauge",
        "1 when the member takes new flows (not down).",
        |m| Some(flag(m["health"] != "down")),
    ),
    (
        "podnet_route_member_weight",
        "gauge",
        "The weight of the member in its group.",
        |m| Some(m["weight"].to_string()),
    ),
    (
        "podnet_route_member_rtt_seconds",
        "gauge",
        "The time of the last probe that passed.",
        |m| m["rtt_ms"].as_f64().map(seconds),
    ),
    (
        "podnet_route_member_rtt_floor_seconds",
        "gauge",
        "The least time in the window of passing probes.",
        |m| m["rtt_floor_ms"].as_f64().map(seconds),
    ),
    (
        "podnet_route_member_open_dials",
        "gauge",
        "The dials of the member that have not ended.",
        |m| Some(m["opens"].to_string()),
    ),
    (
        "podnet_route_member_flows_total",
        "counter",
        "The flows that the pick gave the member.",
        |m| Some(m["flows"].to_string()),
    ),
    (
        "podnet_route_member_bytes_total",
        "counter",
        "The bytes (TCP and UDP, both directions) that the flows of the member carried.",
        |m| Some(m["bytes"].to_string()),
    ),
];

fn flag(on: bool) -> String {
    if on { "1" } else { "0" }.to_string()
}

fn seconds(ms: f64) -> String {
    format!("{}", Duration::from_secs_f64(ms / 1000.0).as_secs_f64())
}

/// The rows of the status document `status` of a router.
pub fn render(status: &Value) -> String {
    let groups = status["groups"].as_array().cloned().unwrap_or_default();
    let mut out = String::new();
    for (name, kind, help, read) in MEMBER_METRICS {
        let _ = writeln!(out, "# HELP {name} {help}\n# TYPE {name} {kind}");
        for group in &groups {
            for member in group["members"].as_array().into_iter().flatten() {
                if let Some(value) = read(member) {
                    let _ = writeln!(
                        out,
                        "{name}{{group=\"{}\",member=\"{}\"}} {value}",
                        label(&group["group"]),
                        label(&member["name"])
                    );
                }
            }
        }
    }
    let name = "podnet_route_group_refused_total";
    let _ = writeln!(
        out,
        "# HELP {name} The flows that the kill switch of the group refused.\n# TYPE {name} counter"
    );
    for group in &groups {
        let _ = writeln!(
            out,
            "{name}{{group=\"{}\"}} {}",
            label(&group["group"]),
            group["refused"]
        );
    }
    let guests = status["guests"].as_array().cloned().unwrap_or_default();
    if !guests.is_empty() {
        for (key, name, kind, help) in super::guest::COUNTERS {
            let _ = writeln!(out, "# HELP {name} {help}\n# TYPE {name} {kind}");
            for guest in &guests {
                let _ = writeln!(
                    out,
                    "{name}{{guest=\"{}\"}} {}",
                    label(&guest["wire"]),
                    guest["counters"][key]
                );
            }
        }
    }
    out
}

/// A label value: the names of the file are letters, digits, `-` and `_`,
/// and the escape keeps the text valid for any other name.
fn label(value: &Value) -> String {
    value
        .as_str()
        .unwrap_or("")
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

impl Router {
    /// The metrics rows of the router (T-097).
    pub fn metrics(&self) -> String {
        render(&self.status())
    }
}

#[cfg(test)]
mod tests;
