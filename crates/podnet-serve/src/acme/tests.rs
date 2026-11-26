//! The tests of `--acme` with no public CA: the check of the name, the
//! state, a kept certificate served with no order, the renewal time, the
//! modes of the files, the challenge on the listener, and the exit code
//! and the words of a first order that fails.

use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;

use pod_acme::certs;
use pod_acme::key::Key;
use pod_proxy::resolve::System;
use pod_proxy::{Dialer, ProxyChoice, Settings};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use super::*;

const NAME: &str = "files.example.com";

fn code(err: &anyhow::Error) -> i32 {
    podnet_core::exit::code_of(err)
}

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

/// The HTTPS client of a test: direct, the roots of podnet, 5 s.
fn http() -> Http {
    let dial = Dialer::new(
        Settings::new("podnet-test", ProxyChoice::Direct),
        Arc::new(System::new()),
    );
    Http {
        client: pod_proxy::Client::new(podnet_core::trust::context().clone(), Arc::new(dial)),
        trust: pod_tls::Trust::Default,
        timeout: Duration::from_secs(5),
    }
}

/// A directory URL where nothing listens: a port that was free a moment ago.
async fn silent_ca() -> String {
    let port = TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    format!("https://127.0.0.1:{port}/dir")
}

/// Keep a self-signed certificate of `name`, made at `now`, as an order
/// keeps one. Gives its PEM.
fn keep_own(acme: &Acme, name: &str, now: i64) -> String {
    let key = Key::generate().unwrap();
    let chain = certs::own(name, &key, now).unwrap();
    acme.store.keep(&key, &chain).unwrap();
    chain
}

#[test]
fn a_name_that_tls_alpn_01_cannot_prove_is_refused_with_64() {
    let long_label = format!("{}.example.com", "a".repeat(64));
    let long_name = format!("{}.com", ["abcdefghi"; 26].join("."));
    let bad = [
        "",
        ".",
        "*.example.com",
        "1.2.3.4",
        "[::1]",
        "::1",
        "localhost",
        "-a.example.com",
        "a-.example.com",
        "a_b.example.com",
        "a..example.com",
        "files.example.com:443",
        "example.123",
        long_label.as_str(),
        long_name.as_str(),
    ];
    for name in bad {
        let err = check_domain(name).expect_err(name);
        assert_eq!(code(&err), 64, "{name:?}: {err:#}");
        assert!(format!("{err:#}").starts_with("--acme "), "{err:#}");
    }
    let good = [
        ("Files.Example.COM.", "files.example.com"),
        ("a-b.example.co.uk", "a-b.example.co.uk"),
        ("xn--bcher-kva.example", "xn--bcher-kva.example"),
    ];
    for (given, name) in good {
        assert_eq!(check_domain(given).unwrap(), name);
    }
}

#[test]
fn no_state_directory_and_a_bad_directory_url_are_refused() {
    let err = places(None).unwrap_err();
    assert_eq!(code(&err), 78, "{err:#}");
    assert!(format!("{err:#}").contains("PODNET_STATE_DIR"), "{err:#}");
    let state = tempfile::tempdir().unwrap();
    let places = places(Some(state.path().to_path_buf())).unwrap();
    for directory in ["http://ca.test/dir", "ca.test/dir"] {
        let err = open(NAME, Some(directory), http(), &places).unwrap_err();
        assert_eq!(code(&err), 64, "{directory}: {err:#}");
        assert!(
            format!("{err:#}").starts_with("--acme-directory: "),
            "{err:#}"
        );
    }
}

#[tokio::test]
async fn a_kept_certificate_is_served_with_no_order_and_renewed_30_days_before_its_end() {
    let state = tempfile::tempdir().unwrap();
    let places = places(Some(state.path().to_path_buf())).unwrap();
    // A CA that does not exist: an order would fail at once.
    let acme = open(NAME, Some("https://ca.invalid/dir"), http(), &places).unwrap();
    let now = pod_acme::order::now_secs();
    keep_own(&acme, NAME, now);
    let served = Served::new(None);
    let (leaf, keep) = start(acme.clone(), served.clone()).await.unwrap();
    assert_eq!(leaf.names, vec![NAME.to_string()]);
    assert!(served.current().is_some(), "the kept certificate is served");
    // The renewal waits: 30 days before the end of a life of a year.
    assert_eq!(renew_at(&leaf), leaf.not_after - 30 * 86_400);
    assert!(renew_at(&leaf) > now + 300 * 86_400);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!keep.is_finished(), "no order and no renewal now");
    keep.abort();
    // No account: podnet did not speak to the CA.
    assert!(!acme.store.dir.join("account.key").exists());
    // The state is private.
    assert!(acme.store.dir.starts_with(state.path().join("acme")));
    assert_eq!(mode(&state.path().join("acme")), 0o700);
    assert_eq!(mode(&acme.store.dir), 0o700);
    assert_eq!(mode(&acme.store.pem_path()), 0o600);
}

