//! The stand-in of the Cloudflare edge, HTTP/2 leg: TLS on TCP with no
//! ALPN, the edge as the HTTP/2 client, the control stream with the
//! `registerConnection` RPC, and one stream for each visitor request.
//!
//! The code is written from the client of podnet (`src/edge_h2.rs`,
//! `src/cloudflare/rpc.rs`) and the schema of cloudflared. No code of
//! another project is in it.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use podnet::cloudflare::capnp::{Builder, Message};
use podnet::cloudflare::http2::{decode_origin_response, CONTROL_STREAM_HEADER};
use podnet::cloudflare::schema::{METHOD_UNREGISTER_CONNECTION, REGISTRATION_SERVER_ID};
use podnet::edge_h2::{body_reader, body_writer};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use uuid::Uuid;

use super::api::Api;

// rpc.capnp: the tags of the unions that the RPC uses.
const MSG_CALL: u16 = 2;
const MSG_RETURN: u16 = 3;
const MSG_BOOTSTRAP: u16 = 8;
const CAP_SENDER_HOSTED: u16 = 1;
/// The capability that the edge exports.
const EDGE_EXPORT: u32 = 42;

/// What the edge answers to `registerConnection`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    /// Accept a tunnel that the stand-in API issued.
    Accept,
    /// Refuse with a cause that is not retryable.
    Refuse(String),
}

/// One `registerConnection` that the edge got.
#[derive(Clone, Debug)]
pub struct Registration {
    pub conn_index: u8,
    pub account_tag: String,
    pub secret: Vec<u8>,
    pub tunnel_id: Uuid,
    pub features: Vec<String>,
    pub accepted: bool,
}

/// What a visitor got through a tunnel.
#[derive(Debug)]
pub struct Visit {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

type Sender = h2::client::SendRequest<Bytes>;

pub struct Edge {
    api: Arc<Api>,
    reply: Mutex<Reply>,
    /// The tunnels with a control stream, by hostname: (connection, sender).
    tunnels: Mutex<HashMap<String, Vec<(usize, Sender)>>>,
    pub registrations: Mutex<Vec<Registration>>,
    pub unregistrations: AtomicUsize,
    next_conn: AtomicUsize,
}

impl Edge {
    pub fn new(api: Arc<Api>) -> Arc<Edge> {
        Arc::new(Edge {
            api,
            reply: Mutex::new(Reply::Accept),
            tunnels: Mutex::new(HashMap::new()),
            registrations: Mutex::new(Vec::new()),
            unregistrations: AtomicUsize::new(0),
            next_conn: AtomicUsize::new(0),
        })
    }

    /// What the next registrations get.
    pub fn answer(&self, reply: Reply) {
        *self.reply.lock().unwrap() = reply;
    }

