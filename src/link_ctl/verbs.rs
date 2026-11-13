//! The verbs of the control socket that change a link (T-307, GitHub #476,
//! row E10-2): `close-link`, `close-stream` and `select`.
//!
//! They run after the token and the uid check of [`super::Control`], like
//! every verb. Each one answers the status document after the change, with
//! `verb` set to its own name, so that a client sees the result without
//! a second call. A bad or missing id exits 64; a link that cannot take the
//! change exits 69.
//!
//! `rotate` and `reconnect` need a carrier that the instance dials again.
//! An instance holds links, not the way to make them, so both answer 69
//! (`Hub::verb`).

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use super::{Answer, Held, Hub, Reply, LIMIT};
use crate::exit;

/// The verbs of the socket, as the error of an unknown verb lists them.
pub(super) const LIST: &str =
    "status, peer, open, accept, close-link, close-stream, select, dump, routes or mesh";

/// The time that `close-stream` waits for the task of the stream to take
/// the stream out of the table.
const SETTLE: Duration = Duration::from_secs(1);

/// The number under `key` in `request`. A missing key or a value that is
/// no whole number exits 64.
fn number(request: &Value, key: &str) -> Result<u64, Answer> {
    request[key].as_u64().ok_or_else(|| {
        Answer::new(
            exit::USAGE,
            format!("give {key} (the id of {}, a whole number)", noun(key)),
        )
    })
}

fn noun(key: &str) -> &'static str {
    if key == "stream" {
        "a stream"
    } else {
        "a link"
    }
}

impl Hub {
    /// The link with this id. An id that no link holds exits 64.
    fn link_of(&self, id: u64) -> Result<Arc<Held>, Answer> {
        self.rows().into_iter().find(|h| h.id == id).ok_or_else(|| {
            Answer::new(
                exit::USAGE,
                format!("this instance holds no link {id} (the verb status lists them)"),
            )
        })
    }

    /// The status document after a change by `verb`.
    fn after(&self, verb: &str) -> Reply {
        let mut doc = self.status();
        doc["verb"] = verb.into();
        Reply::Json(doc)
    }

    /// `close-link`: end the link with a GOAWAY after the frames that wait.
    /// A link that has ended already is not an error.
    pub(super) async fn close_link(&self, request: &Value) -> Result<Reply, Answer> {
        let held = self.link_of(number(request, "link")?)?;
        if tokio::time::timeout(LIMIT, held.link.close())
            .await
            .is_err()
        {
            return Err(Answer::new(
                exit::TIMEOUT,
                format!("link {} did not end in {} s", held.id, LIMIT.as_secs()),
            ));
        }
        eprintln!("podnet: link    close-link {}", held.id);
        Ok(self.after("close-link"))
    }

    /// `close-stream`: reset one stream of a link. The other streams and
    /// the link stay.
    pub(super) async fn close_stream(&self, request: &Value) -> Result<Reply, Answer> {
        let held = self.link_of(number(request, "link")?)?;
        let id = number(request, "stream")?;
        let unknown = || {
            Answer::new(
                exit::USAGE,
                format!(
                    "link {} holds no stream {id} (the verb status lists them)",
                    held.id
                ),
            )
        };
        let sid = u32::try_from(id).map_err(|_| unknown())?;
        let cancel = {
            let streams = held.streams.lock().unwrap_or_else(|e| e.into_inner());
            streams.get(&sid).cloned().ok_or_else(unknown)?
        };
        cancel.notify_one();
        // The task of the stream resets it and takes it out of the table.
        let gone = async {
            while held
                .streams
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .contains_key(&sid)
            {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        };
        let _ = tokio::time::timeout(SETTLE, gone).await;
        eprintln!("podnet: link    close-stream {} {id}", held.id);
        Ok(self.after("close-stream"))
    }

    /// `select`: pin the link that serves new streams, or `null` for the
    /// first link that is up. The streams that run stay where they are.
    pub(super) fn select(&self, request: &Value) -> Result<Reply, Answer> {
        let pin = match &request["link"] {
            Value::Null if request.get("link").is_some() => None,
            _ => {
                let held = self.link_of(number(request, "link")?)?;
                if held.link.ended().is_some() {
                    return Err(Answer::new(
                        exit::UNAVAILABLE,
                        format!("link {} has ended", held.id),
                    ));
                }
                Some(held.id)
            }
        };
        *self.selected.lock().unwrap_or_else(|e| e.into_inner()) = pin;
        eprintln!(
            "podnet: link    select {}",
            pin.map_or("auto".to_string(), |id| id.to_string())
        );
        Ok(self.after("select"))
    }
}
