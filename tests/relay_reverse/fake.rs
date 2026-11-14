//! The fake relay of the tests of the reverse mode: the reverse protocol of
//! the relay on loopback.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::{CONNECT_TOKEN, NAME, NODE_TOKEN};

use futures::{SinkExt as _, StreamExt as _};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::Message;

#[derive(Default)]
pub(crate) struct State {
    /// The messages to the node socket of now.
    pub(crate) node: Option<mpsc::UnboundedSender<Message>>,
    /// The messages to each operator socket, by session id.
    pub(crate) operators: HashMap<String, mpsc::UnboundedSender<Message>>,
    /// How many node sockets came.
    pub(crate) node_sockets: usize,
    /// Drop the first node socket after this many ms.
    pub(crate) drop_first_after: Option<u64>,
    /// How many operator sockets came (each one a relay session).
    pub(crate) operator_sockets: usize,
    /// The cap of the bytes of one relay session, both ways (the relay has
    /// 64 MiB): at the cap, the relay closes the operator socket with 1009.
    pub(crate) cap: Option<usize>,
    /// The bytes of each relay session, both ways, by session id.
    pub(crate) carried: HashMap<String, usize>,
    /// How many relay sessions met the cap.
    pub(crate) capped: usize,
    /// With `Some`, each byte of a binary message that the relay carries,
    /// both ways (the payload of a node frame without its id): what the
    /// relay sees.
    pub(crate) tap: Option<Vec<u8>>,
}

/// Keep `bytes` in the tap, when there is one.
fn tap(state: &Mutex<State>, bytes: &[u8]) {
    if let Some(tap) = state.lock().unwrap().tap.as_mut() {
        tap.extend_from_slice(bytes);
    }
}

/// Count `bytes` of the session `id`. At the cap, the relay closes the
/// operator socket with 1009 and forgets the session; then false.
fn count(state: &Mutex<State>, id: &str, bytes: usize) -> bool {
    let mut state = state.lock().unwrap();
    let Some(cap) = state.cap else {
        return true;
    };
    let carried = state.carried.entry(id.to_string()).or_default();
    *carried += bytes;
    if *carried <= cap {
        return true;
    }
    if let Some(operator) = state.operators.remove(id) {
        state.capped += 1;
        let _ = operator.send(Message::Close(Some(CloseFrame {
            code: CloseCode::from(1009),
            reason: "session byte limit".into(),
        })));
    }
    false
}

/// A relay on loopback with the reverse protocol of tcp.ssh.relay.ajam.dev:
/// `/v1/node/NAME` (token NODE_TOKEN) and `/v1/connect/NAME` (token
/// CONNECT_TOKEN).
pub(crate) async fn fake_relay(drop_first_after: Option<u64>) -> (String, Arc<Mutex<State>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let state = Arc::new(Mutex::new(State {
        drop_first_after,
        ..State::default()
    }));
    let shared = state.clone();
    tokio::spawn(async move {
        let mut next_id = 0u32;
        while let Ok((socket, _)) = listener.accept().await {
            next_id += 1;
            let state = shared.clone();
            let id = format!("{next_id:032x}");
            tokio::spawn(serve(socket, state, id));
        }
    });
    (url, state)
}

