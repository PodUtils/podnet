//! The stand-ins of the transfer tests, on the loopback: a mailbox that
//! speaks the server protocol of magic-wormhole's mailbox, version 1, and a
//! transit relay of magic-wormhole over TCP. `pod-wormhole` keeps its own
//! stand-in in its tests, which a dependent crate cannot use; this one has
//! the commands that a client sends: `bind`, `list`, `allocate`, `claim`,
//! `open`, `add`, `release` and `close`, an `ack` for each one, at most two
//! sides on a nameplate (`crowded` for a third), and each message sent to
//! each side that has the mailbox open, the sender's too.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use pod_proxy::http::request;
use pod_ws::{frame, server};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};
use tokio::task::JoinHandle;

type Tx = UnboundedSender<String>;

#[derive(Default)]
struct Mbox {
    messages: Vec<Value>,
    open: Vec<(String, Tx)>,
}

#[derive(Default)]
struct State {
    /// Each nameplate, the sides that claimed it and its mailbox, of each
    /// AppID. A nameplate that comes again gets a new mailbox.
    plates: HashMap<String, BTreeMap<String, (BTreeSet<String>, String)>>,
    counter: u64,
    boxes: HashMap<String, Mbox>,
    moods: Vec<String>,
}

/// A stand-in mailbox on a free port of the loopback.
pub struct Mailbox {
    /// `ws://127.0.0.1:PORT/v1`.
    pub url: String,
    state: Arc<Mutex<State>>,
    /// The connections that it took.
    pub connections: Arc<AtomicUsize>,
    task: JoinHandle<()>,
}

impl Mailbox {
    pub async fn start() -> Mailbox {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a port of the loopback");
        let port = listener.local_addr().unwrap().port();
        let state = Arc::new(Mutex::new(State::default()));
        let connections = Arc::new(AtomicUsize::new(0));
        let (shared, counted) = (state.clone(), connections.clone());
        let task = tokio::spawn(async move {
            while let Ok((tcp, _)) = listener.accept().await {
                counted.fetch_add(1, Ordering::SeqCst);
                tokio::spawn(serve(tcp, shared.clone()));
            }
        });
        Mailbox {
            url: format!("ws://127.0.0.1:{port}/v1"),
            state,
            connections,
            task,
        }
    }

    /// The moods of each close so far.
    pub fn moods(&self) -> Vec<String> {
        self.state.lock().unwrap().moods.clone()
    }

    /// The nameplates held now, of each AppID.
    pub fn nameplates(&self) -> usize {
        self.state
            .lock()
            .unwrap()
            .plates
            .values()
            .map(BTreeMap::len)
            .sum()
    }
}

