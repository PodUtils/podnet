//! `podnet tunnel --inspect` (T-244) with no edge: each request goes through
//! `serve_decoded` of pod-cloudflare, as a stream of the edge does, to a
//! local origin, and the sink of the inspector is `Host::capture`.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pod_cloudflare::cloudflare::stream::{
    self as cqstream, ConnectRequest, ConnectionType, HTTP_HEADER_KEY, HTTP_HOST_KEY,
    HTTP_METHOD_KEY, HTTP_STATUS_KEY,
};
use pod_cloudflare::proxy::{serve_decoded, Runtime};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio_util::compat::{TokioAsyncReadCompatExt as _, TokioAsyncWriteCompatExt as _};

use podnet::ingress::{Ingress, Service};
use podnet::tunnel::inspect::{header, Inspector, Options, State, REDACTED};

/// The requests that the origin got, as text.
type Seen = Arc<Mutex<Vec<String>>>;

/// A local origin. `/hook` answers 500 to its first request and 200 to
/// each other one; `/stream` sends 5 chunks 150 ms apart; `/bad` sends the
/// chunk size line `ffffffffffffffff` and closes.
async fn origin() -> (String, Seen) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let at = format!("http://{}", listener.local_addr().unwrap());
    let seen: Seen = Arc::default();
    let log = seen.clone();
    tokio::spawn(async move {
        let hooks = Arc::new(std::sync::atomic::AtomicU32::new(0));
        loop {
            let Ok((mut s, _)) = listener.accept().await else {
                return;
            };
            let (log, hooks) = (log.clone(), hooks.clone());
            tokio::spawn(async move {
                let text = read_request(&mut s).await;
                log.lock().unwrap().push(text.clone());
                let path = text.split(' ').nth(1).unwrap_or_default().to_string();
                if path == "/stream" {
                    let _ = s
                        .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n")
                        .await;
                    for n in 0..5 {
                        let _ = s.write_all(format!("2\r\nc{n}\r\n").as_bytes()).await;
                        tokio::time::sleep(Duration::from_millis(150)).await;
                    }
                    let _ = s.write_all(b"0\r\n\r\n").await;
                    return;
                }
                let answer: &[u8] = match path.as_str() {
                    "/bad" => {
                        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nffffffffffffffff\r\n"
                    }
                    _ if hooks.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 => {
                        b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 4\r\n\r\nfail"
                    }
                    _ => b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nSet-Cookie: s=secret\r\n\r\nok",
                };
                let _ = s.write_all(answer).await;
                let _ = s.shutdown().await;
            });
        }
    });
    (at, seen)
}

/// The head and the body of a request (Content-Length framing).
async fn read_request(s: &mut tokio::net::TcpStream) -> String {
    let mut got = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        if let Some(end) = got.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&got[..end]).to_ascii_lowercase();
            let length: usize = head
                .lines()
                .find_map(|l| l.strip_prefix("content-length:"))
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(0);
            if got.len() >= end + 4 + length {
                break;
            }
        }
        match s.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => got.extend_from_slice(&buf[..n]),
        }
    }
    String::from_utf8_lossy(&got).into_owned()
}

/// A request of the edge for `path` with `headers`.
fn request(path: &str, headers: &[(&str, &str)]) -> ConnectRequest {
    let mut metadata = vec![
        (HTTP_METHOD_KEY.to_string(), "POST".to_string()),
        (HTTP_HOST_KEY.to_string(), "hook.example".to_string()),
    ];
    for (name, value) in headers {
        metadata.push((format!("{HTTP_HEADER_KEY}:{name}"), value.to_string()));
    }
    ConnectRequest {
        dest: format!("https://hook.example{path}"),
        conn_type: ConnectionType::Http,
        metadata,
    }
}

struct Tunnel {
    runtime: Arc<Runtime>,
    ingress: Arc<Ingress>,
    inspector: Arc<Inspector>,
    _host: Arc<podnet::tunnel::Host>,
}

/// A runtime of pod-cloudflare with the sink of an inspector, as
/// `tunnel --inspect` builds it.
fn tunnel(origin: &str, options: Options) -> Tunnel {
    let ingress = Ingress::new().catch_all(Service::Http(origin.to_string()));
    let inspector = Inspector::new(options, ingress.clone());
    let mut host = podnet::tunnel::base_host();
    host.capture = Some(inspector.sink(None));
    let host = Arc::new(host);
    inspector.attach(&host);
    let metrics = podnet::metrics::Metrics::new();
    let runtime = Arc::new(Runtime::build(host.clone(), &ingress, metrics).unwrap());
    Tunnel {
        runtime,
        ingress: Arc::new(ingress),
        inspector,
        _host: host,
    }
}