    /// Wait until `hostname` has a control stream.
    pub async fn wait_registered(&self, hostname: &str, within: Duration) -> bool {
        let end = tokio::time::Instant::now() + within;
        while tokio::time::Instant::now() < end {
            if self.connected(hostname) > 0 {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        false
    }

    /// The number of control streams of `hostname`.
    pub fn connected(&self, hostname: &str) -> usize {
        self.tunnels
            .lock()
            .unwrap()
            .get(hostname)
            .map_or(0, Vec::len)
    }

    /// Send a visitor request down a stream of the tunnel `hostname`.
    pub async fn visit(
        &self,
        hostname: &str,
        method: &str,
        path: &str,
        headers: &[(String, String)],
        body: Vec<u8>,
    ) -> Result<Visit, String> {
        let sender = {
            let tunnels = self.tunnels.lock().unwrap();
            tunnels
                .get(hostname)
                .and_then(|list| list.first())
                .map(|(_, sender)| sender.clone())
        }
        .ok_or_else(|| format!("no tunnel for {hostname}"))?;
        let mut request = http::Request::builder()
            .method(method)
            .uri(format!("https://{hostname}{path}"))
            .header("x-forwarded-proto", "https");
        for (name, value) in headers {
            request = request.header(name.as_str(), value.as_str());
        }
        let request = request.body(()).map_err(|e| e.to_string())?;
        let mut sender = sender.ready().await.map_err(|e| e.to_string())?;
        let (response, mut stream) = sender
            .send_request(request, body.is_empty())
            .map_err(|e| e.to_string())?;
        if !body.is_empty() {
            stream
                .send_data(Bytes::from(body), true)
                .map_err(|e| e.to_string())?;
        }
        let response = response.await.map_err(|e| e.to_string())?;
        let status = response.status().as_u16();
        let wire: Vec<(String, String)> = response
            .headers()
            .iter()
            .map(|(n, v)| {
                (
                    n.to_string(),
                    String::from_utf8_lossy(v.as_bytes()).to_string(),
                )
            })
            .collect();
        let mut headers = decode_origin_response(&wire);
        if headers.is_empty() {
            headers = wire;
        }
        let mut incoming = response.into_body();
        let mut body = Vec::new();
        while let Some(chunk) = incoming.data().await {
            let chunk = chunk.map_err(|e| e.to_string())?;
            let _ = incoming.flow_control().release_capacity(chunk.len());
            body.extend_from_slice(&chunk);
        }
        Ok(Visit {
            status,
            headers,
            body,
        })
    }
}

/// Accept connections of podnet on `listener`.
pub fn serve(edge: Arc<Edge>, listener: TcpListener, tls: Arc<rustls::ServerConfig>) {
    let acceptor = TlsAcceptor::from(tls);
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let edge = edge.clone();
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                if let Ok(tls) = acceptor.accept(tcp).await {
                    let _ = connection(edge, tls).await;
                }
            });
        }
    });
}

/// The address of a listener on the loopback.
pub async fn listen() -> (TcpListener, SocketAddr) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind the edge");
    let addr = listener.local_addr().unwrap();
    (listener, addr)
}

async fn connection<T>(edge: Arc<Edge>, io: T) -> Result<(), Box<dyn std::error::Error>>
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (mut sender, driver) = h2::client::handshake(io).await?;
    let driver = tokio::spawn(driver);
    let request = http::Request::builder()
        .method("POST")
        .uri("https://h2.cftunnel.com/")
        .header(CONTROL_STREAM_HEADER, "control-stream")
        .body(())?;
    let (response, body) = sender.clone().ready().await?.send_request(request, false)?;
    let response = response.await?;
    let reader = body_reader(response.into_body());
    let writer = body_writer(body);
    let conn_id = edge.next_conn.fetch_add(1, Ordering::SeqCst);
    let mut hostname = None;
    let result = rpc(&edge, reader, writer, &mut sender, conn_id, &mut hostname).await;
    if let Some(hostname) = hostname {
        let mut tunnels = edge.tunnels.lock().unwrap();
        if let Some(list) = tunnels.get_mut(&hostname) {
            list.retain(|(id, _)| *id != conn_id);
            if list.is_empty() {
                tunnels.remove(&hostname);
            }
        }
    }
    driver.abort();
    result
}

/// The RPC of the control stream, on the side of the edge.
async fn rpc(
    edge: &Edge,
    mut read: tokio::io::DuplexStream,
    mut write: tokio::io::DuplexStream,
    sender: &mut Sender,
    conn_id: usize,
    hostname: &mut Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    // The edge asks for the bootstrap of podnet (question 7).
    let mut b = Builder::new();
    let root = b.init_root(1, 1);
    b.set_u16(root, 0, MSG_BOOTSTRAP);
    let boot = b.init_struct(root, 0, 1, 1);
    b.set_u32(boot, 0, 7);
    write.write_all(&b.to_bytes()).await?;
    while let Some(message) = Message::read(&mut read).await? {
        let root = message.root()?;
        match root.get_u16(0) {
            MSG_BOOTSTRAP => {
                let question = root.get_struct(0)?.ok_or("empty bootstrap")?.get_u32(0);
                let mut b = Builder::new();
                let r = b.init_root(1, 1);
                b.set_u16(r, 0, MSG_RETURN);
                let ret = b.init_struct(r, 0, 2, 1);
                b.set_u32(ret, 0, question);
                let payload = b.init_struct(ret, 0, 0, 2);
                b.set_capability(payload, 0, 0);
                let table = b.init_struct_list(payload, 1, 1, 1, 1);
                b.set_u16(table[0], 0, CAP_SENDER_HOSTED);
                b.set_u32(table[0], 1, EDGE_EXPORT);
                write.write_all(&b.to_bytes()).await?;
            }
            MSG_CALL => {
                let call = root.get_struct(0)?.ok_or("empty call")?;
                let question = call.get_u32(0);
                if call.get_u64(1) != REGISTRATION_SERVER_ID {
                    continue;
                }
                let reply = if call.get_u16(2) == METHOD_UNREGISTER_CONNECTION {
                    edge.unregistrations.fetch_add(1, Ordering::SeqCst);
                    ReplyBody::Empty
                } else {
                    let params = call
                        .get_struct(1)?
                        .ok_or("no payload")?
                        .get_struct(0)?
                        .ok_or("no parameters")?;
                    register(edge, &params, sender, conn_id, hostname)?
                };
                write.write_all(&return_message(question, &reply)).await?;
            }
            // Return (the answer to the bootstrap), Finish and Release.
            _ => {}
        }
    }
    Ok(())
}

