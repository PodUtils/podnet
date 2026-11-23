//! Local submission (SMTP) and retrieval (IMAP) servers for the drive test
//! of GitHub #484. Each server speaks the part of the protocol that the
//! carrier of `pod-link` uses: the greeting, TLS in its two modes, the
//! credential, one message up, and the fetch down. The certificate is made
//! in this process and authenticates nothing that matters.

#![allow(dead_code)]

use std::io;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine as _;
use pod_link::smtp::Credential;
use pod_link::BoxIo;
use pod_tls::server::TlsAcceptor;
use rustls::crypto::CryptoProvider;
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::{TcpListener, TcpStream};

/// The user of the session credential of the stand.
pub const USER: &str = "session-user";
/// The secret of the session credential of the stand: a made-up value,
/// never the password of an account.
pub const SECRET: &str = "made-up-session-secret";

/// The certificate of the stand: a CA and a leaf for `localhost` and
/// `127.0.0.1` that it signs.
pub struct Certificates {
    pub leaf: String,
    pub key: String,
    pub ca: String,
}

/// The certificates of this process, made once.
pub fn certificates() -> &'static Certificates {
    use std::sync::OnceLock;

    use rcgen::{BasicConstraints, CertificateParams, ExtendedKeyUsagePurpose, IsCa, KeyPair};
    static MADE: OnceLock<Certificates> = OnceLock::new();
    MADE.get_or_init(|| {
        let ca_key = KeyPair::generate().expect("a key of the CA of the stand");
        let mut ca_params = CertificateParams::new(vec!["podnet mail stand CA".to_string()])
            .expect("the CA parameters");
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let ca = ca_params.self_signed(&ca_key).expect("the CA of the stand");
        let issuer = rcgen::Issuer::new(ca_params, ca_key);
        let leaf_key = KeyPair::generate().expect("a key of the leaf of the stand");
        let mut leaf_params =
            CertificateParams::new(vec!["localhost".to_string(), "127.0.0.1".to_string()])
                .expect("the leaf parameters");
        leaf_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        let leaf = leaf_params
            .signed_by(&leaf_key, &issuer)
            .expect("the leaf of the stand");
        Certificates {
            leaf: leaf.pem(),
            key: leaf_key.serialize_pem(),
            ca: ca.pem(),
        }
    })
}

pub fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

fn acceptor() -> TlsAcceptor {
    let made = certificates();
    pod_tls::server::acceptor(provider(), made.leaf.as_bytes(), made.key.as_bytes(), &[])
        .expect("the certificate of the stand")
}

/// The client configuration that trusts the certificate of the stand.
pub fn client_config() -> Arc<rustls::ClientConfig> {
    let mut roots = rustls::RootCertStore::empty();
    for certificate in
        pod_tls::server::certificates(certificates().ca.as_bytes()).expect("the CA of the stand")
    {
        roots.add(certificate).expect("the root of the stand");
    }
    Arc::new(
        rustls::ClientConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()
            .expect("TLS 1.3 and 1.2")
            .with_root_certificates(roots)
            .with_no_client_auth(),
    )
}

pub fn credential() -> Credential {
    Credential::new(USER, SECRET).expect("a credential of the stand")
}

/// A submission server: the messages that it took, as they came, and the
/// largest message that it takes (`None`: no limit).
pub struct Smtp {
    pub addr: SocketAddr,
    pub messages: Arc<Mutex<Vec<Vec<u8>>>>,
    /// The time at which the server took each message, in order.
    pub accepted: Arc<Mutex<Vec<std::time::Instant>>>,
    pub limit: Option<u64>,
    /// The time that the server waits before it accepts a message: the
    /// failing control of the rate test.
    pub delay: Option<Duration>,
}

/// Start a submission server in `mode`, with no limit of the message size.
pub async fn smtp(mode: pod_link::smtp::Mode) -> Smtp {
    smtp_with_limit(mode, None).await
}

