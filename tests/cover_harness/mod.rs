//! Shared fixtures and wire helpers for the cover tests.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pod_cover::Switch;
use podnet::cover::{self, Config, Cover};
use rustls::pki_types::ServerName;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsConnector;

pub const PAGE: &str =
    "<!doctype html><html><body><h1>Workshop notes</h1><p>The kettle is on.</p></body></html>\n";

/// The bytes that a TLS 1.3 record adds to its application data: the inner
/// content type and the AEAD tag. The script records application bytes.
pub const TLS13_OVERHEAD: usize = 17;

/// A hand-authored script, not a measurement. It exists for the loader
/// tests only and is never compared to a cover.
pub const AUTHORED_SCRIPT: &str = r#"
# Authored for the loader tests. This is not a measurement of a cover.
date = "2026-11-20"
cover_version = "1.7.2"
seed = 42

[[record_size]]
min = 512
max = 640
count = 50

[[timing]]
micros = 1200
jitter_micros = 80
count = 70

[split]
min = 512
max = 1600
"#;

/// The script fixture measured from the local cover, with its provenance.
pub fn measured_script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/cover/script.toml")
}

/// The capture fixture of the first records, with its provenance.
pub fn first_record_fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/cover/first-record.capture")
}

/// A private CA, its leaf for `localhost`, and the PEM forms.
pub struct Ca {
    ca_der: Vec<u8>,
    leaf_pem: String,
    key_pem: String,
}

pub fn ca() -> Ca {
    let ca_key = rcgen::KeyPair::generate().unwrap();
    let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "cover test CA");
    let ca_cert = params.self_signed(&ca_key).unwrap();
    let issuer = rcgen::Issuer::new(params, ca_key);
    let leaf_key = rcgen::KeyPair::generate().unwrap();
    let leaf = rcgen::CertificateParams::new(vec!["localhost".to_string()]).unwrap();
    let leaf_cert = leaf.signed_by(&leaf_key, &issuer).unwrap();
    Ca {
        ca_der: ca_cert.der().to_vec(),
        leaf_pem: leaf_cert.pem(),
        key_pem: leaf_key.serialize_pem(),
    }
}

pub fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

pub fn client_config(ca: &Ca) -> Arc<rustls::ClientConfig> {
    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(rustls::pki_types::CertificateDer::from(ca.ca_der.clone()))
        .unwrap();
    let mut config = rustls::ClientConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Arc::new(config)
}

/// The site tree, the script, and the switch key of one test.
pub struct Fixture {
    pub cover: Arc<Cover>,
    pub key: [u8; 32],
}

pub async fn echo() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut tcp, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 1024];
                while let Ok(n) = tcp.read(&mut buf).await {
                    if n == 0 || tcp.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                }
            });
        }
    });
    addr
}

pub fn write_site(dir: &Path) {
    std::fs::write(dir.join("index.html"), PAGE).unwrap();
    std::fs::write(dir.join("robots.txt"), "User-agent: *\nDisallow:\n").unwrap();
}

pub async fn fixture(mux: bool) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    write_site(dir.path());
    let script = measured_script();
    let key = [9u8; 32];
    let echo = echo().await;
    let config = Config {
        site: dir.path().display().to_string(),
        name: Some("localhost".to_string()),
        script: script.display().to_string(),
        switch_key: key,
        target: echo.to_string(),
        server: None,
        mux,
    };
    let cover = Arc::new(Cover::load(config).unwrap());
    // The temp directory must outlive the site load only: the site is in
    // memory.
    Fixture { cover, key }
}

pub async fn serve(fixture: &Fixture, ca: &Ca) -> SocketAddr {
    let acceptor = pod_tls::server::acceptor(
        provider(),
        ca.leaf_pem.as_bytes(),
        ca.key_pem.as_bytes(),
        &[pod_tls::server::HTTP1],
    )
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let cover = fixture.cover.clone();
    tokio::spawn(async move {
        let _ = cover::serve(listener, acceptor, cover).await;
    });
    addr
}

