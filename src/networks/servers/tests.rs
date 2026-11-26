//! `vless://`, `ss://` and `wisp:` (T-127): the specs, the secrets, and a
//! dial through each road against the stand-ins of `servers_stand.rs` on the
//! loopback, with the CA of the stands as the only root.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::super::road_stand;
use super::super::servers_stand as stand;
use super::super::via_tests::{round_trip, stand_tls, vars};
use super::super::*;
use super::{SHADOWSOCKS_KEY, SHADOWSOCKS_KEY_FILE, VLESS_ID, VLESS_ID_FILE};
use crate::dialers::Direct;

const WAIT: Duration = Duration::from_secs(20);

/// Bring the road of `spec` up with the variables of `vars`, for the router
/// network `name`, over a direct way and with the trust of the stands.
async fn up_road(
    spec: &str,
    vars: &HashMap<String, String>,
    name: Option<&str>,
) -> Result<Link, Failure> {
    let get = |key: &str| vars.get(key).cloned();
    let below: Arc<dyn Dial> = Arc::new(Direct);
    let (tls, trust) = stand_tls();
    match parse(spec)?.kind {
        Kind::Server(road) => {
            let settings = road.settings(&get, name)?;
            road.up_with(settings, Some(&below), tls, trust).await
        }
        other => panic!("{spec}: not a road: {other:?}"),
    }
}

/// The spec of the VLESS stand at `at`.
fn vless_spec(at: std::net::SocketAddr) -> String {
    format!("vless://{at}?type=ws&security=tls&path=%2Fvless")
}

/// A stream through `link` to `target`: the bytes of `text` go, and the
/// read of the answer gives what came, or the error of the read.
async fn exchange(link: &Link, target: Target, text: &[u8]) -> Result<Vec<u8>, String> {
    let way = link.way.clone().expect("the link has a way");
    let deadline = tokio::time::Instant::now() + WAIT;
    let mut io = way
        .dial
        .dial(&target, deadline)
        .await
        .map_err(|e| e.to_string())?
        .io;
    io.write_all(text).await.map_err(|e| e.to_string())?;
    let mut back = vec![0u8; text.len()];
    match tokio::time::timeout(WAIT, io.read_exact(&mut back)).await {
        Ok(Ok(_)) => Ok(back),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err("no answer in time".into()),
    }
}

#[test]
fn network_spec_the_roads_of_a_server_have_a_scheme_and_a_label() {
    for (spec, scheme, label, tls) in [
        (
            "vless://v.example:443?type=ws&security=tls&path=%2Fws",
            "vless",
            "vless://v.example:443/ws",
            true,
        ),
        (
            "VLESS://[2001:db8::1]:8080?type=xhttp&mode=stream-one#home",
            "vless",
            "vless://[2001:db8::1]:8080/",
            false,
        ),
        (
            "ss://ss.example:8388",
            "shadowsocks",
            "ss://ss.example:8388",
            false,
        ),
        (
            "SS://192.0.2.7:443/",
            "shadowsocks",
            "ss://192.0.2.7:443",
            false,
        ),
        (
            "wisp:wss://w.example/wisp/",
            "wisp",
            "wisp:wss://w.example/wisp/",
            true,
        ),
        (
            "wisp:ws://127.0.0.1:6001/",
            "wisp",
            "wisp:ws://127.0.0.1:6001/",
            false,
        ),
        (
            "wisp:w.example:8443/w/",
            "wisp",
            "wisp:wss://w.example:8443/w/",
            true,
        ),
        (
            "wisp+wss://w.example/?k=v",
            "wisp",
            "wisp:wss://w.example/",
            true,
        ),
    ] {
        let network = parse(spec).unwrap_or_else(|e| panic!("{spec}: {e}"));
        assert_eq!(network.scheme(), scheme, "{spec}");
        assert_eq!(network.label(), label, "{spec}");
        assert_eq!(network.row().scheme, scheme, "{spec}");
        assert_eq!(network.road(), Some(scheme), "{spec}");
        let checks = network.checks();
        assert_eq!(
            (checks[0].name.as_str(), checks[0].tls),
            (scheme, tls),
            "{spec}"
        );
    }
    // The SOCKS5 hop of a host named wisp, as it was.
    assert_eq!(parse("wisp:1080").unwrap().scheme(), "proxy");
}

/// One rule for each bad spec of a road: a usage line that shows no ID, no
/// key and no query.
#[test]
fn network_spec_a_bad_server_spec_is_a_usage_line_with_no_secret() {
    let uri =
        base64::engine::general_purpose::STANDARD.encode("aes-256-gcm:hunter2@h.example:8388");
    for spec in [
        "vless://hunter2@v.example:443?type=ws",
        "vless://v.example:443?type=tcp&security=tls",
        "vless://v.example:443?type=ws&security=reality",
        "vless://v.example?type=ws",
        "vless://",
        "ss://hunter2@ss.example:8388",
        &format!("ss://{uri}"),
        "ss://ss.example",
        "ss://ss.example:0",
        "ss://ss.example:8388/?plugin=hunter2",
        "ss://bad host:8388",
        "wisp:",
        // Built from its parts: a scanner of secrets takes a whole URL with
        // a password for a credential.
        &["wisp:wss://u:", "hunter2@w.example/"].concat(),
        "wisp:https://w.example/hunter2",
        "wisp:wss://w.example:99999/",
    ] {
        let err = parse(spec).expect_err(spec);
        assert_eq!(err.class, Class::Usage, "{spec}: {err}");
        assert!(!err.text().contains('\n'), "{spec}: {err}");
        assert!(!err.text().contains("hunter2"), "{spec}: {err}");
    }
}