/// Start a submission server in `mode` that waits `delay` before it takes
/// each message. The wait is the deliberately slow submission of the rate
/// test's failing control.
pub async fn smtp_with_delay(mode: pod_link::smtp::Mode, delay: Duration) -> Smtp {
    smtp_with_limit_and_delay(mode, None, Some(delay)).await
}

/// Start a submission server in `mode` that refuses a message above `limit`
/// bytes with `552`, the size of the limit in the text. The server does not
/// advertise `SIZE`: the carrier learns the limit by the refusal, as it does
/// from a server whose limit is below the size that it advertised.
pub async fn smtp_with_limit(mode: pod_link::smtp::Mode, limit: Option<u64>) -> Smtp {
    smtp_with_limit_and_delay(mode, limit, None).await
}

async fn smtp_with_limit_and_delay(
    mode: pod_link::smtp::Mode,
    limit: Option<u64>,
    delay: Option<Duration>,
) -> Smtp {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a submission socket");
    let addr = listener.local_addr().expect("the address of the server");
    let messages = Arc::new(Mutex::new(Vec::new()));
    let accepted = Arc::new(Mutex::new(Vec::new()));
    let (taken, stamped) = (messages.clone(), accepted.clone());
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let messages = taken.clone();
            let accepted = stamped.clone();
            tokio::spawn(async move {
                let _ = serve_smtp(stream, mode, messages, accepted, limit, delay).await;
            });
        }
    });
    Smtp {
        addr,
        messages,
        accepted,
        limit,
        delay,
    }
}

async fn serve_smtp(
    stream: TcpStream,
    mode: pod_link::smtp::Mode,
    messages: Arc<Mutex<Vec<Vec<u8>>>>,
    accepted: Arc<Mutex<Vec<std::time::Instant>>>,
    limit: Option<u64>,
    delay: Option<Duration>,
) -> io::Result<()> {
    let mut lines = match mode {
        pod_link::smtp::Mode::Implicit => {
            let tls = acceptor()
                .accept(Box::new(stream) as BoxIo)
                .await
                .map_err(io::Error::other)?;
            BufReader::new(Box::new(tls) as BoxIo)
        }
        pod_link::smtp::Mode::Starttls => BufReader::new(Box::new(stream) as BoxIo),
    };
    write(&mut lines, b"220 stand.test ESMTP\r\n").await?;
    loop {
        let Some(line) = next(&mut lines).await else {
            return Ok(());
        };
        let text = String::from_utf8_lossy(&line).trim_end().to_string();
        let upper = text.to_uppercase();
        if upper.starts_with("EHLO") || upper.starts_with("HELO") {
            let mut reply = String::from("250-stand.test\r\n");
            if mode == pod_link::smtp::Mode::Starttls {
                reply.push_str("250-STARTTLS\r\n");
            }
            reply.push_str("250 AUTH PLAIN LOGIN\r\n");
            write(&mut lines, reply.as_bytes()).await?;
        } else if upper.starts_with("STARTTLS") {
            write(&mut lines, b"220 begin TLS\r\n").await?;
            let io = lines.into_inner();
            let tls = acceptor().accept(io).await.map_err(io::Error::other)?;
            lines = BufReader::new(Box::new(tls) as BoxIo);
        } else if upper.starts_with("AUTH PLAIN ") {
            let token = text.split_whitespace().nth(2).unwrap_or_default();
            let plain = base64::engine::general_purpose::STANDARD
                .decode(token)
                .unwrap_or_default();
            if plain == format!("\0{USER}\0{SECRET}").as_bytes() {
                write(&mut lines, b"235 authenticated\r\n").await?;
            } else {
                write(&mut lines, b"535 no\r\n").await?;
            }
        } else if upper.starts_with("MAIL FROM:") {
            let size = text
                .split_whitespace()
                .find_map(|word| word.strip_prefix("SIZE=")?.parse::<u64>().ok());
            match (limit, size) {
                (Some(max), Some(size)) if size > max => {
                    write(
                        &mut lines,
                        format!(
                            "552 5.3.4 message size exceeds fixed maximum message size {max}\r\n"
                        )
                        .as_bytes(),
                    )
                    .await?;
                }
                _ => write(&mut lines, b"250 sender ok\r\n").await?,
            }
        } else if upper.starts_with("RCPT TO:") {
            write(&mut lines, b"250 recipient ok\r\n").await?;
        } else if upper == "DATA" {
            write(&mut lines, b"354 end with a dot\r\n").await?;
            let mut message = Vec::new();
            loop {
                let Some(line) = next(&mut lines).await else {
                    return Ok(());
                };
                if line == b".\r\n" || line == b".\n" {
                    break;
                }
                message.extend_from_slice(&line);
            }
            if let Some(wait) = delay {
                tokio::time::sleep(wait).await;
            }
            if let Some(max) = limit {
                if message.len() as u64 > max {
                    write(
                        &mut lines,
                        format!(
                            "552 5.3.4 message size exceeds fixed maximum message size {max}\r\n"
                        )
                        .as_bytes(),
                    )
                    .await?;
                    continue;
                }
            }
            messages
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(message);
            accepted
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(std::time::Instant::now());
            write(&mut lines, b"250 message ok\r\n").await?;
        } else if upper == "QUIT" {
            write(&mut lines, b"221 bye\r\n").await?;
            return Ok(());
        } else {
            write(&mut lines, b"250 ok\r\n").await?;
        }
    }
}