enum ReplyBody {
    Empty,
    Details(String),
    Refused(String),
}

fn register(
    edge: &Edge,
    params: &podnet::cloudflare::capnp::StructReader<'_>,
    sender: &Sender,
    conn_id: usize,
    hostname: &mut Option<String>,
) -> Result<ReplyBody, Box<dyn std::error::Error>> {
    let auth = params.get_struct(0)?.ok_or("no auth")?;
    let client = params
        .get_struct(2)?
        .ok_or("no options")?
        .get_struct(0)?
        .ok_or("no client info")?;
    let account_tag = auth.get_text(0)?;
    let secret = auth.get_data(1)?;
    let tunnel_id = params.get_data(1)?;
    let issued = edge.api.issued_by_id(&tunnel_id);
    let valid = issued
        .as_ref()
        .is_some_and(|t| t.account_tag == account_tag && t.secret == secret);
    let reply = edge.reply.lock().unwrap().clone();
    let accepted = valid && reply == Reply::Accept;
    edge.registrations.lock().unwrap().push(Registration {
        conn_index: params.get_u8(0),
        account_tag,
        secret,
        tunnel_id: Uuid::from_slice(&tunnel_id).unwrap_or_default(),
        features: client.get_text_list(1)?,
        accepted,
    });
    if let (true, Some(issued)) = (accepted, issued) {
        edge.tunnels
            .lock()
            .unwrap()
            .entry(issued.hostname.clone())
            .or_default()
            .push((conn_id, sender.clone()));
        *hostname = Some(issued.hostname);
        return Ok(ReplyBody::Details("standin1".into()));
    }
    Ok(ReplyBody::Refused(match reply {
        Reply::Refuse(cause) => cause,
        Reply::Accept => "the tunnel or its secret is not known".into(),
    }))
}

/// A `Return` for `question`: `ConnectionResponse` or an empty result.
fn return_message(question: u32, body: &ReplyBody) -> Vec<u8> {
    let mut b = Builder::new();
    let r = b.init_root(1, 1);
    b.set_u16(r, 0, MSG_RETURN);
    let ret = b.init_struct(r, 0, 2, 1);
    b.set_u32(ret, 0, question);
    let payload = b.init_struct(ret, 0, 0, 2);
    match body {
        ReplyBody::Empty => {
            b.init_struct(payload, 0, 0, 0);
        }
        ReplyBody::Details(location) => {
            let results = b.init_struct(payload, 0, 0, 1);
            let response = b.init_struct(results, 0, 1, 1);
            b.set_u16(response, 0, 1);
            let details = b.init_struct(response, 0, 1, 2);
            b.set_data(details, 0, Uuid::new_v4().as_bytes());
            b.set_text(details, 1, location);
        }
        ReplyBody::Refused(cause) => {
            let results = b.init_struct(payload, 0, 0, 1);
            let response = b.init_struct(results, 0, 1, 1);
            b.set_u16(response, 0, 0);
            // ConnectionError: cause, retryAfter, shouldRetry = false.
            let error = b.init_struct(response, 0, 2, 1);
            b.set_text(error, 0, cause);
        }
    }
    b.to_bytes()
}
