//! The verb `dump` of the control socket (T-309, GitHub #476, row E10-4):
//! the state of the instance for a bug report.
//!
//! The document holds the build (version, optional features, carriers),
//! the fingerprint of the install, the settings and limits, the totals, one
//! row for each link with its peer fingerprint, its counters and its stream
//! table, and the last 20 errors. It holds no seed, no token and no secret
//! of a peer: the hub knows fingerprints and addresses only, and the text
//! that came from outside (the errors) goes through [`report::mask`] and
//! [`report::clean`], as does every other string. The route table waits for
//! the mesh (T-286): `routes.built` is false.
//!
//! The cap of the document is [`DUMP_LIMIT`] (256 KiB), more than the
//! answer limit of the other verbs. A table that does not fit is cut, from
//! the last rows, and the document counts the rows it cut: `streams_cut` in
//! a link, and `cut` at the top.

use std::collections::VecDeque;

use serde_json::{json, Value};

use super::{now, Held, Hub, Reply, DUMP_LIMIT, MAX_CLIENTS, REQUEST_LIMIT, RESPONSE_LIMIT};
use crate::report;

/// The errors that the dump keeps.
pub const ERRORS: usize = 20;
/// The characters that one error keeps.
const ERROR_TEXT: usize = 512;
/// The most links that the dump lists.
const LINK_ROWS: usize = 512;
/// The rows of a stream table to try, from the most to none. The dump takes
/// the first that fits the cap.
const ROW_STEPS: [usize; 8] = [usize::MAX, 1024, 256, 64, 16, 4, 1, 0];

/// The last errors of an instance: the second of the error and its text.
pub(super) type ErrorRing = VecDeque<(u64, String)>;

/// Hide the credentials in each string of `value` and strip its control
/// characters.
fn redact(value: &mut Value) {
    match value {
        Value::String(s) => *s = report::mask(&report::clean(s)),
        Value::Array(items) => items.iter_mut().for_each(redact),
        Value::Object(map) => map.values_mut().for_each(redact),
        _ => {}
    }
}

impl Held {
    /// Whether this end opened the stream: the dialer opens odd ids and the
    /// listener even ids.
    fn owner(&self, id: u32) -> &'static str {
        if (id % 2 == 1) == (self.role == "dialer") {
            "local"
        } else {
            "peer"
        }
    }

    fn dump_row(&self, rows: usize) -> Value {
        let mut row = self.row();
        if let Some(map) = row.as_object_mut() {
            map.remove("stream_ids");
        }
        let ids: Vec<u32> = self
            .streams
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .copied()
            .collect();
        let table: Vec<Value> = ids
            .iter()
            .take(rows)
            .map(|id| json!({"id": id, "owner": self.owner(*id)}))
            .collect();
        row["streams_cut"] = (ids.len() - table.len()).into();
        row["stream_table"] = table.into();
        row
    }
}

impl Hub {
    /// Remember an error for the dump. The dump keeps the last
    /// [`ERRORS`]; a text keeps its first 512 characters.
    pub fn note_error(&self, text: &str) {
        let text: String = text.chars().take(ERROR_TEXT).collect();
        let mut errors = self.errors.lock().unwrap_or_else(|e| e.into_inner());
        if errors.len() == ERRORS {
            errors.pop_front();
        }
        errors.push_back((now(), text));
    }

    /// The document of the verb `dump`, within [`DUMP_LIMIT`].
    pub fn dump(&self) -> Value {
        self.dump_within(DUMP_LIMIT - 1)
    }

    /// The dump, cut to at most `cap` bytes when the tables allow it. A
    /// document with no table rows that still exceeds `cap` is returned as
    /// it is.
    pub fn dump_within(&self, cap: usize) -> Value {
        let links = self.rows();
        let listed = links.len().min(LINK_ROWS);
        let mut last = Value::Null;
        for rows in ROW_STEPS {
            let doc = self.dump_document(&links[..listed], links.len() - listed, rows);
            if doc.to_string().len() <= cap {
                return doc;
            }
            last = doc;
        }
        last
    }

    fn dump_document(
        &self,
        links: &[std::sync::Arc<Held>],
        links_cut: usize,
        rows: usize,
    ) -> Value {
        let table: Vec<Value> = links.iter().map(|h| h.dump_row(rows)).collect();
        let cut: u64 = table
            .iter()
            .map(|r| r["streams_cut"].as_u64().unwrap_or(0))
            .sum();
        let total = |f: &dyn Fn(&Held) -> u64| links.iter().map(|h| f(h)).sum::<u64>();
        let errors: Vec<Value> = self
            .errors
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|(at, text)| json!({"at": at, "text": text}))
            .collect();
        let mut doc = json!({
            "ok": true,
            "verb": "dump",
            "schema": 1,
            "fingerprint": self.fingerprint,
            "pid": std::process::id(),
            "build": {
                "version": env!("CARGO_PKG_VERSION"),
                "features": crate::build_features::enabled(),
                "carriers": crate::carriers::ROWS
                    .iter()
                    .map(|r| json!({"name": r.name, "feature": r.feature, "built": r.built()}))
                    .collect::<Vec<_>>(),
            },
            "settings": {
                "selected": *self.selected.lock().unwrap_or_else(|e| e.into_inner()),
                "limits": {
                    "request": REQUEST_LIMIT,
                    "response": RESPONSE_LIMIT,
                    "dump": DUMP_LIMIT,
                    "clients": MAX_CLIENTS,
                    "errors": ERRORS,
                },
            },
            "totals": {
                "links": links.len(),
                "up": links.iter().filter(|h| h.link.ended().is_none()).count(),
                "streams": total(&|h| h.link.streams() as u64),
                "opened": total(&|h| h.opened.load(std::sync::atomic::Ordering::Relaxed)),
                "accepted": total(&|h| h.accepted.load(std::sync::atomic::Ordering::Relaxed)),
            },
            "links": table,
            "errors": errors,
            "routes": {"built": false, "reason": "the route table waits for the mesh (T-286)"},
            "cut": {"links": links_cut, "stream_rows": cut},
        });
        redact(&mut doc);
        doc
    }

    pub(super) fn dump_reply(&self) -> Reply {
        Reply::Big(self.dump())
    }
}