/// A retrieval server: the messages that the client appended, and the store
/// that it fetches, with the uid of each message as its place plus 1.
pub struct Imap {
    pub addr: SocketAddr,
    pub appended: Arc<Mutex<Vec<Vec<u8>>>>,
    pub store: Arc<Mutex<Vec<Vec<u8>>>>,
}

/// Start a retrieval server in `mode`.
pub async fn imap(mode: pod_link::imap::Mode) -> Imap {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a retrieval socket");
    let addr = listener.local_addr().expect("the address of the server");
    let appended = Arc::new(Mutex::new(Vec::new()));
    let store = Arc::new(Mutex::new(Vec::new()));
    let (sent, held) = (appended.clone(), store.clone());
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let (appended, store) = (sent.clone(), held.clone());
            tokio::spawn(async move {
                let _ = serve_imap(stream, mode, appended, store).await;
            });
        }
    });
    Imap {
        addr,
        appended,
        store,
    }
}

async fn serve_imap(
    stream: TcpStream,
    mode: pod_link::imap::Mode,
    appended: Arc<Mutex<Vec<Vec<u8>>>>,
    store: Arc<Mutex<Vec<Vec<u8>>>>,
) -> io::Result<()> {
    let mut lines = match mode {
        pod_link::imap::Mode::Implicit => {
            let tls = acceptor()
                .accept(Box::new(stream) as BoxIo)
                .await
                .map_err(io::Error::other)?;
            BufReader::new(Box::new(tls) as BoxIo)
        }
        pod_link::imap::Mode::Starttls => BufReader::new(Box::new(stream) as BoxIo),
    };
    write(
        &mut lines,
        b"* OK [CAPABILITY IMAP4rev1 STARTTLS AUTH=PLAIN] stand ready\r\n",
    )
    .await?;
    loop {
        let Some(line) = next(&mut lines).await else {
            return Ok(());
        };
        let text = String::from_utf8_lossy(&line).trim_end().to_string();
        let Some((tag, rest)) = text.split_once(' ') else {
            continue;
        };
        let upper = rest.to_uppercase();
        if upper.starts_with("CAPABILITY") {
            let caps = format!("* CAPABILITY IMAP4rev1 STARTTLS AUTH=PLAIN\r\n{tag} OK done\r\n");
            write(&mut lines, caps.as_bytes()).await?;
        } else if upper.starts_with("STARTTLS") {
            write(&mut lines, format!("{tag} OK begin\r\n").as_bytes()).await?;
            let io = lines.into_inner();
            let tls = acceptor().accept(io).await.map_err(io::Error::other)?;
            lines = BufReader::new(Box::new(tls) as BoxIo);
        } else if upper.starts_with("LOGIN") {
            write(&mut lines, format!("{tag} OK logged in\r\n").as_bytes()).await?;
        } else if upper.starts_with("SELECT") {
            let reply = format!(
                "* 1 EXISTS\r\n* OK [UIDVALIDITY 1] ok\r\n{tag} OK [READ-WRITE] selected\r\n"
            );
            write(&mut lines, reply.as_bytes()).await?;
        } else if upper.starts_with("APPEND") {
            let size = rest
                .split_once('{')
                .and_then(|(_, rest)| rest.split_once('}'))
                .and_then(|(value, _)| value.parse::<usize>().ok());
            let Some(size) = size else {
                write(&mut lines, format!("{tag} BAD no literal\r\n").as_bytes()).await?;
                continue;
            };
            write(&mut lines, b"+ go ahead\r\n").await?;
            let mut body = vec![0u8; size];
            lines.read_exact(&mut body).await?;
            let mut end = Vec::new();
            lines.read_until(b'\n', &mut end).await?;
            appended
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(body);
            write(&mut lines, format!("{tag} OK appended\r\n").as_bytes()).await?;
        } else if upper.starts_with("UID FETCH") {
            let start: u32 = rest
                .split_whitespace()
                .nth(2)
                .and_then(|range| range.split(':').next())
                .and_then(|value| value.parse().ok())
                .unwrap_or(1);
            let messages = store.lock().unwrap_or_else(|e| e.into_inner()).clone();
            for (place, message) in messages.iter().enumerate() {
                let uid = place as u32 + 1;
                if uid < start {
                    continue;
                }
                let head = format!(
                    "* {} FETCH (UID {uid} BODY[] {{{}}}\r\n",
                    place + 1,
                    message.len()
                );
                write(&mut lines, head.as_bytes()).await?;
                write(&mut lines, message).await?;
                write(&mut lines, b")\r\n").await?;
            }
            write(&mut lines, format!("{tag} OK done\r\n").as_bytes()).await?;
        } else if upper.starts_with("LOGOUT") {
            write(&mut lines, format!("* BYE\r\n{tag} OK bye\r\n").as_bytes()).await?;
            return Ok(());
        } else {
            write(&mut lines, format!("{tag} BAD unknown\r\n").as_bytes()).await?;
        }
    }
}