impl Drop for Mailbox {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn serve(mut tcp: TcpStream, state: Arc<Mutex<State>>) {
    let Ok((head, pending)) = request::read_request(&mut tcp, Vec::new()).await else {
        return;
    };
    let Ok(key) = server::check_upgrade(&head) else {
        return;
    };
    if tcp.write_all(&server::switching(&key)).await.is_err() {
        return;
    }
    let session = Arc::new(server::session(tcp, pending, None));
    let (tx, mut rx) = unbounded_channel::<String>();
    let writer = {
        let session = session.clone();
        tokio::spawn(async move {
            while let Some(text) = rx.recv().await {
                if session.send_text(&text).await.is_err() {
                    break;
                }
            }
        })
    };
    let _ = tx.send(json!({ "type": "welcome", "welcome": {} }).to_string());
    let (mut appid, mut side) = (String::new(), String::new());
    while let Ok(f) = session.read_frame().await {
        if f.opcode == frame::OPCODE_CLOSE {
            break;
        }
        if f.opcode != frame::OPCODE_TEXT {
            continue;
        }
        let Ok(command) = serde_json::from_slice::<Value>(&f.payload) else {
            break;
        };
        let _ = tx.send(json!({ "type": "ack", "id": command["id"] }).to_string());
        let text = |key: &str| command[key].as_str().unwrap_or_default().to_string();
        let mut state = state.lock().unwrap();
        let reply = match text("type").as_str() {
            "bind" => {
                (appid, side) = (text("appid"), text("side"));
                None
            }
            "list" => {
                let plates = state.plates.entry(appid.clone()).or_default();
                let all: Vec<Value> = plates.keys().map(|id| json!({ "id": id })).collect();
                Some(json!({ "type": "nameplates", "nameplates": all }))
            }
            "allocate" => {
                let plates = state.plates.entry(appid.clone()).or_default();
                let free = (1u32..)
                    .map(|n| n.to_string())
                    .find(|n| !plates.contains_key(n))
                    .unwrap();
                state.counter += 1;
                let id = format!("mb{}", state.counter);
                let plates = state.plates.entry(appid.clone()).or_default();
                plates.insert(free.clone(), ([side.clone()].into(), id));
                Some(json!({ "type": "allocated", "nameplate": free }))
            }
            "claim" => {
                state.counter += 1;
                let fresh = format!("mb{}", state.counter);
                let plates = state.plates.entry(appid.clone()).or_default();
                let plate = plates
                    .entry(text("nameplate"))
                    .or_insert_with(|| (BTreeSet::new(), fresh));
                if !plate.0.contains(&side) && plate.0.len() >= 2 {
                    Some(json!({ "type": "error", "error": "crowded", "orig": command }))
                } else {
                    plate.0.insert(side.clone());
                    let id = plate.1.clone();
                    state.boxes.entry(id.clone()).or_default();
                    Some(json!({ "type": "claimed", "mailbox": id }))
                }
            }
            "open" => {
                if let Some(mailbox) = state.boxes.get_mut(&text("mailbox")) {
                    for message in &mailbox.messages {
                        let _ = tx.send(message.to_string());
                    }
                    mailbox.open.push((side.clone(), tx.clone()));
                }
                None
            }
            "add" => {
                let message = json!({ "type": "message", "side": side, "phase": text("phase"), "body": text("body"), "id": "0" });
                if let Some(mailbox) = state
                    .boxes
                    .values_mut()
                    .find(|m| m.open.iter().any(|(s, _)| *s == side))
                {
                    mailbox.messages.push(message.clone());
                    for (_, other) in &mailbox.open {
                        let _ = other.send(message.to_string());
                    }
                }
                None
            }
            "release" => {
                let plates = state.plates.entry(appid.clone()).or_default();
                if let Some(plate) = plates.get_mut(&text("nameplate")) {
                    plate.0.remove(&side);
                    if plate.0.is_empty() {
                        plates.remove(&text("nameplate"));
                    }
                }
                Some(json!({ "type": "released" }))
            }
            "close" => {
                state.moods.push(text("mood"));
                if let Some(mailbox) = state.boxes.get_mut(&text("mailbox")) {
                    mailbox.open.retain(|(s, _)| *s != side);
                }
                Some(json!({ "type": "closed" }))
            }
            _ => Some(json!({ "type": "error", "error": "unknown command", "orig": command })),
        };
        drop(state);
        if let Some(reply) = reply {
            let _ = tx.send(reply.to_string());
        }
    }
    writer.abort();
}

/// A stand-in transit relay of magic-wormhole over TCP: two connections
/// with one token and two sides get `ok\n`, then each one's bytes go to the
/// other.
pub struct Relay {
    /// `tcp:127.0.0.1:PORT`.
    pub url: String,
    /// The pairs that it joined.
    pub joined: Arc<AtomicUsize>,
    task: JoinHandle<()>,
}

impl Relay {
    pub async fn start() -> Relay {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a port of the loopback");
        let port = listener.local_addr().unwrap().port();
        let waiting: Arc<Mutex<HashMap<String, (String, TcpStream)>>> = Arc::default();
        let joined = Arc::new(AtomicUsize::new(0));
        let counted = joined.clone();
        let task = tokio::spawn(async move {
            while let Ok((mut tcp, _)) = listener.accept().await {
                let (waiting, counted) = (waiting.clone(), counted.clone());
                tokio::spawn(async move {
                    let Some(line) = read_line(&mut tcp).await else {
                        return;
                    };
                    let Some(rest) = line.strip_prefix("please relay ") else {
                        return;
                    };
                    let Some((token, side)) = rest.split_once(" for side ") else {
                        return;
                    };
                    let other = {
                        let mut waiting = waiting.lock().unwrap();
                        match waiting.remove(token) {
                            Some((their_side, other)) if their_side != side => Some(other),
                            Some(same) => {
                                waiting.insert(token.to_string(), same);
                                None
                            }
                            None => {
                                waiting.insert(token.to_string(), (side.to_string(), tcp));
                                return;
                            }
                        }
                    };
                    let Some(mut other) = other else { return };
                    counted.fetch_add(1, Ordering::SeqCst);
                    if other.write_all(b"ok\n").await.is_err()
                        || tcp.write_all(b"ok\n").await.is_err()
                    {
                        return;
                    }
                    let _ = tokio::io::copy_bidirectional(&mut tcp, &mut other).await;
                });
            }
        });
        Relay {
            url: format!("tcp:127.0.0.1:{port}"),
            joined,
            task,
        }
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn read_line(tcp: &mut TcpStream) -> Option<String> {
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    while line.len() < 256 {
        tcp.read_exact(&mut byte).await.ok()?;
        if byte[0] == b'\n' {
            return String::from_utf8(line).ok();
        }
        line.push(byte[0]);
    }
    None
}

/// A free port of the loopback where nothing listens.
pub fn nobody_there() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    format!(
        "ws://127.0.0.1:{}/v1",
        listener.local_addr().unwrap().port()
    )
}
