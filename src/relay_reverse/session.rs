//! One session of the node: the messages to the relay, the state of each
//! session on the node socket (the gate), and the task of a session.

use std::collections::{HashMap, HashSet};

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use super::{Target, FRAME, ID_LEN, LOCAL_CONNECT};

/// After the target shuts down its write side, the node waits this long for
/// a frame of the operator before it ends the session (the relay or a
/// failed write can end it before; T-196).
const HALF_CLOSE_IDLE: std::time::Duration = std::time::Duration::from_secs(2);

/// A text control message of the node protocol.
pub(super) fn control(kind: &str, id: &str, reason: Option<&str>) -> Message {
    let mut doc = serde_json::json!({ "type": kind, "id": id });
    if let Some(reason) = reason {
        doc["reason"] = reason.into();
    }
    Message::text(doc.to_string())
}

pub(super) fn valid_id(id: &str) -> bool {
    id.len() == ID_LEN
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The frames that a session of the node can queue for one session.
pub(super) const SESSION_QUEUE: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum OutKind {
    Ready,
    Data,
    /// `close` or `reject`: the last message of a session.
    End,
}

/// A message of a session task to the relay.
pub(super) struct Out {
    pub(super) id: String,
    pub(super) kind: OutKind,
    pub(super) message: Message,
}

impl Out {
    pub(super) fn ready(id: &str) -> Self {
        Self {
            id: id.into(),
            kind: OutKind::Ready,
            message: control("ready", id, None),
        }
    }

    pub(super) fn data(id: &str, frame: Vec<u8>) -> Self {
        Self {
            id: id.into(),
            kind: OutKind::Data,
            message: Message::binary(frame),
        }
    }

    pub(super) fn end(id: &str, kind: &str, reason: Option<&str>) -> Self {
        Self {
            id: id.into(),
            kind: OutKind::End,
            message: control(kind, id, reason),
        }
    }
}

/// The data queue of each session, and its task.
pub(super) type Sessions = HashMap<String, (mpsc::Sender<Vec<u8>>, tokio::task::AbortHandle)>;

/// Remove a session. The task stops: a task in a write to a target that does
/// not read does not stop when its sender drops.
pub(super) fn end_session(sessions: &mut Sessions, id: &str) {
    if let Some((_, task)) = sessions.remove(id) {
        task.abort();
    }
}

/// The state of each session on the node socket. The relay closes the
/// whole node socket for a frame before `ready` or for an id that is not
/// open (`1003`), so the writer sends a frame only in the correct state.
#[derive(Default)]
pub(super) struct Gate {
    /// Opened by the relay; no `ready` yet.
    opening: HashSet<String>,
    /// `ready` sent; data can go.
    open: HashSet<String>,
}

impl Gate {
    /// The relay opened `id`. False when `id` is open already.
    pub(super) fn opened(&mut self, id: &str) -> bool {
        !self.open.contains(id) && self.opening.insert(id.to_string())
    }

    /// The relay closed `id`: nothing more goes for it.
    pub(super) fn closed_by_relay(&mut self, id: &str) {
        self.opening.remove(id);
        self.open.remove(id);
    }

    /// Whether `out` can go to the relay now.
    pub(super) fn pass(&mut self, out: &Out) -> bool {
        match out.kind {
            OutKind::Ready => {
                let ok = self.opening.remove(&out.id);
                if ok {
                    self.open.insert(out.id.clone());
                }
                ok
            }
            OutKind::Data => self.open.contains(&out.id),
            OutKind::End => self.opening.remove(&out.id) | self.open.remove(&out.id),
        }
    }
}

/// One session of the node: connect to the target, then copy bytes until
/// one side ends. A `close` goes at each end.
pub(super) async fn session(
    target: Target,
    id: String,
    mut data_rx: mpsc::Receiver<Vec<u8>>,
    out: mpsc::Sender<Out>,
) {
    let io = match tokio::time::timeout(LOCAL_CONNECT, target.connect()).await {
        Ok(Ok(io)) => io,
        Ok(Err(err)) => {
            let reason = format!("{target}: {err:#}");
            tracing::info!(%id, %reason, "a session is refused");
            let _ = out.send(Out::end(&id, "reject", Some(&reason))).await;
            return;
        }
        Err(_) => {
            let reason = format!("{target}: no answer in {} s", LOCAL_CONNECT.as_secs());
            let _ = out.send(Out::end(&id, "reject", Some(&reason))).await;
            return;
        }
    };
    if out.send(Out::ready(&id)).await.is_err() {
        return;
    }
    tracing::info!(%id, %target, "a session is open");
    let (mut read, mut write) = tokio::io::split(io);
    // The read side sets this when the target shuts down its write side.
    let (half_tx, mut half_rx) = tokio::sync::watch::channel(false);
    let to_relay = {
        let out = out.clone();
        let id = id.clone();
        async move {
            let mut buf = vec![0u8; FRAME];
            loop {
                let n = match read.read(&mut buf).await {
                    Ok(0) => return ReadEnd::Eof,
                    Err(err) => {
                        return ReadEnd::Done(Some(format!(
                            "the read of the local side failed: {err}"
                        )))
                    }
                    Ok(n) => n,
                };
                let mut frame = Vec::with_capacity(ID_LEN + n);
                frame.extend_from_slice(id.as_bytes());
                frame.extend_from_slice(&buf[..n]);
                if out.send(Out::data(&id, frame)).await.is_err() {
                    return ReadEnd::Done(None);
                }
            }
        }
    };
    // After the half close, the task ends when no frame comes for
    // HALF_CLOSE_IDLE.
    let to_local = {
        let id = id.clone();
        async move {
            loop {
                let after_half_close = async {
                    let _ = half_rx.wait_for(|half| *half).await;
                    tokio::time::sleep(HALF_CLOSE_IDLE).await;
                };
                let data = tokio::select! {
                    data = data_rx.recv() => data,
                    () = after_half_close => {
                        tracing::info!(%id, "no frame after the target closed its write side");
                        let _ = write.shutdown().await;
                        return None;
                    }
                };
                let Some(data) = data else { break };
                if let Err(err) = write.write_all(&data).await {
                    return Some(format!("the write to the local side failed: {err}"));
                }
            }
            let _ = write.shutdown().await;
            None
        }
    };
    tokio::pin!(to_relay, to_local);
    let reason = tokio::select! {
        ended = &mut to_relay => match ended {
            ReadEnd::Done(reason) => reason,
            // No `close` here: the operator can still send. The queued
            // frames go to the target first.
            ReadEnd::Eof => {
                tracing::info!(%id, "the target closed its write side");
                let _ = half_tx.send(true);
                to_local.await
            }
        },
        reason = &mut to_local => reason,
    };
    // After a close of the relay, the gate drops this close.
    let _ = out.send(Out::end(&id, "close", reason.as_deref())).await;
    tracing::info!(%id, reason = reason.as_deref().unwrap_or("an end of the stream"), "a session ended");
}

/// How the read side of a session ended.
enum ReadEnd {
    /// The target shut down its write side.
    Eof,
    /// The session ends, with a reason when it is a fault.
    Done(Option<String>),
}