pub async fn connect(addr: SocketAddr, ca: &Ca) -> tokio_rustls::client::TlsStream<TcpStream> {
    let tcp = TcpStream::connect(addr).await.unwrap();
    TlsConnector::from(client_config(ca))
        .connect(ServerName::try_from("localhost").unwrap(), tcp)
        .await
        .unwrap()
}

/// The bytes that crossed one socket, with the instant of each read.
pub type Wire = Arc<Mutex<Vec<(u64, Vec<u8>)>>>;

/// A tee on the server socket: it forwards both directions unchanged and
/// records every byte, with the microseconds from the start of the tee.
pub struct Tee {
    pub addr: SocketAddr,
    pub c2s: Wire,
    pub s2c: Wire,
}

pub async fn tee(server: SocketAddr) -> Tee {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let c2s: Wire = Arc::new(Mutex::new(Vec::new()));
    let s2c: Wire = Arc::new(Mutex::new(Vec::new()));
    let start = Instant::now();
    let (left, right) = (c2s.clone(), s2c.clone());
    tokio::spawn(async move {
        while let Ok((client, _)) = listener.accept().await {
            let Ok(upstream) = TcpStream::connect(server).await else {
                continue;
            };
            let (mut cr, mut cw) = client.into_split();
            let (mut ur, mut uw) = upstream.into_split();
            let (c2s, s2c) = (left.clone(), right.clone());
            tokio::spawn(async move {
                let mut buf = [0u8; 8192];
                loop {
                    let n = match cr.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };
                    c2s.lock()
                        .unwrap()
                        .push((start.elapsed().as_micros() as u64, buf[..n].to_vec()));
                    if uw.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                }
                let _ = uw.shutdown().await;
            });
            tokio::spawn(async move {
                let mut buf = [0u8; 8192];
                loop {
                    let n = match ur.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };
                    s2c.lock()
                        .unwrap()
                        .push((start.elapsed().as_micros() as u64, buf[..n].to_vec()));
                    if cw.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                }
                let _ = cw.shutdown().await;
            });
        }
    });
    Tee { addr, c2s, s2c }
}

/// One TLS record as the wire held it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Record {
    pub at_micros: u64,
    pub kind: u8,
    pub version: [u8; 2],
    pub payload: usize,
}

/// The TLS records of the bytes that one direction carried.
pub fn tls_records(wire: &Wire) -> Vec<Record> {
    let chunks = wire.lock().unwrap().clone();
    let mut flat = Vec::new();
    for (at, bytes) in chunks {
        for byte in bytes {
            flat.push((at, byte));
        }
    }
    let mut out = Vec::new();
    let mut at = 0;
    while at + 5 <= flat.len() {
        let kind = flat[at].1;
        let version = [flat[at + 1].1, flat[at + 2].1];
        let payload = (usize::from(flat[at + 3].1) << 8) | usize::from(flat[at + 4].1);
        if !(0x14..=0x17).contains(&kind) || at + 5 + payload > flat.len() {
            break;
        }
        out.push(Record {
            at_micros: flat[at].0,
            kind,
            version,
            payload,
        });
        at += 5 + payload;
    }
    out
}

/// The first application-data record of one direction, or a panic.
pub fn first_application(wire: &Wire) -> Record {
    tls_records(wire)
        .into_iter()
        .find(|record| record.kind == 0x17)
        .expect("a first application-data record")
}

/// A GET of `size` bytes: the padding header grows the request, so its
/// first TLS record is the size of the cover's first record.
pub fn padded_get(size: usize) -> Vec<u8> {
    let head = b"GET / HTTP/1.1\r\nHost: localhost\r\nX-Cover-Pad: ";
    let tail = b"\r\n\r\n";
    assert!(size >= head.len() + tail.len(), "a GET of {size} bytes");
    let mut out = Vec::with_capacity(size);
    out.extend_from_slice(head);
    out.resize(size - tail.len(), b'a');
    out.extend_from_slice(tail);
    out
}

