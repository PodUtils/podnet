//! The qlog gate of PodUtils/podnet#486 (QUIC-7): a qlog capture shows the
//! HTTP/3 control stream of the shape.
//!
//! Two traces write into one directory. The HTTP/3 trace of
//! [`podnet::warp::h3::qlog`] names the control stream and its SETTINGS;
//! the QUIC trace of noq writes the stream frames. The test reads both
//! files and asserts that the control stream is in each one, at the stream
//! id that the HTTP/3 trace names.

use std::sync::Arc;
use std::time::Duration;

use pod_quic::noq::crypto::rustls::QuicServerConfig;
use pod_quic::noq::{ClientConfig, Connection};
use pod_warp::h3::wire::{self, Frames, FRAME_HEADERS};
use pod_warp::h3::{frame, Shape};
use podnet::warp::h3::qlog::Trace;
use podnet::warp::h3::serve::Listener;

/// The server name of the certificate.
const NAME: &str = "localhost";

/// The certificate of the shape and the roots that trust it.
fn cert() -> (Arc<QuicServerConfig>, rustls::RootCertStore) {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let ca_key = rcgen::KeyPair::generate().unwrap();
    let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "podnet h3 qlog CA");
    let ca = params.self_signed(&ca_key).unwrap();
    let issuer = rcgen::Issuer::new(params, ca_key);
    let leaf_key = rcgen::KeyPair::generate().unwrap();
    let leaf = rcgen::CertificateParams::new(vec![NAME.to_string(), "127.0.0.1".to_string()])
        .unwrap()
        .signed_by(&leaf_key, &issuer)
        .unwrap();
    let mut tls = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![leaf.der().clone(), ca.der().clone()],
            rustls::pki_types::PrivateKeyDer::Pkcs8(leaf_key.serialize_der().into()),
        )
        .unwrap();
    tls.alpn_protocols = vec![b"h3".to_vec()];
    let mut roots = rustls::RootCertStore::empty();
    roots.add(ca.der().clone()).unwrap();
    (
        Arc::new(QuicServerConfig::try_from(Arc::new(tls)).unwrap()),
        roots,
    )
}

/// The client: a QUIC connection with a qlog trace of its own, a control
/// stream, and one GET.
async fn get(
    addr: std::net::SocketAddr,
    roots: rustls::RootCertStore,
    qlog: &std::path::Path,
) -> (Connection, u16) {
    let settings = pod_quic::Settings::default();
    let tls = pod_quic::connect::tls(
        Arc::new(rustls::crypto::ring::default_provider()),
        Arc::new(roots),
        &[b"h3"],
    )
    .expect("the TLS of the client");
    let config: ClientConfig =
        pod_quic::connect::client_config(tls, &settings, false).expect("the client config");
    let mut transport = settings.transport(false);
    transport.qlog_from_path(qlog, "client");
    let mut config = config;
    config.transport_config(Arc::new(transport));
    let endpoint = pod_quic::endpoint::system(false).expect("the endpoint of the client");
    let connecting = endpoint
        .connect_with(config, addr, NAME)
        .expect("the connect of the client");
    let conn = tokio::time::timeout(settings.handshake_wait, connecting)
        .await
        .expect("a handshake")
        .expect("the client");
    let mut control = conn
        .open_uni()
        .await
        .expect("the control stream of the client");
    let mut first = vec![0x00];
    first.extend_from_slice(&frame(0x04, &Shape::common().settings_payload()));
    control
        .write_all(&first)
        .await
        .expect("the SETTINGS of the client");
    let (mut send, mut recv) = conn.open_bi().await.expect("a request stream");
    let section = request_get(NAME);
    send.write_all(&frame(FRAME_HEADERS, &section))
        .await
        .expect("the HEADERS of the request");
    send.finish().expect("the end of the request");
    let mut frames = Frames::default();
    let status = loop {
        if let Some((id, payload)) = frames.next_frame().expect("a frame of the answer") {
            if id == FRAME_HEADERS {
                break wire::response_status(&payload).expect("the status of the answer");
            }
            continue;
        }
        match recv.read_chunk(64 * 1024).await.expect("the answer") {
            Some(chunk) => frames.push(&chunk),
            None => panic!("the answer ended with no HEADERS"),
        }
    };
    let _control = control;
    (conn, status)
}