#[tokio::test]
async fn a_first_order_that_fails_exits_69_and_names_the_ca() {
    pod_testkit::need_loopback!();
    let state = tempfile::tempdir().unwrap();
    let places = places(Some(state.path().to_path_buf())).unwrap();
    let directory = silent_ca().await;
    let acme = open(NAME, Some(&directory), http(), &places).unwrap();
    let now = pod_acme::order::now_secs();
    // Each kept file that cannot be served now: an order runs, and fails.
    for what in ["none", "expired", "another name", "not a pair"] {
        match what {
            "expired" => drop(keep_own(&acme, NAME, now - 400 * 86_400)),
            "another name" => drop(keep_own(&acme, "other.example.com", now)),
            "not a pair" => {
                let path = acme.store.pem_path();
                std::fs::write(&path, "no PEM here").unwrap();
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
            }
            _ => {}
        }
        let served = Served::new(None);
        let err = start(acme.clone(), served.clone()).await.expect_err(what);
        let text = format!("{err:#}");
        assert_eq!(code(&err), 69, "{what}: {text}");
        assert!(text.starts_with("ACME at 127.0.0.1: "), "{what}: {text}");
        assert!(
            text.ends_with(&format!("; no certificate for {NAME} to serve")),
            "{what}: {text}"
        );
        assert!(served.current().is_none(), "{what}: nothing is served");
    }
    // The account key was made before the request, private.
    assert_eq!(mode(&acme.store.dir.join("account.key")), 0o600);
    assert_eq!(mode(&acme.store.dir), 0o700);
}

/// A TLS client that trusts `roots` and offers `alpn`.
fn client(roots: &[&str], alpn: &[u8]) -> tokio_rustls::TlsConnector {
    let mut store = rustls::RootCertStore::empty();
    for pem in roots {
        for der in pod_tls::server::certificates(pem.as_bytes()).unwrap() {
            store.add(der).unwrap();
        }
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(store)
        .with_no_client_auth();
    config.alpn_protocols = vec![alpn.to_vec()];
    tokio_rustls::TlsConnector::from(Arc::new(config))
}

#[tokio::test]
async fn the_listener_serves_https_and_answers_the_challenge_of_the_ca_alone() {
    pod_testkit::need_loopback!();
    let state = tempfile::tempdir().unwrap();
    let places = places(Some(state.path().to_path_buf())).unwrap();
    let acme = open(NAME, Some("https://ca.invalid/dir"), http(), &places).unwrap();
    let now = pod_acme::order::now_secs();
    let kept = keep_own(&acme, NAME, now);
    let served = Served::new(None);
    let (_, keep) = start(acme, served.clone()).await.unwrap();
    let (listener, bound) = bind("tcp://127.0.0.1:0").await.unwrap();
    let router = axum::Router::new().fallback(|| async { "files" });
    let server = tokio::spawn(serve_tls(
        listener,
        acceptor(served.clone()).unwrap(),
        router,
    ));
    let name = rustls_pki_types::ServerName::try_from(NAME).unwrap();
    // A visitor: HTTP over TLS, with the kept certificate.
    let tcp = tokio::net::TcpStream::connect(bound).await.unwrap();
    let mut tls = client(&[&kept], HTTP1)
        .connect(name.clone(), tcp)
        .await
        .unwrap();
    tls.write_all(b"GET / HTTP/1.1\r\nHost: files.example.com\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut answer = String::new();
    tls.read_to_string(&mut answer).await.unwrap_or_default();
    assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
    assert!(answer.ends_with("files"), "{answer}");
    // A validation with no challenge under way gets no certificate.
    let tcp = tokio::net::TcpStream::connect(bound).await.unwrap();
    assert!(client(&[&kept], ACME_TLS_1)
        .connect(name.clone(), tcp)
        .await
        .is_err());
    // A validation during a challenge gets the challenge's certificate, and
    // the connection carries no HTTP.
    let key = Key::generate().unwrap();
    let challenge = certs::own(NAME, &key, now).unwrap();
    let pair = pod_tls::server::certified(challenge.as_bytes(), key.pem().as_bytes()).unwrap();
    served.set_challenge(NAME, pair);
    let tcp = tokio::net::TcpStream::connect(bound).await.unwrap();
    let mut tls = client(&[&challenge], ACME_TLS_1)
        .connect(name, tcp)
        .await
        .unwrap();
    let got = tls.get_ref().1.peer_certificates().unwrap()[0].clone();
    let want = pod_tls::server::certificates(challenge.as_bytes())
        .unwrap()
        .remove(0);
    assert_eq!(got, want, "the certificate of the challenge");
    let _ = tls.write_all(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n").await;
    let mut rest = Vec::new();
    let _ = tls.read_to_end(&mut rest).await;
    assert!(rest.is_empty(), "no HTTP after a validation");
    server.abort();
    keep.abort();
}

#[tokio::test]
async fn a_unix_or_malformed_listen_address_is_refused_with_64() {
    for listen in [
        "unix:/tmp/x.sock",
        "files.example.com:443",
        "tcp://0.0.0.0",
        "443",
    ] {
        let err = bind(listen).await.expect_err(listen);
        assert_eq!(code(&err), 64, "{listen}: {err:#}");
    }
}