/// Serve `request` with the body `sent`. Gives the status, the body that
/// the visitor got, and the time of the first body byte and of the end.
async fn visit(
    tunnel: &Tunnel,
    request: ConnectRequest,
    sent: &[u8],
) -> (String, Vec<u8>, Option<Duration>, Duration) {
    let start = Instant::now();
    let (ours, theirs) = tokio::io::duplex(1 << 16);
    let (mut visitor, input) = tokio::io::duplex(1 << 16);
    visitor.write_all(sent).await.unwrap();
    drop(visitor);
    let serve = serve_decoded(
        tunnel.runtime.clone(),
        tunnel.ingress.clone(),
        request,
        Box::pin(input.compat()),
        Box::pin(ours.compat_write()),
    );
    let read = async {
        let mut reader = theirs.compat();
        let (_, meta) = cqstream::read_connect_response(&mut reader).await.unwrap();
        let status = meta
            .iter()
            .find(|(k, _)| k == HTTP_STATUS_KEY)
            .map(|(_, v)| v.clone())
            .unwrap_or_default();
        let mut body = Vec::new();
        let mut first = None;
        let mut buf = [0u8; 1024];
        loop {
            match futures::io::AsyncReadExt::read(&mut reader, &mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    first.get_or_insert_with(|| start.elapsed());
                    body.extend_from_slice(&buf[..n]);
                }
            }
        }
        (status, body, first, start.elapsed())
    };
    let ((), got) =
        tokio::time::timeout(Duration::from_secs(20), async { tokio::join!(serve, read) })
            .await
            .expect("the exchange ends");
    got
}

