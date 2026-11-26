//! `https://` and `wstunnel:` (T-126): the specs, the secrets, and a dial
//! through each road against the stand-ins of `road_stand.rs` on the
//! loopback, with the CA of the stand as the only root.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::road_stand::{self as stand, Stand};
use super::turn_mock;
use super::*;
use crate::dialers::Direct;

const WAIT: Duration = Duration::from_secs(20);

/// A case of a secret fault: the spec, the variables, the name of the
/// router network, and the text that the line must name.
type Case<'a> = (&'a str, HashMap<String, String>, Option<&'a str>, &'a str);

/// The context and the trust of the stands: their CA alone, in a file of
/// this module (the file of `turn_mock::client_context` is written again by
/// each call, so a parallel read can see it half written).
pub(super) fn stand_tls() -> (Arc<pod_tls::Context>, pod_tls::Trust) {
    static MADE: std::sync::OnceLock<(Arc<pod_tls::Context>, pod_tls::Trust)> =
        std::sync::OnceLock::new();
    MADE.get_or_init(|| {
        let dir = std::env::temp_dir();
        let path = podnet_core::state::temp::random_path(&dir, "pn-road-ca-", ".pem");
        std::fs::write(&path, stand::ca_pem()).expect("the CA of the stands");
        let mut settings = pod_tls::Settings::new(pod_tls::Words::new("road stand"));
        settings.ca_file = Some(pod_tls::CaFile::new("the CA of the road stands", path));
        let tls = Arc::new(pod_tls::Context::new(turn_mock::provider(), settings));
        let config = tls
            .client_config_with(&[], &[])
            .expect("the CA of the stand");
        let trust = pod_tls::Trust::caller(Arc::new(config)).expect("a trust with no ALPN");
        (tls, trust)
    })
    .clone()
}

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
        Kind::Via(spec) => {
            let settings = via::settings(&get, name)?;
            via::up_with(&spec, settings, Some(&below), tls, trust).await
        }
        Kind::Wstunnel(spec) => {
            let settings = wstunnel::settings(&get, name)?;
            wstunnel::up_with(&spec, settings, Some(&below), tls, trust).await
        }
        other => panic!("{spec}: not a road: {other:?}"),
    }
}

