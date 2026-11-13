//! A piping server as the last way of the transit (T-252): two ends of
//! podnet meet at a stand-in mailbox, and the sealed records go through a
//! stand-in piping server on the loopback that gives the body of a `PUT` on
//! a path to the one `GET` on that path. The SHA-256 of each byte that
//! lands is the sender's, and the server sees no plain byte of the file.

#[path = "transfer_harness/mod.rs"]
mod harness;

mod test_host;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use harness::Mailbox;
use pod_wormhole::Code;
use pod_ws::url::WsUrl;
use podnet::exit;
use podnet::transfer::land::Caps;
use podnet::transfer::{self, Listen, Receive, Received, Settings, What};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;

/// A mark in the file: a server that sees it sees plain bytes.
const MARK: &[u8] = b"PLAIN-MARK-OF-THE-FILE";

fn way() -> pod_ws::Way {
    podnet::net::init_crypto();
    let settings = pod_proxy::Settings::new("podnet-test", pod_proxy::ProxyChoice::Direct);
    let dial = pod_proxy::Dialer::new(settings, Arc::new(pod_proxy::resolve::System::new()));
    pod_ws::Way::new(
        podnet::trust::context().clone(),
        Arc::new(dial),
        "podnet/test",
    )
}

fn settings(mailbox: &str, piping: &[&str], listen: Listen) -> Settings {
    let mut settings = Settings::new(vec![WsUrl::parse(mailbox).unwrap()], Vec::new(), way());
    settings.listen = listen;
    settings.piping = piping.iter().map(|u| u.to_string()).collect();
    settings.wait = Duration::from_secs(30);
    settings.step = Duration::from_secs(5);
    settings.transit_wait = Duration::from_secs(30);
    settings
}

/// The path and the body of each `PUT`.
type Puts = Arc<Mutex<Vec<(String, Vec<u8>)>>>;

enum Slot {
    Get(oneshot::Sender<Vec<u8>>),
    Put(Vec<u8>, oneshot::Sender<()>),
}

/// The stand-in piping server: one `PUT` and one `GET` on a path meet; the
/// `GET` gets the body, then the `PUT` gets its answer. It keeps the path
/// and the body of each `PUT`.
struct Piping {
    url: String,
    puts: Puts,
    requests: Arc<Mutex<usize>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Piping {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Piping {
    async fn start() -> Piping {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let slots: Arc<Mutex<HashMap<String, Slot>>> = Arc::default();
        let puts: Puts = Arc::default();
        let requests = Arc::new(Mutex::new(0));
        let (p, r) = (puts.clone(), requests.clone());
        let task = tokio::spawn(async move {
            while let Ok((tcp, _)) = listener.accept().await {
                let (slots, puts, requests) = (slots.clone(), p.clone(), r.clone());
                tokio::spawn(async move {
                    let _ = serve(tcp, slots, puts, requests).await;
                });
            }
        });
        Piping {
            url,
            puts,
            requests,
            task,
        }
    }

    fn requests(&self) -> usize {
        *self.requests.lock().unwrap()
    }
}

async fn serve(
    mut tcp: TcpStream,
    slots: Arc<Mutex<HashMap<String, Slot>>>,
    puts: Puts,
    requests: Arc<Mutex<usize>>,
) -> std::io::Result<()> {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        tcp.read_exact(&mut byte).await?;
        head.push(byte[0]);
    }
    *requests.lock().unwrap() += 1;
    let head = String::from_utf8_lossy(&head).to_string();
    let mut words = head.split_whitespace();
    let (method, path) = (
        words.next().unwrap_or("").to_string(),
        words.next().unwrap_or("").to_string(),
    );
    let length: usize = head
        .lines()
        .find_map(|l| {
            let (name, value) = l.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().ok())?
        })
        .unwrap_or(0);
    let body = match method.as_str() {
        "PUT" | "POST" => {
            let mut body = vec![0u8; length];
            tcp.read_exact(&mut body).await?;
            puts.lock().unwrap().push((path.clone(), body.clone()));
            let (done, taken) = oneshot::channel();
            // None: a GET took it. Some(Ok): it waits. Some(Err): refused.
            let waiting: Option<Result<(), ()>> = {
                let mut slots = slots.lock().unwrap();
                match slots.remove(&path) {
                    Some(Slot::Get(get)) => match get.send(body) {
                        Ok(()) => None,
                        Err(body) => {
                            slots.insert(path.clone(), Slot::Put(body, done));
                            Some(Ok(()))
                        }
                    },
                    Some(other) => {
                        slots.insert(path.clone(), other);
                        Some(Err(()))
                    }
                    None => {
                        slots.insert(path.clone(), Slot::Put(body, done));
                        Some(Ok(()))
                    }
                }
            };
            match waiting {
                None => {}
                Some(Ok(())) => {
                    let _ = taken.await;
                }
                Some(Err(())) => return answer(&mut tcp, 400, b"[ERROR] a sender waits").await,
            }
            b"[INFO] sent".to_vec()
        }
        "GET" => {
            let (give, got) = oneshot::channel();
            let ready = {
                let mut slots = slots.lock().unwrap();
                match slots.remove(&path) {
                    Some(Slot::Put(body, done)) => {
                        let _ = done.send(());
                        Some(body)
                    }
                    // A receiver that left: the new one takes its place.
                    _ => {
                        slots.insert(path, Slot::Get(give));
                        None
                    }
                }
            };
            match ready {
                Some(body) => body,
                None => match got.await {
                    Ok(body) => body,
                    Err(_) => return Ok(()),
                },
            }
        }
        _ => return answer(&mut tcp, 405, b"").await,
    };
    answer(&mut tcp, 200, &body).await
}

