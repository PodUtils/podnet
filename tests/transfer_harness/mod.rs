//! The stand-ins of the transfer tests, on the loopback: the mailbox of
//! magic-wormhole's server protocol, version 1 (`pod_wormhole::standin`,
//! the feature `standin` of `pod-wormhole`), and a transit relay of
//! magic-wormhole over TCP, which no crate gives.

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use pod_wormhole::standin::{self, Stand};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

/// A stand-in mailbox on a free port of the loopback.
pub struct Mailbox {
    /// `ws://127.0.0.1:PORT/v1`.
    pub url: String,
    stand: Stand,
}

impl Mailbox {
    pub async fn start() -> Mailbox {
        let stand = standin::start().await.expect("a port of the loopback");
        Mailbox {
            url: format!("ws://127.0.0.1:{}/v1", stand.url.port),
            stand,
        }
    }

    /// The moods of each close so far.
    pub fn moods(&self) -> Vec<String> {
        self.stand
            .moods()
            .into_iter()
            .map(|(_, mood)| mood)
            .collect()
    }

    /// The nameplates held now, of the AppID of the file transfer.
    pub fn nameplates(&self) -> usize {
        self.stand.nameplates(podnet::transfer::APPID).len()
    }
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