/// The field section of `GET /` with the authority `NAME`: the static
/// table, and one literal with a name reference for `:authority`.
fn request_get(authority: &str) -> Vec<u8> {
    let mut section = vec![
        0x00, // Required Insert Count 0
        0x00, // Base 0
        0xd1, // :method GET (static 17)
        0xd7, // :scheme https (static 23)
        0xc1, // :path / (static 1)
        0x50, // :authority (static name 0), a literal of no Huffman code
        authority.len() as u8,
    ];
    section.extend_from_slice(authority.as_bytes());
    section
}

/// The gate: the HTTP/3 trace names the control stream and its SETTINGS,
/// and the QUIC trace of noq has the stream frames of that stream.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_qlog_shows_the_http3_control_stream() {
    let dir = tempfile::tempdir().expect("a directory for the traces");
    let qlog = dir.path().join("qlog");
    std::fs::create_dir_all(&qlog).expect("the directory of the QUIC traces");
    let trace = Arc::new(Trace::create(dir.path().join("h3.qlog")).expect("the HTTP/3 trace"));
    let (crypto, roots) = cert();
    let listener = Arc::new(
        Listener::bind(
            "127.0.0.1:0".parse().unwrap(),
            crypto,
            Shape::common(),
            &pod_quic::Settings::default(),
            Some(trace),
            Some(&qlog),
        )
        .expect("the listener of the shape"),
    );
    let addr = listener.local_addr().expect("the address of the shape");
    let serve = tokio::spawn({
        let listener = listener.clone();
        async move { listener.accept().await }
    });
    let (conn, status) = get(addr, roots, &qlog).await;
    assert_eq!(status, 200, "the status of the answer");
    conn.close(0u32.into(), b"done");
    let _ = tokio::time::timeout(Duration::from_secs(5), conn.closed()).await;
    let _ = tokio::time::timeout(Duration::from_secs(5), serve).await;

    // The HTTP/3 trace: the control stream, its role and its SETTINGS.
    let h3 = std::fs::read_to_string(dir.path().join("h3.qlog")).expect("the HTTP/3 trace");
    let events: Vec<serde_json::Value> = h3
        .lines()
        .skip(1)
        .map(|line| serde_json::from_str(line).expect("a line of the HTTP/3 trace"))
        .collect();
    let control = events
        .iter()
        .find(|e| e["name"] == "http3:control_stream")
        .expect("the control stream in the HTTP/3 trace");
    assert_eq!(control["data"]["role"], "server");
    assert_eq!(control["data"]["stream_type"], "control");
    let id = control["data"]["stream_id"]
        .as_u64()
        .expect("the stream id");
    assert_eq!(id, 3, "the first unilateral stream of the server");
    let settings = control["data"]["settings"]
        .as_array()
        .expect("the SETTINGS of the control stream");
    assert!(
        settings.iter().any(|pair| pair[0] == 0x08),
        "the SETTINGS: {settings:?}"
    );

    // The QUIC trace of noq: a stream frame of that stream.
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&qlog)
        .expect("the directory of the QUIC traces")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    files.sort();
    assert!(
        !files.is_empty(),
        "no qlog file of the QUIC transport in {}",
        qlog.display()
    );
    let mut named = Vec::new();
    for file in &files {
        let text = std::fs::read_to_string(file).unwrap_or_default();
        let mut found = false;
        for line in text.lines() {
            let line = line.strip_prefix('\u{1e}').unwrap_or(line);
            let Ok(event) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            if event["name"] != "quic:packet_sent" && event["name"] != "quic:packet_received" {
                continue;
            }
            let Some(frames) = event["data"]["frames"].as_array() else {
                continue;
            };
            for frame in frames {
                if frame["frame_type"] == "stream" && frame["stream_id"].as_u64() == Some(id) {
                    found = true;
                }
            }
        }
        if found {
            named.push(file.clone());
        }
    }
    assert!(
        !named.is_empty(),
        "no QUIC qlog with a stream frame of stream {id}: {files:?}"
    );
    println!(
        "h3 qlog: control stream_id={id}, traced files={:?}",
        named
            .iter()
            .map(|f| f.file_name().unwrap().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
    );
}
