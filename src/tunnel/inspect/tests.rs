//! The ring, the redaction, the refusals of a replay and the socket of the
//! inspector (T-244). `tests/tunnel_inspect.rs` sends exchanges through
//! pod-cloudflare to a local origin.

use std::time::{Duration, SystemTime};

use pod_cloudflare::capture::{Capture, Direction, End, Head, Line, Refused};
use serde_json::json;

use super::replay::{metadata, pick_rule, refusal, target_of};
use super::*;
use crate::ingress::{IngressRule, OriginOptions, PathPattern, Service};

fn request(target: &str, headers: &[(&str, &str)]) -> Head {
    Head {
        line: Line::Request {
            method: "POST".into(),
            target: target.into(),
        },
        headers: headers
            .iter()
            .map(|(n, v)| (n.to_string(), v.to_string()))
            .collect(),
    }
}

fn answer(status: u16, headers: &[(&str, &str)]) -> Head {
    Head {
        line: Line::Response { status },
        headers: headers
            .iter()
            .map(|(n, v)| (n.to_string(), v.to_string()))
            .collect(),
    }
}

fn end(complete: bool) -> End {
    End {
        status: Some(200),
        started: SystemTime::now(),
        elapsed: Duration::from_millis(7),
        complete,
    }
}

fn inspector(keep: usize, body: usize) -> Arc<Inspector> {
    let options = Options {
        keep,
        body,
        raw: false,
    };
    Inspector::new(options, Ingress::new())
}

/// One rule for each name: the value of a header is a secret by its full
/// name, or by a word in its name; other headers stay.
#[test]
fn a_header_is_redacted_by_its_name() {
    let table: &[(&str, bool)] = &[
        ("Authorization", true),
        ("proxy-authorization", true),
        ("Cookie", true),
        ("Set-Cookie", true),
        ("X-Api-Key", true),
        ("X-Auth-Token", true),
        ("X-Hub-Signature-256", true),
        ("X-Client-Secret", true),
        ("X-Session-Id", true),
        ("X-Password", true),
        ("Sec-WebSocket-Key", true),
        ("Content-Type", false),
        ("Content-Length", false),
        ("Host", false),
        ("User-Agent", false),
        ("Accept", false),
        ("X-Request-Id", false),
        ("Keep-Alive", false),
        ("Cf-Connecting-Ip", false),
    ];
    for (name, secret) in table {
        assert_eq!(secret_name(name), *secret, "{name}");
        let mut names = Vec::new();
        let headers = vec![(name.to_string(), "v=1".to_string())];
        let stored = redact_headers(&headers, false, &mut names);
        let want = if *secret { REDACTED } else { "v=1" };
        assert_eq!(stored[0].1, want, "{name}");
        assert_eq!(names.len(), usize::from(*secret), "{name}");
        // --inspect-raw keeps each value.
        let mut none = Vec::new();
        assert_eq!(redact_headers(&headers, true, &mut none)[0].1, "v=1");
        assert!(none.is_empty());
    }
}

/// A query parameter that names a secret is redacted in the stored target,
/// and a replay leaves it out.
#[test]
fn a_secret_query_parameter_is_redacted_and_left_out() {
    let mut names = Vec::new();
    let stored = redact_target("/hook?id=7&access_token=abc&x=1", false, &mut names);
    assert_eq!(stored, format!("/hook?id=7&access_token={REDACTED}&x=1"));
    assert_eq!(names, ["access_token"]);
    let mut left = Vec::new();
    assert_eq!(target_of(&stored, &mut left), "/hook?id=7&x=1");
    assert_eq!(left, ["access_token (query)"]);
    assert_eq!(
        redact_target("/a?token=1", true, &mut Vec::new()),
        "/a?token=1"
    );
    assert_eq!(redact_target("/plain", false, &mut Vec::new()), "/plain");
}

