//! The local topology and status view of a mesh (T-289).
//!
//! [`Mesh::view`] builds the document of the verb `mesh` of the control
//! socket: the nodes of the route table (and this node) with their edges,
//! and the counters. [`metrics_text`] writes the counters as Prometheus
//! text. The view is local: the control socket is a unix socket of mode
//! 0600, and no network endpoint serves the view.
//!
//! * A node has its fingerprint, its local name (or `null`), the last `seq`
//!   of its announcements, the hops and the cost of its best route, the
//!   state of that route (`up`, `held` or `down`), its services, and
//!   `age_ms` (the time since its last accepted announcement).
//! * An edge has the peer, the cost, `last_probe_ms` (the time since the
//!   last accepted announcement of the node at the other side, `null` if
//!   none) and a state. The edges of this node are its links: `up`. The
//!   edges of another node are the neighbours that the node signed in its
//!   announcement: `claimed`. A claim is no proof of adjacency.
//! * Limits ([`ViewLimits`]): 4096 nodes, 64 edges for each node and a
//!   response of 256 KiB. The nodes are in the order of their fingerprints
//!   (this node first). A cut keeps the first ones. `truncated` counts what
//!   was cut: `nodes`, `edges` and `bytes` (nodes cut to fit the response).

use serde_json::{json, Value};

use super::Mesh;
use crate::link_ctl::DUMP_LIMIT;
use crate::mesh_table::MAX_ORIGINS;

/// The most edges of one node.
pub const MAX_EDGES: usize = 64;

/// The limits of the view. A request can only make them smaller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewLimits {
    /// The most nodes (at least 1: this node).
    pub nodes: usize,
    /// The most edges of one node.
    pub edges: usize,
    /// The most bytes of the document.
    pub bytes: usize,
}

impl Default for ViewLimits {
    fn default() -> Self {
        ViewLimits {
            nodes: MAX_ORIGINS,
            edges: MAX_EDGES,
            bytes: DUMP_LIMIT - 1,
        }
    }
}

impl ViewLimits {
    /// The limits that `request` asks for (`max_nodes`, `max_edges`),
    /// clamped to the default limits. A value that is no number exits as
    /// an error text.
    pub fn from_request(request: &Value) -> Result<ViewLimits, String> {
        let top = ViewLimits::default();
        let pick = |key: &str, max: usize, min: usize| -> Result<usize, String> {
            match &request[key] {
                Value::Null => Ok(max),
                v => v
                    .as_u64()
                    .map(|n| (n as usize).clamp(min, max))
                    .ok_or_else(|| format!("{key}: give a whole number")),
            }
        };
        Ok(ViewLimits {
            nodes: pick("max_nodes", top.nodes, 1)?,
            edges: pick("max_edges", top.edges, 0)?,
            bytes: top.bytes,
        })
    }
}

fn age(now: u64, at: u64) -> Value {
    json!(now.saturating_sub(at))
}