/// One rule for each fault of a secret: a usage line that names the
/// variable and shows no value. Each fault comes before any connection.
#[tokio::test]
async fn a_server_road_secret_fault_names_the_variable_and_hides_the_value() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing").to_str().unwrap().to_string();
    let vless = "vless://v.example:443?type=ws&security=tls";
    let ss = "ss://ss.example:8388";
    let other = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode("aes-256-gcm:hunter2");
    let other = format!("ss://{other}@other.example:8388");
    let cases = [
        (vless, vars(&[]), None, VLESS_ID_FILE),
        (
            vless,
            vars(&[(VLESS_ID, "x".repeat(31).as_str())]),
            None,
            VLESS_ID,
        ),
        (
            vless,
            vars(&[(VLESS_ID, "hunter2-0000-0000-0000-000000000000")]),
            None,
            VLESS_ID,
        ),
        (
            vless,
            vars(&[(VLESS_ID_FILE, &missing)]),
            None,
            VLESS_ID_FILE,
        ),
        (
            vless,
            vars(&[(VLESS_ID, "a"), (VLESS_ID_FILE, &missing)]),
            None,
            VLESS_ID,
        ),
        (
            vless,
            vars(&[("PODNET_VLESS_ID_FILE_OFFICE_4G", &missing)]),
            Some("office-4g"),
            "PODNET_VLESS_ID_FILE_OFFICE_4G",
        ),
        (ss, vars(&[]), None, SHADOWSOCKS_KEY_FILE),
        (
            ss,
            vars(&[(SHADOWSOCKS_KEY, "hunter2")]),
            None,
            SHADOWSOCKS_KEY,
        ),
        (
            ss,
            vars(&[(SHADOWSOCKS_KEY, "hunter2:aes-256-gcm")]),
            None,
            SHADOWSOCKS_KEY,
        ),
        (
            ss,
            vars(&[(SHADOWSOCKS_KEY, "aes-256-cfb:hunter2")]),
            None,
            SHADOWSOCKS_KEY,
        ),
        (
            ss,
            vars(&[(SHADOWSOCKS_KEY, "aes-256-gcm:")]),
            None,
            SHADOWSOCKS_KEY,
        ),
        (
            ss,
            vars(&[(SHADOWSOCKS_KEY, "2022-blake3-aes-256-gcm:hunter2")]),
            None,
            SHADOWSOCKS_KEY,
        ),
        (
            ss,
            vars(&[(SHADOWSOCKS_KEY, "aes-256-gcm:a\nhunter2")]),
            None,
            SHADOWSOCKS_KEY,
        ),
        (
            ss,
            vars(&[(SHADOWSOCKS_KEY, &other)]),
            None,
            SHADOWSOCKS_KEY,
        ),
        (
            ss,
            vars(&[(SHADOWSOCKS_KEY_FILE, &missing)]),
            None,
            SHADOWSOCKS_KEY_FILE,
        ),
    ];
    for (spec, vars, name, named) in cases {
        let err = match up_road(spec, &vars, name).await {
            Ok(_) => panic!("{spec} {vars:?}: it came up"),
            Err(e) => e,
        };
        assert_eq!(err.class, Class::Usage, "{spec} {vars:?}: {err}");
        assert!(err.text().contains(named), "{spec} {vars:?}: {err}");
        assert!(!err.text().contains("hunter2"), "{spec} {vars:?}: {err}");
        assert!(!err.text().contains('\n'), "{spec} {vars:?}: {err}");
    }
}

/// The ID from a file and from a variable carries the bytes over a
/// WebSocket with TLS. A wrong ID ends the stream before VLESS's answer, and
/// the read says so.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn vless_dialer_through_a_stand_in_server() {
    pod_testkit::need_loopback!();
    let (server, echo) = (stand::vless().await, road_stand::echo().await);
    let spec = vless_spec(server.addr);
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("id");
    std::fs::write(&file, format!("{}\n", stand::VLESS_ID)).unwrap();
    let file = file.to_str().unwrap();
    for (given, form) in [
        (vars(&[(VLESS_ID_FILE, file)]), "file"),
        (vars(&[(VLESS_ID, stand::VLESS_ID)]), "variable"),
    ] {
        let link = up_road(&spec, &given, None)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(link.status["id"], form, "{}", link.status);
        assert_eq!(link.status["transport"], "a WebSocket", "{}", link.status);
        assert_eq!(
            link.way.as_ref().unwrap().label(),
            format!("vless://{}/vless", server.addr)
        );
        assert!(!link.status.to_string().contains(stand::VLESS_ID));
        round_trip(&link, echo, b"through VLESS").await.unwrap();
    }
    assert_eq!(server.seen.lock().unwrap().targets.len(), 2);
    let link = up_road(&spec, &vars(&[(VLESS_ID, "another-id")]), None)
        .await
        .unwrap();
    let err = exchange(&link, Target::new("127.0.0.1", echo.port()), b"x")
        .await
        .expect_err("a wrong ID");
    assert!(err.contains("refused the ID"), "{err}");
    assert_eq!(server.seen.lock().unwrap().refused, 1);
}