#[allow(clippy::result_large_err)] // the callback type of tungstenite
async fn serve(socket: tokio::net::TcpStream, state: Arc<Mutex<State>>, id: String) {
    use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
    let path = Arc::new(Mutex::new(String::new()));
    let seen = path.clone();
    let check = move |request: &Request, response: Response| {
        let token = request
            .headers()
            .get("x-relay-token")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let path = request.uri().path().to_string();
        let good = (path.starts_with("/v1/node/") && token == NODE_TOKEN)
            || (path == format!("/v1/connect/{NAME}") && token == CONNECT_TOKEN);
        *seen.lock().unwrap() = path;
        if good {
            Ok(response)
        } else {
            let mut refused = ErrorResponse::new(Some("missing or wrong token".into()));
            *refused.status_mut() = http::StatusCode::FORBIDDEN;
            Err(refused)
        }
    };
    let Ok(ws) = tokio_tungstenite::accept_hdr_async(socket, check).await else {
        return;
    };
    let path = path.lock().unwrap().clone();
    let (mut sink, mut stream) = ws.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
    let writer = tokio::spawn(async move {
        while let Some(message) = rx.recv().await {
            let close = matches!(message, Message::Close(_));
            if sink.send(message).await.is_err() || close {
                break;
            }
        }
    });
    if path.starts_with("/v1/node/") {
        let drop_after = {
            let mut state = state.lock().unwrap();
            state.node = Some(tx.clone());
            state.node_sockets += 1;
            if state.node_sockets == 1 {
                state.drop_first_after
            } else {
                None
            }
        };
        let _ = tx.send(Message::text(r#"{"type":"hello"}"#));
        if let Some(ms) = drop_after {
            let tx = tx.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(ms)).await;
                let _ = tx.send(Message::Close(Some(CloseFrame {
                    code: CloseCode::from(1011),
                    reason: "socket error".into(),
                })));
            });
        }
        while let Some(Ok(message)) = stream.next().await {
            match message {
                Message::Text(text) => {
                    let doc: serde_json::Value = serde_json::from_str(text.as_str()).unwrap();
                    let id = doc["id"].as_str().unwrap_or("").to_string();
                    let operator = state.lock().unwrap().operators.get(&id).cloned();
                    let Some(operator) = operator else { continue };
                    match doc["type"].as_str() {
                        Some("ready") => {
                            // The relay gives the operator the ready of the node
                            // with its id, as `pod-relay` reads it.
                            let _ = operator.send(Message::text(text.to_string()));
                        }
                        Some("reject") => {
                            let _ = operator.send(Message::text(text.to_string()));
                            let _ = operator.send(Message::Close(Some(CloseFrame {
                                code: CloseCode::from(1011),
                                reason: "reject".into(),
                            })));
                        }
                        Some("close") => {
                            let _ = operator.send(Message::Close(Some(CloseFrame {
                                code: CloseCode::Normal,
                                reason: "close".into(),
                            })));
                        }
                        _ => {}
                    }
                }
                Message::Binary(data) => {
                    assert!(data.len() > 32, "a node frame has the id first");
                    let id = std::str::from_utf8(&data[..32]).unwrap().to_string();
                    let operator = state.lock().unwrap().operators.get(&id).cloned();
                    if !count(&state, &id, data.len() - 32) {
                        continue;
                    }
                    tap(&state, &data[32..]);
                    if let Some(operator) = operator {
                        let _ = operator.send(Message::binary(data[32..].to_vec()));
                    }
                }
                _ => {}
            }
        }
    } else {
        {
            let mut state = state.lock().unwrap();
            state.operators.insert(id.clone(), tx.clone());
            state.operator_sockets += 1;
        }
        let node = state.lock().unwrap().node.clone();
        let Some(node) = node else {
            let _ = tx.send(Message::Close(Some(CloseFrame {
                code: CloseCode::from(1011),
                reason: "node offline".into(),
            })));
            return;
        };
        let _ = node.send(Message::text(format!(r#"{{"type":"open","id":"{id}"}}"#)));
        while let Some(Ok(message)) = stream.next().await {
            match message {
                Message::Binary(data) => {
                    if !count(&state, &id, data.len()) {
                        continue;
                    }
                    tap(&state, &data);
                    let mut frame = id.as_bytes().to_vec();
                    frame.extend_from_slice(&data);
                    let _ = node.send(Message::binary(frame));
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
        let _ = node.send(Message::text(format!(r#"{{"type":"close","id":"{id}"}}"#)));
        let _ = tx.send(Message::Close(None));
    }
    let _ = writer.await;
}
