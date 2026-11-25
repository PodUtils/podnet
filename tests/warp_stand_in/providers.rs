//! A second provider passes the same session test as the first
//! (PodUtils/podnet#486, QUIC-4 and QUIC-5): the Cloudflare adapter
//! registers a record at the stand-in API, the static adapter takes the
//! pinned edge and no record, and both open a CONNECT-IP session at the same
//! edge.
//!
//! The test lives in its own file so that `tests/warp_stand_in.rs` stays
//! below the 800-line limit of `scripts/check-size.py`. The stand-in API and
//! the edge are the ones of that test.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use super::warp_edge::Edge;
use super::{test_host, World};

/// The TLS provider of the test.
fn provider() -> std::sync::Arc<rustls::crypto::CryptoProvider> {
    std::sync::Arc::new(rustls::crypto::ring::default_provider())
}

/// The API of the stand-in at `base`, with a direct HTTP client.
fn api(base: &str) -> pod_warp::Api {
    let dialer = pod_proxy::Dialer::new(
        pod_proxy::Settings::new("podnet", pod_proxy::ProxyChoice::Direct),
        std::sync::Arc::new(pod_proxy::resolve::System::new()),
    );
    let context = std::sync::Arc::new(pod_tls::Context::new(
        provider(),
        pod_tls::Settings::new(pod_tls::Words::new("podnet")),
    ));
    pod_warp::Api::new(pod_proxy::Client::new(context, std::sync::Arc::new(dialer)))
        .with_bases(vec![base.to_string()])
}

/// A dial of the edge: each target is the address of the target.
struct Direct;

impl pod_core::Dial for Direct {
    fn label(&self) -> String {
        "the direct dial of the test".into()
    }

    fn dial<'a>(
        &'a self,
        target: &'a pod_core::Target,
        _deadline: tokio::time::Instant,
    ) -> pod_core::BoxFuture<'a, Result<pod_core::Connected, pod_core::Failure>> {
        Box::pin(async move {
            let addr = match target {
                pod_core::Target::Addr(addr) => *addr,
                pod_core::Target::Name { port, .. } => {
                    std::net::SocketAddr::from(([127, 0, 0, 1], *port))
                }
            };
            let tcp = tokio::net::TcpStream::connect(addr)
                .await
                .map_err(|e| pod_core::Failure::io(pod_core::Stage::Connect, "a test dial", e))?;
            Ok(pod_core::Connected {
                io: Box::new(tcp),
                peer: Some(addr),
                local: None,
            })
        })
    }
}

/// The way to the edge: HTTP/2 on the direct dial, with the time limit of
/// the test.
fn way() -> pod_warp::Way {
    let mut way = pod_warp::Way::new(std::sync::Arc::new(Direct), provider());
    way.timeout = Duration::from_secs(10);
    way
}

/// The settings of a provider at `base`, with the edge pinned as the front
/// door.
fn settings(base: &str, edge: SocketAddr) -> pod_warp::Settings {
    let mut settings = pod_warp::Settings::new(api(base), way());
    settings.door = pod_warp::Door::Pinned(pod_core::Target::Addr(edge));
    settings
}

/// The DER of the `PUBLIC KEY` PEM.
fn pin(pem: &str) -> Vec<u8> {
    use base64::Engine as _;
    let body: String = pem
        .lines()
        .filter(|line| !line.starts_with("-----"))
        .collect();
    base64::engine::general_purpose::STANDARD
        .decode(body)
        .expect("the edge's public key")
}

/// The same session assertion of both providers: a CONNECT-IP session
/// opens, one packet reaches the edge, and the session closes.
async fn the_session_test(warp: &pod_warp::Warp, edge: &Edge) {
    let before = edge
        .watch
        .families
        .iter()
        .map(|count| count.load(std::sync::atomic::Ordering::SeqCst))
        .sum::<usize>();
    let session = warp.open_session().await.expect("a CONNECT-IP session");
    assert!(!session.is_h3(), "the edge of the test is HTTP/2");
    let (mut tx, _rx) = session.split();
    tx.send_packets(&[vec![0x45u8; 200]])
        .await
        .expect("a packet into the session");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let after = edge
            .watch
            .families
            .iter()
            .map(|count| count.load(std::sync::atomic::Ordering::SeqCst))
            .sum::<usize>();
        if after > before {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the packet did not reach the edge"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    tx.close().expect("the session closes");
}

#[tokio::test(flavor = "multi_thread")]
async fn two_providers_open_the_same_session_at_the_edge() {
    test_host::need_loopback!();
    let world = World::new();
    let edge = Edge::start_open(world.api.clone(), vec![]).await;
    world.api.serve_key(&edge.pem());
    let cloudflare = pod_warp::Warp::register(settings(
        &format!("http://{}", world.served.addr()),
        edge.addr,
    ))
    .await
    .expect("the Cloudflare adapter registers");
    assert_eq!(cloudflare.provider(), Some("cloudflare"));
    the_session_test(&cloudflare, &edge).await;
    let local_settings = pod_warp::Settings::new(api("http://127.0.0.1:1"), way());
    let provider =
        pod_warp::StaticEnrolment::at("local-edge", edge.addr, pin(&edge.pem())).unwrap();
    let local = pod_warp::Warp::enrol(local_settings, &provider)
        .await
        .expect("the static adapter enrols");
    assert_eq!(local.provider(), Some("local-edge"));
    assert!(
        local.device().id.is_empty(),
        "a static provider keeps no record"
    );
    assert_eq!(
        local.account().await.unwrap_err().class,
        pod_core::Class::Usage
    );
    the_session_test(&local, &edge).await;
    assert_eq!(
        edge.seen.tunnels.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "each provider opened one CONNECT-IP tunnel"
    );
    assert_eq!(
        edge.seen.header("cf-connect-proto").as_deref(),
        Some("cf-connect-ip"),
        "the static session used the same protocol"
    );
    assert_eq!(
        edge.seen.refused.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "the edge refused no session"
    );
    eprintln!(
        "two providers: cloudflare and local-edge each opened a CONNECT-IP session at {}",
        edge.addr
    );
}