/// The ring keeps the last `keep` exchanges in their order; a part of an
/// exchange that the ring dropped is refused, and the rest of the ring
/// stays.
#[test]
fn the_ring_keeps_the_cap_and_the_order() {
    let inspector = inspector(3, 64);
    let sink = inspector.sink(None);
    for exchange in 1..=5u64 {
        let target = format!("/{exchange}");
        sink.head(exchange, &request(&target, &[])).unwrap();
    }
    let ids: Vec<u64> = inspector
        .list()
        .iter()
        .map(|e| e["id"].as_u64().unwrap())
        .collect();
    assert_eq!(ids, [3, 4, 5]);
    // Exchange 1 of the crate is gone: its parts are refused.
    assert_eq!(sink.body(1, Direction::Request, b"x"), Err(Refused));
    assert_eq!(sink.head(1, &answer(200, &[])), Err(Refused));
    // Exchange 5 is open; it ends complete.
    sink.body(5, Direction::Response, b"ok").unwrap();
    sink.head(5, &answer(201, &[("Set-Cookie", "s=1")]))
        .unwrap();
    sink.end(5, &end(true));
    let five = inspector.get(5).unwrap();
    assert_eq!(five.state, State::Complete);
    assert_eq!(five.status, Some(200), "the end gives the status");
    assert_eq!(five.response.bytes, b"ok");
    assert_eq!(five.response_headers[0].1, REDACTED);
    assert_eq!(five.elapsed_ms, Some(7));
    assert!(inspector.get(1).is_none());
    // The drop of the sink marks each open exchange incomplete.
    drop(sink);
    assert_eq!(inspector.get(4).unwrap().state, State::Incomplete);
    assert_eq!(inspector.get(5).unwrap().state, State::Complete);
}

/// Each body keeps its cap and counts the bytes that the cap cut.
#[test]
fn a_body_keeps_its_cap_and_counts_the_cut() {
    let inspector = inspector(4, 4);
    let sink = inspector.sink(None);
    sink.head(1, &request("/up", &[])).unwrap();
    for part in [&b"abc"[..], b"defg", b"h"] {
        sink.body(1, Direction::Request, part).unwrap();
    }
    sink.body(1, Direction::Response, b"12").unwrap();
    let one = inspector.get(1).unwrap();
    assert_eq!(one.request.bytes, b"abcd");
    assert_eq!((one.request.total, one.request.cut()), (8, 4));
    assert_eq!((one.response.total, one.response.cut()), (2, 0));
    let doc = one.detail();
    assert_eq!(doc["request"]["cut"], 4);
    assert_eq!(doc["request"]["body"], "abcd");
    assert_eq!(doc["cut"], 4);
    // A body that is not UTF-8 goes in base64.
    let mut body = Body::default();
    body.push(&[0xff, 0x00], 8);
    assert_eq!(body.json(&[])["body_base64"], "/wA=");
}

/// A replay is refused for an open, an incomplete, an upgrade and a cut
/// exchange; a complete one goes.
#[test]
fn a_replay_refuses_what_it_cannot_send_whole() {
    let inspector = inspector(8, 4);
    let sink = inspector.sink(None);
    // 1: open. 2: incomplete. 3: an upgrade. 4: a cut body. 5: complete.
    for exchange in 1..=5u64 {
        let headers: &[(&str, &str)] = match exchange {
            3 => &[("Upgrade", "websocket")],
            _ => &[],
        };
        sink.head(exchange, &request("/", headers)).unwrap();
    }
    sink.end(2, &end(false));
    sink.end(3, &end(true));
    sink.body(4, Direction::Request, b"too long").unwrap();
    sink.end(4, &end(true));
    sink.end(5, &end(true));
    let words = |id| refusal(&inspector.get(id).unwrap());
    assert!(words(1).unwrap().contains("still open"));
    assert!(words(2).unwrap().contains("incomplete"));
    assert!(words(3).unwrap().contains("WebSocket"));
    assert!(words(4).unwrap().contains("cut (4 of 8 bytes kept)"));
    assert_eq!(words(5), None);
}

