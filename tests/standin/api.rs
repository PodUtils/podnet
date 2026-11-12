//! The stand-in of the quick-tunnel service (`POST /tunnel`) on the loopback.
//!
//! The body of an answer has the shape of the real service (a response
//! captured on 2026-10-09 is in the tests of `src/quicktunnel.rs`). Each
//! tunnel that it issues is kept, so that the edge can check the account tag
//! and the secret of a registration.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use uuid::Uuid;

/// What a request gets.
#[derive(Clone, Debug)]
pub enum Answer {
    /// 200 and a new tunnel.
    Issue,
    /// 429 (error 1015) with `Retry-After` when given.
    TooMany { retry_after: Option<u64> },
    /// A status with a plain body.
    Status(u16),
}

/// A tunnel that the service issued.
#[derive(Clone, Debug)]
pub struct Issued {
    pub id: Uuid,
    pub hostname: String,
    pub account_tag: String,
    pub secret: Vec<u8>,
}

/// One request that the service got.
#[derive(Clone, Debug)]
pub struct Seen {
    pub user_agent: String,
    pub body: String,
}

pub struct Api {
    script: Mutex<VecDeque<Answer>>,
    otherwise: Mutex<Answer>,
    hits: AtomicUsize,
    pub issued: Mutex<Vec<Issued>>,
    pub seen: Mutex<Vec<Seen>>,
}

impl Api {
    pub fn new() -> Arc<Api> {
        Arc::new(Api {
            script: Mutex::new(VecDeque::new()),
            otherwise: Mutex::new(Answer::Issue),
            hits: AtomicUsize::new(0),
            issued: Mutex::new(Vec::new()),
            seen: Mutex::new(Vec::new()),
        })
    }

    /// The answers of the next requests, in order. After them, the
    /// answer of [`Api::always`].
    pub fn script(&self, answers: impl IntoIterator<Item = Answer>) {
        *self.script.lock().unwrap() = answers.into_iter().collect();
    }

    /// The answer when the script is empty.
    pub fn always(&self, answer: Answer) {
        *self.otherwise.lock().unwrap() = answer;
    }

    /// The number of `POST /tunnel` requests.
    pub fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }

    pub fn issued_by_id(&self, id: &[u8]) -> Option<Issued> {
        let id = Uuid::from_slice(id).ok()?;
        self.issued
            .lock()
            .unwrap()
            .iter()
            .find(|t| t.id == id)
            .cloned()
    }

    fn next(&self) -> Answer {
        let scripted = self.script.lock().unwrap().pop_front();
        scripted.unwrap_or_else(|| self.otherwise.lock().unwrap().clone())
    }

    fn issue(&self) -> Issued {
        let mut issued = self.issued.lock().unwrap();
        let id = Uuid::new_v4();
        let mut secret = Uuid::new_v4().as_bytes().to_vec();
        secret.extend_from_slice(Uuid::new_v4().as_bytes());
        let tunnel = Issued {
            id,
            hostname: format!(
                "standin-{}-{}.trycloudflare.com",
                issued.len() + 1,
                &id.simple().to_string()[..8]
            ),
            account_tag: Uuid::new_v4().simple().to_string(),
            secret,
        };
        issued.push(tunnel.clone());
        tunnel
    }
}

/// Serve the API on a free loopback port.
pub async fn serve(api: Arc<Api>) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind the API");
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let api = api.clone();
            tokio::spawn(async move {
                let _ = handle(api, stream).await;
            });
        }
    });
    addr
}

async fn handle(api: Arc<Api>, mut stream: TcpStream) -> std::io::Result<()> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 2048];
    let head_end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let header = |name: &str| {
        head.lines().skip(1).find_map(|l| {
            let (key, value) = l.split_once(':')?;
            key.eq_ignore_ascii_case(name)
                .then(|| value.trim().to_string())
        })
    };
    let length: usize = header("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    while buf.len() < head_end + length {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let body = String::from_utf8_lossy(&buf[head_end..]).to_string();
    let first = head.lines().next().unwrap_or("");
    let reply = if first.starts_with("POST /tunnel ") {
        api.hits.fetch_add(1, Ordering::SeqCst);
        api.seen.lock().unwrap().push(Seen {
            user_agent: header("user-agent").unwrap_or_default(),
            body,
        });
        match api.next() {
            Answer::Issue => response(200, "", &tunnel_body(&api.issue())),
            Answer::TooMany { retry_after } => {
                let extra = retry_after.map_or(String::new(), |s| format!("Retry-After: {s}\r\n"));
                response(429, &extra, "error code: 1015")
            }
            Answer::Status(code) => response(code, "", "stand-in status"),
        }
    } else {
        response(404, "", "not found")
    };
    stream.write_all(reply.as_bytes()).await?;
    stream.shutdown().await
}

fn response(status: u16, extra: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status} Stand-in\r\nContent-Type: application/json\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

/// The body of the real service: the fields at the top and in `result`.
fn tunnel_body(t: &Issued) -> String {
    let secret = podnet::cloudflare::http2::base64_encode(&t.secret);
    let fields = serde_json::json!({
        "id": t.id.to_string(),
        "name": t.hostname.trim_end_matches(".trycloudflare.com"),
        "hostname": t.hostname,
        "account_tag": t.account_tag,
        "secret": secret,
    });
    let mut body = fields.clone();
    body["success"] = true.into();
    body["result"] = fields;
    body["errors"] = serde_json::json!([]);
    body["messages"] = serde_json::json!([]);
    body.to_string()
}