/// After a failing delivery, show gives the request with each secret
/// redacted, while the origin got the real value; replay sends it again
/// with no redacted header and records a new exchange that names the first.
#[tokio::test]
async fn replay_records_a_new_exchange_that_names_the_first() {
    pod_testkit::need_loopback!();
    let (at, seen) = origin().await;
    let tunnel = tunnel(&at, Options::default());
    let headers = [
        ("Content-Length", "5"),
        ("Content-Type", "application/json"),
        ("Authorization", "Bearer s3cret"),
        ("X-Hub-Signature-256", "sha256=abc"),
    ];
    let (status, body, _, _) = visit(
        &tunnel,
        request("/hook?id=1&token=t0p", &headers),
        b"{\"a\"}",
    )
    .await;
    assert_eq!((status.as_str(), body.as_slice()), ("500", &b"fail"[..]));
    assert!(
        seen.lock().unwrap()[0].contains("Bearer s3cret"),
        "the forward keeps the value"
    );

    let first = tunnel.inspector.get(1).expect("exchange 1");
    assert_eq!(first.state, State::Complete);
    assert_eq!(first.status, Some(500));
    assert_eq!(first.target, format!("/hook?id=1&token={REDACTED}"));
    assert_eq!(first.request.bytes, b"{\"a\"}");
    let doc = first.detail();
    let text = doc.to_string();
    for secret in ["s3cret", "sha256=abc", "t0p"] {
        assert!(!text.contains(secret), "{secret} in {text}");
    }

    let replayed = tunnel.inspector.replay(1).await.unwrap();
    assert_eq!(replayed["replayed_from"], 1);
    let left: Vec<&str> = replayed["left_out"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert_eq!(
        left,
        ["token (query)", "Authorization", "X-Hub-Signature-256"]
    );
    let new_id = replayed["exchange"]["id"].as_u64().unwrap();
    assert_eq!(new_id, 2);
    let second = tunnel.inspector.get(new_id).unwrap();
    assert_eq!((second.replayed_from, second.status), (Some(1), Some(200)));
    assert_eq!(second.state, State::Complete);
    assert_eq!(second.response.bytes, b"ok");
    assert_eq!(second.response_headers[1].1, REDACTED, "Set-Cookie");
    let again = seen.lock().unwrap()[1].clone();
    assert!(again.starts_with("POST /hook?id=1 HTTP/1.1\r\n"), "{again}");
    assert!(again.ends_with("{\"a\"}"), "{again}");
    assert!(
        !again.to_ascii_lowercase().contains("authorization"),
        "{again}"
    );
    // The replay is no visitor request of the tunnel.
    assert_eq!(tunnel.runtime.metrics().snapshot().streams_total, 1);
}

/// A request body over the cap is kept up to the cap, and its replay is
/// refused (exit 65).
#[tokio::test]
async fn replay_of_a_truncated_request_is_refused() {
    pod_testkit::need_loopback!();
    let (at, _) = origin().await;
    let options = Options {
        body: 4,
        ..Options::default()
    };
    let tunnel = tunnel(&at, options);
    visit(
        &tunnel,
        request("/hook", &[("Content-Length", "10")]),
        b"0123456789",
    )
    .await;
    let one = tunnel.inspector.get(1).unwrap();
    assert_eq!(
        (one.request.bytes.as_slice(), one.request.cut()),
        (&b"0123"[..], 6)
    );
    let err = tunnel.inspector.replay(1).await.unwrap_err();
    assert_eq!(podnet::exit::code_of(&err), 65, "{err:#}");
    assert!(tunnel.inspector.get(2).is_none(), "no new exchange");
}

/// A streaming answer reaches the visitor chunk by chunk with the sink:
/// the first chunk comes long before the last one.
#[tokio::test]
async fn a_streaming_response_keeps_its_timing_with_inspect() {
    pod_testkit::need_loopback!();
    let (at, _) = origin().await;
    let tunnel = tunnel(&at, Options::default());
    let (status, body, first, all) = visit(&tunnel, request("/stream", &[]), b"").await;
    assert_eq!(
        (status.as_str(), body.as_slice()),
        ("200", &b"c0c1c2c3c4"[..])
    );
    let first = first.expect("a body byte");
    assert!(all >= Duration::from_millis(600), "{all:?}");
    assert!(
        first + Duration::from_millis(400) < all,
        "first {first:?}, all {all:?}"
    );
    let one = tunnel.inspector.get(1).unwrap();
    assert_eq!(
        one.response.bytes, b"c0c1c2c3c4",
        "the payload, no size lines"
    );
    assert_eq!(one.state, State::Complete);
}

/// A chunk size line that does not fit ends the exchange with an error, and
/// the ring keeps the partial exchange as incomplete.
#[tokio::test]
async fn a_bad_chunk_size_ends_the_exchange_as_incomplete() {
    pod_testkit::need_loopback!();
    let (at, _) = origin().await;
    let tunnel = tunnel(&at, Options::default());
    visit(&tunnel, request("/bad", &[]), b"").await;
    let one = tunnel.inspector.get(1).unwrap();
    assert_eq!(one.state, State::Incomplete);
    assert_eq!(one.status, Some(200));
    let err = tunnel.inspector.replay(1).await.unwrap_err();
    assert_eq!(podnet::exit::code_of(&err), 65);
}

/// The ring keeps the last N exchanges of the tunnel in their order.
#[tokio::test]
async fn the_ring_keeps_the_cap_and_the_flags() {
    pod_testkit::need_loopback!();
    let (at, _) = origin().await;
    let options = Options {
        keep: 2,
        ..Options::default()
    };
    let tunnel = tunnel(&at, options);
    for _ in 0..3 {
        visit(&tunnel, request("/hook", &[("Cookie", "c=1")]), b"").await;
    }
    let list = tunnel.inspector.list();
    let ids: Vec<u64> = list.iter().map(|e| e["id"].as_u64().unwrap()).collect();
    assert_eq!(ids, [2, 3]);
    assert!(list.iter().all(|e| e["state"] == "complete"));
    let two = tunnel.inspector.get(2).unwrap();
    assert_eq!(two.redacted, ["Cookie", "Set-Cookie"]);
}

/// The origin gets the real value of each secret header; the ring keeps
/// `[redacted]` for each one, and for the Set-Cookie of the answer.
#[tokio::test]
async fn authorization_is_redacted_and_forwarded() {
    pod_testkit::need_loopback!();
    let (at, seen) = origin().await;
    let tunnel = tunnel(&at, Options::default());
    let headers = [
        ("Authorization", "Basic dXNlcjpwYXNz"),
        ("Cookie", "sid=42"),
        ("X-Api-Key", "k-123"),
        ("Accept", "text/plain"),
    ];
    // The first request of /hook gets 500; the second one has Set-Cookie.
    for _ in 0..2 {
        visit(&tunnel, request("/hook", &headers), b"").await;
    }
    let got = seen.lock().unwrap()[1].clone();
    for value in ["Basic dXNlcjpwYXNz", "sid=42", "k-123"] {
        assert!(got.contains(value), "the origin lacks {value}: {got}");
    }
    let two = tunnel.inspector.get(2).unwrap();
    let stored = |name: &str| header(&two.request_headers, name).map(str::to_string);
    for name in ["Authorization", "Cookie", "X-Api-Key"] {
        assert_eq!(stored(name).as_deref(), Some(REDACTED), "{name}");
    }
    assert_eq!(stored("Accept").as_deref(), Some("text/plain"));
    assert_eq!(header(&two.response_headers, "set-cookie"), Some(REDACTED));
    assert!(!two.detail().to_string().contains("dXNlcjpwYXNz"));
}