impl Mesh {
    /// The view of the mesh at time `now` (the verb `mesh`).
    pub fn view(&self, now: u64, limits: ViewLimits) -> Value {
        let st = self.lock();
        let me = self.me.public();
        let name = |k: &pod_identity::PublicKey| st.names.get(k).map_or(Value::Null, |n| json!(n));
        let probe =
            |k: &pod_identity::PublicKey| st.seen.get(k).map_or(Value::Null, |s| age(now, s.at_ms));
        let (mut cut_edges, mut total_edges) = (0u64, 0u64);
        let mut cap = |mut rows: Vec<Value>| {
            total_edges += rows.len() as u64;
            if rows.len() > limits.edges {
                cut_edges += (rows.len() - limits.edges) as u64;
                rows.truncate(limits.edges);
            }
            rows
        };
        let mut mine: Vec<(pod_identity::PublicKey, u32)> =
            st.neighbours.iter().map(|(k, c)| (*k, *c)).collect();
        mine.sort_by_key(|(k, _)| k.0);
        let edges = cap(mine
            .iter()
            .map(|(k, c)| {
                json!({"peer": k.display_fingerprint(), "cost": c,
                       "last_probe_ms": probe(k), "state": "up"})
            })
            .collect());
        let mut nodes = vec![json!({
            "fingerprint": me.display_fingerprint(),
            "name": name(&me),
            "self": true,
            "seq": st.seq,
            "hops": 0,
            "cost": 0,
            "next_hop": Value::Null,
            "state": "up",
            "services": self.services,
            "age_ms": 0,
            "edges": edges,
        })];
        let mut origins: Vec<_> = st.table.origins().copied().collect();
        origins.sort_by_key(|k| k.0);
        for origin in &origins {
            let ranked = st.table.ranked(origin, now);
            let Some(best) = ranked.first() else { continue };
            let state = if best.down_since_ms.is_some() {
                "down"
            } else if best.held(now) {
                "held"
            } else {
                "up"
            };
            let seen = st.seen.get(origin);
            let claimed = seen.map_or(Vec::new(), |s| {
                let mut n = s.neighbours.clone();
                n.sort_by_key(|(k, _)| k.0);
                n.iter()
                    .map(|(k, c)| {
                        json!({"peer": k.display_fingerprint(), "cost": c,
                               "last_probe_ms": age(now, s.at_ms), "state": "claimed"})
                    })
                    .collect()
            });
            nodes.push(json!({
                "fingerprint": origin.display_fingerprint(),
                "name": name(origin),
                "self": false,
                "seq": best.seq,
                "hops": best.hops,
                "cost": best.cost,
                "next_hop": best.next_hop.display_fingerprint(),
                "state": state,
                "services": seen.map_or(Vec::new(), |s| s.services.clone()),
                "age_ms": probe(origin),
                "edges": cap(claimed),
            }));
        }
        let total_nodes = nodes.len() as u64;
        let mut cut_nodes = 0u64;
        if nodes.len() > limits.nodes {
            cut_nodes = (nodes.len() - limits.nodes) as u64;
            nodes.truncate(limits.nodes);
        }
        // The response limit: drop nodes from the end until it fits. The
        // size of a node is its own text; the rest is the fixed part.
        let fixed = 1024;
        let mut size: usize = nodes.iter().map(|n| n.to_string().len() + 1).sum();
        let mut cut_bytes = 0u64;
        while nodes.len() > 1 && size + fixed > limits.bytes {
            let last = nodes.pop().map_or(0, |n| n.to_string().len() + 1);
            size -= last;
            cut_bytes += 1;
        }
        // Edges of cut nodes are not shown: count them as cut.
        let shown_edges: u64 = nodes
            .iter()
            .map(|n| n["edges"].as_array().map_or(0, Vec::len) as u64)
            .sum();
        let edges_cut_by_nodes = total_edges - cut_edges - shown_edges;
        json!({
            "ok": true,
            "verb": "mesh",
            "node": me.display_fingerprint(),
            "now_ms": now,
            "limits": {"nodes": limits.nodes, "edges_per_node": limits.edges, "bytes": limits.bytes},
            "counters": {
                "nodes": total_nodes,
                "edges": total_edges,
                "neighbours": st.neighbours.len(),
                "announcements_received": st.received,
                "announcements_dropped": st.dropped,
                "announcements_forwarded": st.forwarded,
                "route_evictions": st.table.evictions(),
                "truncated_nodes": cut_nodes,
                "truncated_edges": cut_edges + edges_cut_by_nodes,
                "truncated_bytes": cut_bytes,
            },
            "truncated": {
                "nodes": cut_nodes,
                "edges": cut_edges + edges_cut_by_nodes,
                "bytes": cut_bytes,
            },
            "nodes": nodes,
        })
    }
}

/// The counters of a view as Prometheus text (`--metrics`).
pub fn metrics_text(view: &Value) -> String {
    const ROWS: [(&str, &str, &str, &str); 10] = [
        (
            "nodes",
            "podnet_mesh_nodes",
            "gauge",
            "Nodes in the route table and this node.",
        ),
        ("edges", "podnet_mesh_edges", "gauge", "Edges of the view."),
        (
            "neighbours",
            "podnet_mesh_neighbours",
            "gauge",
            "Links of this node that are up.",
        ),
        (
            "announcements_received",
            "podnet_mesh_announcements_received_total",
            "counter",
            "Announcements that reached the route table.",
        ),
        (
            "announcements_dropped",
            "podnet_mesh_announcements_dropped_total",
            "counter",
            "Announcements that were dropped.",
        ),
        (
            "announcements_forwarded",
            "podnet_mesh_announcements_forwarded_total",
            "counter",
            "Announcements sent on with the hop of this node.",
        ),
        (
            "route_evictions",
            "podnet_mesh_route_evictions_total",
            "counter",
            "Origins that the route table evicted.",
        ),
        (
            "truncated_nodes",
            "podnet_mesh_truncated_nodes",
            "gauge",
            "Nodes cut from the view by the node limit.",
        ),
        (
            "truncated_edges",
            "podnet_mesh_truncated_edges",
            "gauge",
            "Edges cut from the view.",
        ),
        (
            "truncated_bytes",
            "podnet_mesh_truncated_bytes_nodes",
            "gauge",
            "Nodes cut from the view by the response limit.",
        ),
    ];
    let mut out = String::new();
    for (key, name, kind, help) in ROWS {
        let n = view["counters"][key].as_u64().unwrap_or(0);
        out.push_str(&format!(
            "# HELP {name} {help}\n# TYPE {name} {kind}\n{name} {n}\n"
        ));
    }
    out
}
