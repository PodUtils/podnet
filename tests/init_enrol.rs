//! GitHub #500, INIT-2: enrolment against a local registrar. The success
//! test registers over TLS with an `https` relay; the rollback test gets a
//! 500 and asserts that no destination and no temporary directory is left.
//! Neither test reaches a public service.

mod test_host;

use std::io;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use pod_tls::server::TlsAcceptor;
use podnet::initcfg::{
    enrol, is_enrolled, NodeConfig, Registrar, ALLOW_FILE, CONFIG_FILE, RELAY_FILE,
};
use rustls::crypto::CryptoProvider;
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::{TcpListener, TcpStream};

const CA_NAME: &str = "podnet enrol test CA";

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// A CA and a leaf for `127.0.0.1`, made once.
fn certificates() -> (&'static str, &'static str, &'static str) {
    static MADE: OnceLock<(String, String, String)> = OnceLock::new();
    let (leaf, key, ca) = MADE.get_or_init(|| {
        use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair};
        let ca_key = KeyPair::generate().expect("a key of the CA");
        let mut ca_params =
            CertificateParams::new(vec![CA_NAME.to_string()]).expect("the CA parameters");
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let ca = ca_params.self_signed(&ca_key).expect("the CA");
        let issuer = rcgen::Issuer::new(ca_params, ca_key);
        let leaf_key = KeyPair::generate().expect("a key of the leaf");
        let leaf_params =
            CertificateParams::new(vec!["127.0.0.1".to_string()]).expect("the leaf parameters");
        let leaf = leaf_params.signed_by(&leaf_key, &issuer).expect("the leaf");
        (leaf.pem(), leaf_key.serialize_pem(), ca.pem())
    });
    (leaf.as_str(), key.as_str(), ca.as_str())
}

fn acceptor() -> TlsAcceptor {
    let (leaf, key, _) = certificates();
    pod_tls::server::acceptor(provider(), leaf.as_bytes(), key.as_bytes(), &[])
        .expect("the certificate of the registrar")
}

/// A registrar on the loopback. It answers `status` and records the request
/// of each connection.
async fn registrar(tls: bool, status: u16) -> (SocketAddr, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a registrar socket");
    let addr = listener.local_addr().expect("the address of the registrar");
    let seen: Arc<Mutex<Vec<String>>> = Arc::default();
    let requests = seen.clone();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let requests = requests.clone();
            tokio::spawn(async move {
                let _ = answer(stream, tls, status, requests).await;
            });
        }
    });
    (addr, seen)
}

async fn answer(
    stream: TcpStream,
    tls: bool,
    status: u16,
    requests: Arc<Mutex<Vec<String>>>,
) -> io::Result<()> {
    let io: pod_link::BoxIo = if tls {
        Box::new(
            acceptor()
                .accept(Box::new(stream) as pod_link::BoxIo)
                .await
                .map_err(io::Error::other)?,
        )
    } else {
        Box::new(stream)
    };
    let mut lines = BufReader::new(io);
    let mut head = String::new();
    let mut length = 0usize;
    loop {
        let mut line = String::new();
        if lines.read_line(&mut line).await? == 0 {
            return Ok(());
        }
        if line == "\r\n" || line == "\n" {
            head.push_str(&line);
            break;
        }
        if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            length = value.trim().parse().unwrap_or(0);
        }
        head.push_str(&line);
    }
    let mut body = vec![0u8; length];
    lines.read_exact(&mut body).await?;
    let request = format!("{head}{}", String::from_utf8_lossy(&body));
    requests
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(request);
    let body = if (200..300).contains(&status) {
        serde_json::json!({
            "registered": true,
            "node": "node-a",
            "relay": "https://relay.stand.test",
        })
        .to_string()
    } else {
        "the relay refused the registration".to_string()
    };
    let reply = format!(
        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let mut io = lines.into_inner();
    io.write_all(reply.as_bytes()).await?;
    io.flush().await?;
    // A TLS stream needs the close_notify, or the client reads an unclean EOF.
    io.shutdown().await
}

/// The body of the request: the node and the key that was issued.
fn body_of(request: &str) -> serde_json::Value {
    let split = request.find("\r\n\r\n").expect("the request has a body");
    serde_json::from_str(&request[split + 4..]).expect("a JSON body")
}

/// INIT-2: an `https` relay is enrolled over TLS, and the identity, the
/// allow list, the relay registration and the configuration are written.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_https_enrolment_registers_with_a_local_registrar() {
    test_host::need_loopback!();
    let root = tempfile::tempdir().expect("a temp dir");
    let (_, _, ca) = certificates();
    let ca_file = root.path().join("ca.pem");
    std::fs::write(&ca_file, ca).expect("the CA file");
    // The roots of this test process: the CA of the registrar alone.
    std::env::set_var("SSL_CERT_FILE", &ca_file);

    let (addr, seen) = registrar(true, 201).await;
    let relay = format!("https://127.0.0.1:{}", addr.port());
    let config = NodeConfig::generated("node-a", &relay);
    let dest = root.path().join("state");
    let enrolled = enrol(
        &dest,
        &config,
        &Registrar::Http {
            timeout: Duration::from_secs(5),
        },
    )
    .await
    .expect("the enrolment over TLS");

    assert!(is_enrolled(&dest), "the destination is not whole");
    assert_eq!(enrolled.registration["registered"], true);
    assert_eq!(enrolled.registration["node"], "node-a");
    assert_eq!(enrolled.node, "node-a");
    let relay_json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dest.join(RELAY_FILE)).unwrap()).unwrap();
    assert_eq!(relay_json, enrolled.registration);
    assert_eq!(
        std::fs::read_to_string(dest.join(ALLOW_FILE)).unwrap(),
        "",
        "an empty allow list is an empty file"
    );
    let read = NodeConfig::load(&dest.join(CONFIG_FILE)).expect("the written configuration");
    assert_eq!(read, config);

    let requests = seen.lock().unwrap().clone();
    assert_eq!(requests.len(), 1, "one registration");
    let request = &requests[0];
    assert!(
        request.starts_with("POST /v1/enrol HTTP/1.1\r\n"),
        "the request line: {request}"
    );
    let body = body_of(request);
    assert_eq!(body["node"], "node-a");
    assert_eq!(body["public_key"], enrolled.public_key);
    assert_eq!(enrolled.public_key.len(), 64, "64 hex digits of a key");
    eprintln!(
        "enrolled over TLS at {relay}: {} key {}",
        request.lines().next().unwrap_or(""),
        enrolled.public_key
    );
}

/// INIT-3: a registrar that refuses leaves no destination and no sibling
/// temporary directory.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refused_enrolment_leaves_no_partial_state() {
    test_host::need_loopback!();
    let root = tempfile::tempdir().expect("a temp dir");
    let (addr, seen) = registrar(false, 500).await;
    let relay = format!("http://127.0.0.1:{}", addr.port());
    let config = NodeConfig::generated("node-a", &relay);
    let dest = root.path().join("state");
    let error = enrol(
        &dest,
        &config,
        &Registrar::Http {
            timeout: Duration::from_secs(5),
        },
    )
    .await
    .expect_err("the refusal");
    let code = podnet::exit::code_of(&error);
    assert_eq!(code, 69, "{error:#}");
    assert!(!dest.exists(), "the destination exists after a refusal");
    let left: Vec<_> = std::fs::read_dir(root.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert!(left.is_empty(), "partial state left: {left:?}");
    let requests = seen.lock().unwrap().clone();
    assert_eq!(requests.len(), 1, "the registrar saw the request");
    eprintln!("refused enrolment: exit {code}, {error:#}, left {left:?}");
}