/// The exporter of this client connection, the channel binding of SWITCH-1.
pub fn exporter(tls: &tokio_rustls::client::TlsStream<TcpStream>) -> [u8; 32] {
    let mut out = [0u8; 32];
    tls.get_ref()
        .1
        .export_keying_material(&mut out, cover::EXPORTER_LABEL, cover::EXPORTER_CONTEXT)
        .unwrap();
    out
}

/// A switch bound to `tls`, which is what the listener checks.
pub fn bound(key: [u8; 32], tls: &tokio_rustls::client::TlsStream<TcpStream>) -> Switch {
    Switch::new(key).bind(&exporter(tls))
}

/// One HTTP/1.1 answer: its head text and its body.
pub async fn response(tls: &mut tokio_rustls::client::TlsStream<TcpStream>) -> (String, Vec<u8>) {
    let mut got = Vec::new();
    let mut buf = [0u8; 4096];
    while !got.windows(4).any(|w| w == b"\r\n\r\n") {
        let n = tls.read(&mut buf).await.unwrap();
        assert!(n > 0, "the connection ended before a head");
        got.extend_from_slice(&buf[..n]);
    }
    let end = got.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
    let head = String::from_utf8_lossy(&got[..end]).into_owned();
    let length: usize = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().ok())
                .flatten()
        })
        .unwrap_or(0);
    let mut body = got[end..].to_vec();
    while body.len() < length {
        let n = tls.read(&mut buf).await.unwrap();
        assert!(n > 0, "the body ended early");
        body.extend_from_slice(&buf[..n]);
    }
    (head, body)
}

/// A live measurement of the local cover: the TLS records that the server
/// wrote and the microseconds between them.
pub struct Measured {
    pub records: Vec<Record>,
    /// The gaps between the response records of each run, in order.
    pub gaps: Vec<Vec<u64>>,
}

/// The runs of the measurement: five connections, each three GETs, which
/// is what the script fixture records (`measured_runs`).
pub const MEASURED_RUNS: usize = 5;

pub async fn measure_cover(fixture: &Fixture, ca: &Ca) -> Measured {
    let addr = serve(fixture, ca).await;
    let mut records = Vec::new();
    let mut gaps = Vec::new();
    for _ in 0..MEASURED_RUNS {
        let tee = tee(addr).await;
        let mut tls = connect(tee.addr, ca).await;
        // Drop the handshake records: the measurement is of the responses.
        tokio::time::sleep(Duration::from_millis(20)).await;
        tee.s2c.lock().unwrap().clear();
        for path in ["/", "/robots.txt", "/missing"] {
            let request = format!("GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n");
            tls.write_all(request.as_bytes()).await.unwrap();
            let _ = response(&mut tls).await;
        }
        let run: Vec<Record> = tls_records(&tee.s2c)
            .into_iter()
            .filter(|record| record.kind == 0x17)
            .collect();
        gaps.push(
            run.windows(2)
                .map(|pair| pair[1].at_micros.saturating_sub(pair[0].at_micros))
                .collect(),
        );
        records.extend(run);
    }
    Measured { records, gaps }
}

/// The share of `sizes` in each bucket.
pub fn fractions(sizes: &[usize], buckets: &[(usize, usize)]) -> Vec<f64> {
    let total = sizes.len().max(1) as f64;
    buckets
        .iter()
        .map(|&(min, max)| {
            sizes
                .iter()
                .filter(|&&size| (min..=max).contains(&size))
                .count() as f64
                / total
        })
        .collect()
}

/// The total variation distance of two distributions.
pub fn total_variation(left: &[f64], right: &[f64]) -> f64 {
    0.5 * left
        .iter()
        .zip(right)
        .map(|(a, b)| (a - b).abs())
        .sum::<f64>()
}