async fn next(lines: &mut BufReader<BoxIo>) -> Option<Vec<u8>> {
    let mut line = Vec::new();
    match lines.read_until(b'\n', &mut line).await {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(line),
    }
}

async fn write(lines: &mut BufReader<BoxIo>, bytes: &[u8]) -> io::Result<()> {
    lines.write_all(bytes).await?;
    lines.flush().await
}

/// Wait until the retrieval server appended `count` messages.
pub async fn wait_appended(server: &Imap, count: usize) -> Vec<Vec<u8>> {
    for _ in 0..200 {
        let appended = server
            .appended
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if appended.len() >= count {
            return appended;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    server
        .appended
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

/// Wait until the submission server took `count` messages.
pub async fn wait_messages(server: &Smtp, count: usize) -> Vec<Vec<u8>> {
    for _ in 0..200 {
        let messages = server
            .messages
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if messages.len() >= count {
            return messages;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    server
        .messages
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

/// The body of a submitted message, decoded from base64.
pub fn smtp_body(message: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(message);
    let body = text
        .split_once("\r\n\r\n")
        .map(|(_, body)| body)
        .unwrap_or("");
    let joined: String = body.chars().filter(|c| *c != '\r' && *c != '\n').collect();
    base64::engine::general_purpose::STANDARD
        .decode(joined)
        .expect("a base64 body")
}