/// `METHOD:PASSWORD` from a file and from a variable, and the `ss://` URI
/// of the server, carry the bytes. A wrong password gets no answer.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shadowsocks_dialer_through_a_stand_in_server() {
    pod_testkit::need_loopback!();
    let (server, echo) = (stand::shadowsocks().await, road_stand::echo().await);
    let spec = format!("ss://{}", server.addr);
    let key = format!("{}:{}", stand::SS_METHOD, stand::SS_PASSWORD);
    let userinfo = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&key);
    let uri = format!("ss://{userinfo}@{}#home", server.addr);
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("key");
    std::fs::write(&file, format!("{key}\n")).unwrap();
    let file = file.to_str().unwrap();
    for (given, form) in [
        (vars(&[(SHADOWSOCKS_KEY_FILE, file)]), "file"),
        (vars(&[(SHADOWSOCKS_KEY, &key)]), "variable"),
        (vars(&[(SHADOWSOCKS_KEY, &uri)]), "variable"),
    ] {
        let link = up_road(&spec, &given, None)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(link.status["key"], form, "{}", link.status);
        assert_eq!(link.status["method"], stand::SS_METHOD, "{}", link.status);
        assert_eq!(link.way.as_ref().unwrap().label(), spec);
        assert!(!link.status.to_string().contains(stand::SS_PASSWORD));
        round_trip(&link, echo, b"through Shadowsocks")
            .await
            .unwrap();
    }
    assert_eq!(server.seen.lock().unwrap().targets.len(), 3);
    let wrong = vars(&[(SHADOWSOCKS_KEY, "aes-256-gcm:hunter2")]);
    let link = up_road(&spec, &wrong, None).await.unwrap();
    let err = exchange(&link, Target::new("127.0.0.1", echo.port()), b"x")
        .await
        .expect_err("a wrong password");
    assert!(!err.contains("hunter2"), "{err}");
    assert_eq!(server.seen.lock().unwrap().refused, 1);
}

/// Two streams of one connection carry the bytes; a target that the server
/// cannot reach is a failure with the reason of the server; a connection
/// that the server closed is opened again by the next dial.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wisp_dialer_through_a_stand_in_server() {
    pod_testkit::need_loopback!();
    let (server, echo) = (stand::wisp().await, road_stand::echo().await);
    let at = server.stand.addr;
    let spec = format!("wisp:wss://{at}{}", stand::WISP_PATH);
    let link = up_road(&spec, &vars(&[]), None)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(link.status["version"], "2.1", "{}", link.status);
    assert_eq!(link.status["confirms"], true, "{}", link.status);
    assert_eq!(link.way.as_ref().unwrap().label(), spec);
    round_trip(&link, echo, b"first through Wisp")
        .await
        .unwrap();
    round_trip(&link, echo, b"second through Wisp")
        .await
        .unwrap();
    assert_eq!(*server.connections.lock().unwrap(), 1, "one WebSocket");
    let closed = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let err = exchange(&link, Target::new("127.0.0.1", closed.port()), b"x")
        .await
        .expect_err("a refused target");
    assert!(err.contains("0x44"), "{err}");
    server.close_all.notify_waiters();
    tokio::time::sleep(Duration::from_millis(200)).await;
    round_trip(&link, echo, b"after the close").await.unwrap();
    assert_eq!(*server.connections.lock().unwrap(), 2, "a new WebSocket");
    assert_eq!(server.stand.seen.lock().unwrap().targets.len(), 3);
}

/// A server that does not answer fails at `up`, and the line starts with
/// the spec and names the hop.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_server_road_with_no_server_fails_at_up_and_names_the_hop() {
    pod_testkit::need_loopback!();
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    for (spec, given, hop) in [
        (
            vless_spec(format!("127.0.0.1:{port}").parse().unwrap()),
            vars(&[(VLESS_ID, stand::VLESS_ID)]),
            "the VLESS server",
        ),
        (
            format!("ss://127.0.0.1:{port}"),
            vars(&[(SHADOWSOCKS_KEY, "aes-256-gcm:pw")]),
            "the Shadowsocks server",
        ),
        (
            format!("wisp:wss://127.0.0.1:{port}/"),
            vars(&[]),
            "the Wisp server",
        ),
    ] {
        let err = up_road(&spec, &given, None).await.expect_err("no server");
        let label = parse(&spec).unwrap().label();
        assert!(err.text().starts_with(&format!("{label}: {hop}")), "{err}");
    }
}
