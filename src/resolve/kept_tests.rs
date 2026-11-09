//! One kept connection for each DoH server (T-175): N queries, one
//! connection at a loopback DoH server.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use pod_core::{BoxFuture, Connected, Dial, Failure, Stage, Target};
use pod_dns::{Doh, Names, Resolver, Settings};
use pod_tls::{Context, TlsRoots, Trust, Words};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::Instant;

const NAME: &str = "doh.test";

/// The loopback server: its port, the trust of its CA, the connections it
/// accepted and the requests it read.
async fn server() -> (u16, Trust, Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let ca_key = rcgen::KeyPair::generate().unwrap();
    let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let ca = params.self_signed(&ca_key).unwrap();
    let issuer = rcgen::Issuer::new(params, ca_key);
    let leaf_key = rcgen::KeyPair::generate().unwrap();
    let leaf = rcgen::CertificateParams::new(vec![NAME.to_string()])
        .unwrap()
        .signed_by(&leaf_key, &issuer)
        .unwrap();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ServerConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![leaf.der().clone()],
            rustls::pki_types::PrivateKeyDer::Pkcs8(leaf_key.serialize_der().into()),
        )
        .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (accepted, requests) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    let (seen_connections, seen_requests) = (accepted.clone(), requests.clone());
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            seen_connections.fetch_add(1, Ordering::SeqCst);
            let (acceptor, requests) = (acceptor.clone(), seen_requests.clone());
            tokio::spawn(async move {
                let Ok(mut tls) = acceptor.accept(tcp).await else {
                    return;
                };
                let mut head = Vec::new();
                let mut chunk = [0u8; 1024];
                loop {
                    while !head.ends_with(b"\r\n\r\n") {
                        match tls.read(&mut chunk).await {
                            Ok(n) if n > 0 => head.extend_from_slice(&chunk[..n]),
                            _ => return,
                        }
                    }
                    requests.fetch_add(1, Ordering::SeqCst);
                    let text = String::from_utf8_lossy(&head).into_owned();
                    head.clear();
                    let body = if text.contains("type=AAAA") {
                        r#"{"Status":0,"Answer":[{"name":"relay.test.","type":28,"TTL":60,"data":"2001:db8::1"}]}"#
                    } else {
                        r#"{"Status":0,"Answer":[{"name":"relay.test.","type":1,"TTL":60,"data":"192.0.2.1"}]}"#
                    };
                    let answer = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                        body.len()
                    );
                    if tls.write_all(answer.as_bytes()).await.is_err() {
                        return;
                    }
                }
            });
        }
    });
    let context = Context::new(provider, pod_tls::Settings::new(Words::new("podnet")));
    let pem = ca.der().clone();
    let mut store = rustls::RootCertStore::empty();
    store.add(pem).unwrap();
    let roots = TlsRoots {
        count: 1,
        roots: store,
        source: "the test's own CA".into(),
    };
    let trust = Trust::caller(context.client_config(&roots).unwrap()).unwrap();
    (port, trust, accepted, requests)
}

/// A way out to the loopback server that counts its dials.
struct Counting {
    port: u16,
    dials: AtomicUsize,
}

impl Dial for Counting {
    fn label(&self) -> String {
        "the loopback server".into()
    }

    fn dial<'a>(
        &'a self,
        _target: &'a Target,
        _deadline: Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(async move {
            self.dials.fetch_add(1, Ordering::SeqCst);
            let tcp = TcpStream::connect(("127.0.0.1", self.port))
                .await
                .map_err(|e| Failure::io(Stage::Connect, e))?;
            Ok(Connected::new(Box::new(tcp)))
        })
    }

    fn names_inside(&self) -> bool {
        true
    }
}

/// Four lookups of A and AAAA make eight requests over one connection and
/// one dial.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn doh_queries_share_one_connection() {
    crate::test_host::need_loopback!();
    let (port, trust, accepted, requests) = server().await;
    let settings = Settings {
        resolvers: vec![Doh::new([192, 0, 2, 53].into(), NAME).json("/resolve")],
        names: Names::First,
        both: true,
        trust,
        ..Settings::default()
    };
    let dial = Arc::new(Counting {
        port,
        dials: AtomicUsize::new(0),
    });
    let context = Arc::new(Context::new(
        Arc::new(rustls::crypto::ring::default_provider()),
        pod_tls::Settings::new(Words::new("podnet")),
    ));
    let resolver = Resolver::new(settings).with_doh(context, dial.clone());
    let way = resolver.way().expect("the resolver asks DoH");
    for _ in 0..4 {
        let found = way
            .lookup("relay.test", Instant::now() + Duration::from_secs(10))
            .await
            .unwrap();
        assert_eq!(found.len(), 2);
    }
    assert_eq!(requests.load(Ordering::SeqCst), 8);
    assert_eq!(
        (
            accepted.load(Ordering::SeqCst),
            dial.dials.load(Ordering::SeqCst)
        ),
        (1, 1)
    );
}