pub(super) fn vars(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// Send `text` through `link` to the echo target `echo`, and read it back.
pub(super) async fn round_trip(
    link: &Link,
    echo: std::net::SocketAddr,
    text: &[u8],
) -> Result<(), Failure> {
    let way = link.way.clone().expect("the link has a way");
    let target = Target::new("127.0.0.1", echo.port());
    let mut io = way
        .dial
        .dial(&target, tokio::time::Instant::now() + WAIT)
        .await?
        .io;
    io.write_all(text).await.unwrap();
    let mut back = vec![0u8; text.len()];
    tokio::time::timeout(WAIT, io.read_exact(&mut back))
        .await
        .expect("the echo in time")
        .unwrap();
    assert_eq!(back, text);
    Ok(())
}

#[test]
fn network_spec_the_roads_of_pod_egress_have_a_scheme_and_a_label() {
    for (spec, scheme, label) in [
        (
            "https://proxy.example",
            "https",
            "https://proxy.example:443",
        ),
        (
            "https://user@proxy.example:8443",
            "https",
            "https://proxy.example:8443",
        ),
        (
            "HTTPS://proxy.example:3128/",
            "https",
            "https://proxy.example:3128",
        ),
        (
            "wstunnel:wss://ws.example",
            "wstunnel",
            "wstunnel:wss://ws.example",
        ),
        (
            "wstunnel:ws://127.0.0.1:8080",
            "wstunnel",
            "wstunnel:ws://127.0.0.1:8080",
        ),
        (
            "wstunnel:ws.example:8443",
            "wstunnel",
            "wstunnel:wss://ws.example:8443",
        ),
        (
            "wstunnel://ws.example",
            "wstunnel",
            "wstunnel:wss://ws.example",
        ),
        // The SOCKS5 hop of a host named wstunnel, as it was.
        ("wstunnel:1080", "proxy", "wstunnel:1080"),
    ] {
        let network = parse(spec).unwrap_or_else(|e| panic!("{spec}: {e}"));
        assert_eq!(network.scheme(), scheme, "{spec}");
        assert_eq!(network.label(), label, "{spec}");
        assert_eq!(network.row().scheme, scheme, "{spec}");
        assert_eq!(network.road().is_some(), scheme != "proxy", "{spec}");
    }
    let checks = parse("wstunnel:ws://127.0.0.1:8080").unwrap().checks();
    assert_eq!(checks[0].target, Target::new("127.0.0.1", 8080));
    assert!(!checks[0].tls);
    let checks = parse("https://proxy.example").unwrap().checks();
    assert_eq!(
        (checks[0].name.as_str(), checks[0].tls),
        ("https-proxy", true)
    );
}

/// One rule for each bad spec of a road: a usage line that shows no user,
/// password, path or query.
#[test]
fn network_spec_a_bad_road_spec_is_a_usage_line_with_no_secret() {
    for spec in [
        "https://user:hunter2@proxy.example",
        "https://proxy.example/hunter2",
        "https://proxy.example?key=hunter2",
        "https://",
        "https://proxy.example:0",
        "wstunnel:",
        "wstunnel:https://ws.example",
        "wstunnel:wts://ws.example",
        "wstunnel:wss://ws.example/hunter2",
        "wstunnel:wss://ws.example?hunter2",
        "wstunnel:wss://user:hunter2@ws.example",
        "wstunnel:ftp://ws.example",
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
async fn a_road_secret_fault_names_the_variable_and_hides_the_value() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("auth");
    std::fs::write(&file, "proxy-user:hunter2\n").unwrap();
    let file = file.to_str().unwrap();
    let missing = dir.path().join("missing").to_str().unwrap().to_string();
    let (auth_file, auth) = (via::AUTH_FILE_VARIABLE, via::AUTH_VARIABLE);
    let (secret_file, secret) = (wstunnel::SECRET_FILE_VARIABLE, wstunnel::SECRET_VARIABLE);
    let cases: Vec<Case<'_>> = vec![
        (
            "https://proxy-user@proxy.example",
            vars(&[]),
            None,
            auth_file,
        ),
        (
            "https://proxy.example",
            vars(&[(auth_file, &missing)]),
            None,
            auth_file,
        ),
        (
            "https://proxy.example",
            vars(&[(auth_file, file), (auth, "u:hunter2")]),
            None,
            auth,
        ),
        (
            "https://proxy.example",
            vars(&[(auth, "hunter2")]),
            None,
            auth,
        ),
        ("https://proxy.example", vars(&[(auth, "  ")]), None, auth),
        (
            "https://proxy.example",
            vars(&[(auth, "u:hunter2\nv:hunter2")]),
            None,
            auth,
        ),
        (
            "https://proxy.example",
            vars(&[(via::VERSION_VARIABLE, "3")]),
            None,
            via::VERSION_VARIABLE,
        ),
        (
            "https://proxy.example",
            vars(&[("PODNET_HTTPS_PROXY_AUTH_FILE_OFFICE_4G", &missing)]),
            Some("office-4g"),
            "PODNET_HTTPS_PROXY_AUTH_FILE_OFFICE_4G",
        ),
        (
            "wstunnel:wss://ws.example",
            vars(&[(secret, "bogus hunter2")]),
            None,
            secret,
        ),
        (
            "wstunnel:wss://ws.example",
            vars(&[(secret, "path-prefix a/hunter2")]),
            None,
            secret,
        ),
        (
            "wstunnel:wss://ws.example",
            vars(&[(secret_file, &missing)]),
            None,
            secret_file,
        ),
        (
            "wstunnel:ws://ws.example",
            vars(&[(secret, "hunter2")]),
            None,
            "wss://",
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

/// TLS to the proxy with the CA of the stand, then CONNECT on HTTP/1.1 with
/// Basic: both forms of the credentials carry the bytes. A wrong password
/// fails the dial with the 407 of the proxy, which names it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn https_proxy_dialer_through_a_test_ca() {
    pod_testkit::need_loopback!();
    let (proxy, echo): (Stand, _) = (stand::https_proxy().await, stand::echo().await);
    let at = proxy.addr;
    let line = format!("{}:{}", stand::USER, stand::PASSWORD);
    let dir = tempfile::tempdir().unwrap();
    let password = dir.path().join("password");
    std::fs::write(&password, format!("{}\n", stand::PASSWORD)).unwrap();
    let password = password.to_str().unwrap();
    for (spec, given, form) in [
        (
            format!("https://{at}"),
            vars(&[(via::AUTH_VARIABLE, &line)]),
            "variable",
        ),
        (
            format!("https://{}@{at}", stand::USER),
            vars(&[(via::AUTH_FILE_VARIABLE, password)]),
            "file",
        ),
    ] {
        let link = up_road(&spec, &given, None)
            .await
            .unwrap_or_else(|e| panic!("{spec}: {e}"));
        assert_eq!(link.status["credentials"], form, "{}", link.status);
        assert_eq!(link.way.as_ref().unwrap().label(), format!("https://{at}"));
        assert!(!link.status.to_string().contains(stand::PASSWORD));
        round_trip(&link, echo, b"through the HTTPS proxy")
            .await
            .unwrap();
    }
    assert_eq!(proxy.seen.lock().unwrap().targets.len(), 2);
    let wrong = vars(&[(via::AUTH_VARIABLE, "proxy-user:hunter2")]);
    let link = up_road(&format!("https://{at}"), &wrong, None)
        .await
        .unwrap();
    let err = round_trip(&link, echo, b"x").await.expect_err("a 407");
    let text = err.text();
    assert!(
        text.contains("407") && text.contains(&format!("https://{at}")),
        "{text}"
    );
    assert!(!text.contains("hunter2"), "{text}");
    assert_eq!(proxy.seen.lock().unwrap().refused, 1);
}

/// A proxy whose certificate the roots do not hold fails at `up`, and the
/// line names the proxy.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn https_proxy_with_an_unknown_ca_fails_at_up_and_names_the_proxy() {
    pod_testkit::need_loopback!();
    let proxy = stand::https_proxy().await;
    let spec = parse(&format!("https://{}", proxy.addr)).unwrap();
    let Kind::Via(spec) = spec.kind else {
        panic!("not https")
    };
    let settings = via::settings(&|_: &str| None, None).unwrap();
    let below: Arc<dyn Dial> = Arc::new(Direct);
    let tls = Arc::new(pod_tls::Context::new(
        turn_mock::provider(),
        pod_tls::Settings::new(pod_tls::Words::new("test")),
    ));
    let err = via::up_with(&spec, settings, Some(&below), tls, pod_tls::Trust::Default)
        .await
        .expect_err("an unknown CA");
    let text = err.text();
    assert!(
        text.starts_with(&format!("https://{}: the proxy: TLS", proxy.addr)),
        "{text}"
    );
}

/// The upgrade with the prefix of the secret and the token of the target,
/// the bytes in frames, with the CA of the stand. A wrong prefix fails the
/// dial with the 400 of the server, which names it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wstunnel_dialer_through_a_stand_in_server() {
    pod_testkit::need_loopback!();
    let (server, echo) = (stand::wstunnel().await, stand::echo().await);
    let spec = format!("wstunnel:wss://{}", server.addr);
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("secret");
    std::fs::write(
        &file,
        format!(
            "# the secret of the server\npath-prefix {}\n",
            stand::PREFIX
        ),
    )
    .unwrap();
    let file = file.to_str().unwrap();
    for (given, form) in [
        (vars(&[(wstunnel::SECRET_FILE_VARIABLE, file)]), "file"),
        (
            vars(&[(wstunnel::SECRET_VARIABLE, stand::PREFIX)]),
            "variable",
        ),
    ] {
        let link = up_road(&spec, &given, None)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(link.status["secret"], form, "{}", link.status);
        assert_eq!(link.way.as_ref().unwrap().label(), spec);
        assert!(!link.status.to_string().contains(stand::PREFIX));
        round_trip(&link, echo, b"through wstunnel").await.unwrap();
    }
    let targets = server.seen.lock().unwrap().targets.clone();
    assert_eq!(targets, vec![format!("127.0.0.1:{}", echo.port()); 2]);
    // The defaults of wstunnel: the prefix v1, which this server refuses.
    let link = up_road(&spec, &vars(&[]), None).await.unwrap();
    assert_eq!(link.status["secret"], "default");
    let err = round_trip(&link, echo, b"x").await.expect_err("a 400");
    let text = err.text();
    assert!(
        text.contains("400") && text.contains(&format!("wss://{}", server.addr)),
        "{text}"
    );
    assert_eq!(server.seen.lock().unwrap().refused, 1);
}

/// A server that does not answer fails at `up`, and the line names it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wstunnel_dialer_with_no_server_fails_at_up_and_names_the_server() {
    pod_testkit::need_loopback!();
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let spec = format!("wstunnel:wss://127.0.0.1:{port}");
    let err = up_road(&spec, &vars(&[]), None)
        .await
        .expect_err("no server");
    assert!(
        err.text()
            .starts_with(&format!("{spec}: the wstunnel server")),
        "{err}"
    );
}