async fn answer(tcp: &mut TcpStream, status: u16, body: &[u8]) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    tcp.write_all(head.as_bytes()).await?;
    tcp.write_all(body).await?;
    tcp.shutdown().await
}

/// A file of `size` bytes with [`MARK`] each 4 KiB.
fn marked_file(dir: &Path, size: usize) -> std::path::PathBuf {
    let mut data: Vec<u8> = (0..size as u32)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8)
        .collect();
    for at in (0..size.saturating_sub(MARK.len())).step_by(4096) {
        data[at..at + MARK.len()].copy_from_slice(MARK);
    }
    let path = dir.join("marked.bin");
    std::fs::write(&path, data).unwrap();
    path
}

fn sha256(path: &Path) -> String {
    pod_wormhole::hex(&Sha256::digest(std::fs::read(path).unwrap()))
}

async fn run(
    sender: Settings,
    receiver: Settings,
    what: What,
    out: &Path,
) -> (anyhow::Result<transfer::Sent>, anyhow::Result<Received>) {
    let opened = transfer::open(&sender).await.expect("a code");
    let code = Code::parse(opened.code()).expect("a code that parses");
    let sending = tokio::spawn(async move { opened.send(&sender, &what).await });
    let how = Receive {
        out: out.to_path_buf(),
        caps: Caps::default(),
    };
    let mut ask = |_: &transfer::Offered| Ok(true);
    let received = transfer::receive(&receiver, &code, &how, &mut ask).await;
    (sending.await.unwrap(), received)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn transit_over_piping_server_when_no_other_way_works() {
    test_host::need_loopback!();
    let mailbox = Mailbox::start().await;
    let piping = Piping::start().await;
    let (src, out) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let file = marked_file(src.path(), 700_001);
    let sender = settings(&mailbox.url, &[&piping.url], Listen::No);
    let receiver = settings(&mailbox.url, &[&piping.url], Listen::No);
    let (sent, received) = run(sender, receiver, What::Path(file.clone()), out.path()).await;
    let sent = sent.unwrap();
    let Received::Path { path, via, .. } = received.unwrap() else {
        panic!("a path")
    };
    assert_eq!(sent.via, format!("piping {}", piping.url));
    assert_eq!(via, sent.via);
    assert_eq!(sha256(&path), sha256(&file));
    let puts = piping.puts.lock().unwrap().clone();
    let from_sender: Vec<&(String, Vec<u8>)> =
        puts.iter().filter(|(p, _)| p.contains("/s/")).collect();
    assert!(
        from_sender.len() >= 4,
        "the file goes in pieces: {}",
        from_sender.len()
    );
    assert!(
        puts.iter().all(|(_, b)| b.len() <= transfer::piping::PIECE),
        "each piece is at most PIECE bytes"
    );
    let base = puts[0].0.split('/').nth(1).unwrap().to_string();
    assert_eq!(base.len(), 32, "the base path is 32 hex digits");
    assert!(puts
        .iter()
        .all(|(p, _)| p.starts_with(&format!("/{base}/"))));
    let seen: usize = puts.iter().map(|(_, b)| b.len()).sum();
    assert!(seen > 700_001, "the records of the file passed the server");
    for (path, body) in &puts {
        assert!(
            !body.windows(MARK.len()).any(|w| w == MARK),
            "the piping server saw a plain mark at {path}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn transit_over_piping_server_waits_behind_a_direct_hint() {
    test_host::need_loopback!();
    let mailbox = Mailbox::start().await;
    let piping = Piping::start().await;
    let (src, out) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let file = marked_file(src.path(), 10_000);
    let loopback = Listen::At(SocketAddr::from(([127, 0, 0, 1], 0)));
    let sender = settings(&mailbox.url, &[&piping.url], Listen::No);
    let receiver = settings(&mailbox.url, &[&piping.url], loopback);
    let (sent, received) = run(sender, receiver, What::Path(file.clone()), out.path()).await;
    assert!(sent.unwrap().via.starts_with("direct to 127.0.0.1:"));
    let Received::Path { path, .. } = received.unwrap() else {
        panic!("a path")
    };
    assert_eq!(sha256(&path), sha256(&file));
    assert_eq!(piping.requests(), 0, "the piping server is the last way");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn transit_over_piping_server_needs_a_server_that_both_ends_name() {
    test_host::need_loopback!();
    let mailbox = Mailbox::start().await;
    let piping = Piping::start().await;
    let (src, out) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let file = marked_file(src.path(), 1_000);
    let mut sender = settings(&mailbox.url, &[&piping.url], Listen::No);
    let mut receiver = settings(&mailbox.url, &[], Listen::No);
    sender.transit_wait = Duration::from_secs(2);
    receiver.transit_wait = Duration::from_secs(2);
    let (sent, received) = run(sender, receiver, What::Path(file), out.path()).await;
    assert_eq!(exit::code_of(&sent.unwrap_err()), exit::UNAVAILABLE);
    assert_eq!(exit::code_of(&received.unwrap_err()), exit::UNAVAILABLE);
    assert_eq!(piping.requests(), 0, "a receiver with no piping server");
    assert_eq!(std::fs::read_dir(out.path()).unwrap().count(), 0);
}