/// The request of a replay leaves out each redacted header, the framing
/// and the hop, and gives the body its Content-Length.
#[test]
fn the_replay_request_leaves_out_the_redacted_headers() {
    let inspector = inspector(2, 64);
    let sink = inspector.sink(None);
    let head = request(
        "/hook",
        &[
            ("Host", "hook.example"),
            ("Authorization", "Bearer x"),
            ("Content-Type", "text/plain"),
            ("Transfer-Encoding", "chunked"),
            ("Connection", "close"),
        ],
    );
    sink.head(1, &head).unwrap();
    sink.body(1, Direction::Request, b"ping").unwrap();
    sink.end(1, &end(true));
    let mut left = Vec::new();
    let meta = metadata(&inspector.get(1).unwrap(), "hook.example", &mut left);
    assert_eq!(left, ["Authorization"]);
    let names: Vec<&str> = meta.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(
        names,
        [
            "HttpMethod",
            "HttpHost",
            "HttpHeader:Content-Type",
            "HttpHeader:Content-Length"
        ]
    );
    assert_eq!(meta[3].1, "4");
}

/// The rule of a replay: the httpHostHeader of a rule, else its hostname,
/// and its path.
#[test]
fn the_replay_picks_the_rule_of_the_host_and_the_path() {
    let rule =
        |host: Option<&str>, path: Option<&str>, header: Option<&str>, url: &str| IngressRule {
            hostname: host.map(str::to_string),
            path: path.map(PathPattern::prefix),
            service: Service::Http(url.into()),
            origin: OriginOptions {
                http_host_header: header.map(str::to_string),
                ..OriginOptions::default()
            },
        };
    let ingress = Ingress::new()
        .rule_with(rule(
            Some("a.example"),
            Some("/api"),
            None,
            "http://127.0.0.1:1",
        ))
        .rule_with(rule(
            Some("b.example"),
            None,
            Some("internal"),
            "http://127.0.0.1:2",
        ))
        .rule_with(rule(None, None, None, "http://127.0.0.1:3"));
    let table = [
        ("a.example", "/api/x", "http://127.0.0.1:1"),
        ("a.example", "/other", "http://127.0.0.1:3"),
        ("internal", "/", "http://127.0.0.1:2"),
        ("c.example", "/", "http://127.0.0.1:3"),
    ];
    for (host, target, want) in table {
        let got = pick_rule(&ingress, host, target).map(|r| r.service.describe());
        assert_eq!(got.as_deref(), Some(want), "{host} {target}");
    }
}

/// The socket has the mode 0600, it answers list and show, and it refuses
/// a bad request with its exit code. The rule of the peer: the user of the
/// tunnel, or root.
#[tokio::test]
async fn the_socket_is_private_and_answers() {
    use std::os::unix::fs::PermissionsExt as _;
    for (peer, own, allowed) in [(1000, 1000, true), (0, 1000, true), (1001, 1000, false)] {
        assert_eq!(peer_allowed(peer, own), allowed, "{peer} {own}");
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(format!("t{SUFFIX}"));
    let inspector = inspector(4, 64);
    let sink = inspector.sink(None);
    sink.head(1, &request("/a", &[("Cookie", "c=1")])).unwrap();
    sink.end(1, &end(true));
    let served = serve(inspector.clone(), &path).unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    assert_eq!(served_path().as_deref(), Some(path.as_path()));
    let list = client::ask(&path, &json!({"verb": "list"})).await.unwrap();
    assert_eq!(list["exchanges"][0]["id"], 1);
    assert_eq!(list["keep"], 4);
    let show = client::ask(&path, &json!({"verb": "show", "id": 1}))
        .await
        .unwrap();
    assert_eq!(show["exchange"]["request"]["headers"][0][1], REDACTED);
    let table = [
        (json!({"verb": "show", "id": 9}), 66),
        (json!({"verb": "show"}), 64),
        (json!({"verb": "drop"}), 64),
        (json!({"verb": "replay", "id": 9}), 66),
    ];
    for (request, code) in table {
        let err = client::ask(&path, &request).await.unwrap_err();
        assert_eq!(crate::exit::code_of(&err), code, "{request}");
    }
    // A replay before the tunnel started: no host.
    let err = client::ask(&path, &json!({"verb": "replay", "id": 1}))
        .await
        .unwrap_err();
    assert_eq!(
        crate::exit::code_of(&err),
        65,
        "no rule of the empty ingress"
    );
    drop(served);
    assert!(!path.exists(), "the drop removes the socket");
    let err = client::ask(&path, &json!({"verb": "list"}))
        .await
        .unwrap_err();
    assert_eq!(crate::exit::code_of(&err), 69);
}
