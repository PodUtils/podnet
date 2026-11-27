//! End-to-end MASQUE QUIC-1 to QUIC-5 integration tests (T-367, GitHub #486).
//!
//! Exercises one QUIC/HTTP/3 stack (`pod-quic` / `noq` + `pod-warp::h3`) with
//! a local self-signed MASQUE stand-in server:
//! - CONNECT-IP (RFC 9484) to a non-WARP endpoint via `--masque-endpoint`
//! - CONNECT-UDP (RFC 9298) through MASQUE to a local UDP echo server
//! - WebTransport over the same QUIC/HTTP/3 connection
//! - IPv6-only host rule with zero IPv4 socket or connect attempts
//! - Two enrolment providers (`WarpEnrolment`, `TokenEnrolment`) on one suite
//! - Enrolment versus transport failure reporting in library and CLI

use std::net::SocketAddr;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use pod_testkit::stand;
use pod_warp::{Device, DeviceHints, Identity};
use podnet::warp::masque::standin::{generate_cert, StandinMasque};
use podnet::warp::masque::{
    AttemptKind, AttemptTracker, IpPolicy, MasqueClient, MasqueEndpoint, MasqueEnrolment,
    MasqueError, TokenEnrolment, WarpEnrolment,
};
use tokio::net::UdpSocket;

const WAIT: Duration = Duration::from_secs(5);

async fn start_udp_echo(bind: SocketAddr) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let sock = UdpSocket::bind(bind).await.expect("bind local UDP echo");
    let addr = sock.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let mut buf = vec![0u8; 65535];
        while let Ok((n, from)) = sock.recv_from(&mut buf).await {
            let _ = sock.send_to(&buf[..n], from).await;
        }
    });
    (addr, handle)
}

fn warp_device_for(door: SocketAddr, spki_der: &[u8]) -> Device {
    use base64::Engine as _;
    let b64 = base64::engine::general_purpose::STANDARD.encode(spki_der);
    let pem = format!("-----BEGIN PUBLIC KEY-----\n{b64}\n-----END PUBLIC KEY-----\n");
    let (v4, v6) = match door {
        SocketAddr::V4(a) => (a.to_string(), "[2606:4700:103::1]:443".into()),
        SocketAddr::V6(a) => ("162.159.198.1:443".into(), a.to_string()),
    };
    serde_json::from_value(serde_json::json!({
        "id": "dev-masque-test",
        "token": "tok-masque-test",
        "tunnel_type": "masque",
        "key_type": "secp256r1",
        "account": { "id": "acct-test", "account_type": "free" },
        "config": {
            "interface": { "addresses": { "v4": "172.16.0.2", "v6": "2606:4700:110::2" } },
            "peers": [{ "public_key": pem, "endpoint": { "v4": v4, "v6": v6, "host": "localhost:443" } }]
        }
    }))
    .unwrap()
}

/// Shared session test suite run identically against multiple providers (QUIC-4).
async fn run_provider_session_suite(enrolment: Arc<dyn MasqueEnrolment>, echo_addr: SocketAddr) {
    let client = MasqueClient::new(enrolment)
        .with_ip_policy(IpPolicy::DualStack)
        .with_timeout(WAIT);
    let conn = tokio::time::timeout(WAIT, client.connect())
        .await
        .unwrap()
        .expect("provider connect");

    // 1. CONNECT-IP datagram and capsule round-trip.
    let mut ip = conn.open_connect_ip().await.expect("open CONNECT-IP");
    let pkt = b"\x45\x00\x00\x1cprovider-ip-datagram";
    ip.send_packet(pkt).await.unwrap();
    let got = tokio::time::timeout(WAIT, ip.recv_packet())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(got, pkt);

    let big = vec![0x45u8; 1600];
    ip.send_capsule(&big).await.unwrap();
    let got_big = tokio::time::timeout(WAIT, ip.recv_packet())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(got_big, big);

    // 2. CONNECT-UDP round-trip through the stand-in to the local UDP echo server.
    let mut udp = conn
        .open_connect_udp(&echo_addr.ip().to_string(), echo_addr.port())
        .await
        .expect("open CONNECT-UDP");
    let msg = format!("udp-via-{}", conn.provider());
    udp.send(msg.as_bytes()).await.unwrap();
    let echoed = tokio::time::timeout(WAIT, udp.recv())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(echoed, msg.as_bytes());

    // 3. WebTransport bidirectional stream and datagram on the SAME connection.
    let mut wt = conn
        .open_webtransport("/wt")
        .await
        .expect("open WebTransport");
    let (mut bi_tx, mut bi_rx) = wt.open_bi().await.expect("open WebTransport bidi stream");
    let wt_msg = format!("wt-bidi-{}", conn.provider());
    bi_tx.write_all(wt_msg.as_bytes()).await.unwrap();
    bi_tx.finish().unwrap();
    let echoed_bi = tokio::time::timeout(WAIT, bi_rx.read_to_end(4096))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(echoed_bi, wt_msg.as_bytes());

    let wt_dgram = format!("wt-dgram-{}", conn.provider());
    wt.send_datagram(wt_dgram.as_bytes()).await.unwrap();
    let echoed_dgram = tokio::time::timeout(WAIT, wt.recv_datagram())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(echoed_dgram, wt_dgram.as_bytes());

    ip.close();
    udp.close();
    wt.close();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connect_ip_to_non_warp_endpoint() {
    pod_testkit::need_loopback!();
    let server = StandinMasque::start("127.0.0.1:0".parse().unwrap(), Some("tok-ip".into()));
    let endpoint = MasqueEndpoint::parse(&format!("https://{}", server.addr)).unwrap();
    let enrolment = Arc::new(
        TokenEnrolment::new(endpoint)
            .with_provider_name("custom-masque")
            .with_bearer_token("tok-ip")
            .with_ca_der(server.ca_der.clone())
            .with_sni("localhost"),
    );
    let mut ip = MasqueClient::new(enrolment)
        .with_ip_policy(IpPolicy::DualStack)
        .with_timeout(WAIT)
        .connect_ip()
        .await
        .expect("CONNECT-IP to non-WARP endpoint");
    let packet = b"\x45\x00\x00\x14non-warp-connect-ip";
    ip.send_packet(packet).await.unwrap();
    let got = tokio::time::timeout(WAIT, ip.recv_packet())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(got, packet);
    ip.close();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connect_udp_through_masque_to_local_udp_echo() {
    pod_testkit::need_loopback!();
    let server = StandinMasque::start("127.0.0.1:0".parse().unwrap(), None);
    let (echo_addr, echo_task) = start_udp_echo("127.0.0.1:0".parse().unwrap()).await;
    let enrolment = Arc::new(
        TokenEnrolment::new(MasqueEndpoint::from_socket_addr(server.addr))
            .with_ca_der(server.ca_der.clone())
            .with_sni("localhost"),
    );
    let mut udp = MasqueClient::new(enrolment)
        .with_ip_policy(IpPolicy::DualStack)
        .with_timeout(WAIT)
        .connect_udp("127.0.0.1", echo_addr.port())
        .await
        .expect("CONNECT-UDP session");
    for idx in 0..5u32 {
        let payload = format!("masque-udp-echo-{idx}");
        udp.send(payload.as_bytes()).await.unwrap();
        let reply = tokio::time::timeout(WAIT, udp.recv())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(reply, payload.as_bytes());
    }
    udp.close();
    echo_task.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn webtransport_and_all_three_modes_share_one_quic_connection() {
    pod_testkit::need_loopback!();
    let server = StandinMasque::start("127.0.0.1:0".parse().unwrap(), Some("shared-tok".into()));
    let (echo_addr, echo_task) = start_udp_echo("127.0.0.1:0".parse().unwrap()).await;
    let endpoint = MasqueEndpoint::parse(&format!("https://{}", server.addr)).unwrap();
    let enrolment = Arc::new(
        TokenEnrolment::new(endpoint)
            .with_provider_name("multi-mode")
            .with_bearer_token("shared-tok")
            .with_ca_der(server.ca_der.clone())
            .with_sni("localhost"),
    );
    run_provider_session_suite(enrolment, echo_addr).await;
    echo_task.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ipv6_only_host_makes_zero_ipv4_attempts() {
    pod_testkit::need_loopback!();
    if UdpSocket::bind("[::1]:0").await.is_err() {
        eprintln!("SKIP ipv6_only_host_makes_zero_ipv4_attempts: IPv6 loopback unavailable");
        return;
    }
    let server = StandinMasque::start("[::1]:0".parse().unwrap(), None);
    let (echo_addr, echo_task) = start_udp_echo("[::1]:0".parse().unwrap()).await;
    let tracker = AttemptTracker::new();
    let endpoint = MasqueEndpoint::parse(&format!("https://{}", server.addr)).unwrap();
    let enrolment = Arc::new(
        TokenEnrolment::new(endpoint)
            .with_endpoints(vec!["127.0.0.1:1".parse().unwrap(), server.addr])
            .with_ca_der(server.ca_der.clone())
            .with_sni("localhost"),
    );
    let conn = MasqueClient::new(enrolment)
        .with_ip_policy(IpPolicy::Ipv6Only)
        .with_tracker(tracker.clone())
        .with_timeout(WAIT)
        .connect()
        .await
        .expect("IPv6-only connect");
    assert!(conn.peer_addr().is_ipv6());
    let mut udp = conn
        .open_connect_udp("::1", echo_addr.port())
        .await
        .unwrap();
    udp.send(b"ipv6-only-echo").await.unwrap();
    let echoed = tokio::time::timeout(WAIT, udp.recv())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(echoed, b"ipv6-only-echo");
    udp.close();

    assert!(tracker.ipv4_attempts().is_empty(), "{:?}", tracker.lines());
    assert_eq!(tracker.refused_ipv4().len(), 1);
    let v6 = tracker.ipv6_attempts();
    assert!(v6
        .iter()
        .any(|a| a.kind == AttemptKind::Bind && a.addr == "[::]:0".parse().unwrap()));
    assert!(v6
        .iter()
        .any(|a| a.kind == AttemptKind::Connect && a.addr == server.addr));

    let v4_tracker = AttemptTracker::new();
    let v4_only = Arc::new(
        TokenEnrolment::new(MasqueEndpoint::parse("https://127.0.0.1:443").unwrap())
            .with_ca_der(server.ca_der.clone())
            .with_sni("localhost"),
    );
    let err = MasqueClient::new(v4_only)
        .with_ip_policy(IpPolicy::Ipv6Only)
        .with_tracker(v4_tracker.clone())
        .connect()
        .await
        .err()
        .expect("IPv4-only target must fail under Ipv6Only");
    assert!(err.is_transport(), "{err}");
    assert!(v4_tracker.ipv4_attempts().is_empty());
    assert_eq!(v4_tracker.refused_ipv4().len(), 1);
    echo_task.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_providers_pass_the_same_session_suite() {
    pod_testkit::need_loopback!();
    let (echo_addr, echo_task) = start_udp_echo("127.0.0.1:0".parse().unwrap()).await;

    // Provider 1: WarpEnrolment (pinned peer SPKI + P-256 identity, cf-connect-ip).
    let warp_server = StandinMasque::start("127.0.0.1:0".parse().unwrap(), None);
    let identity = Identity::generate().unwrap();
    let device = warp_device_for(warp_server.addr, &warp_server.leaf_spki_der);
    let warp_provider: Arc<dyn MasqueEnrolment> = Arc::new(
        WarpEnrolment::from_parts(device, identity)
            .with_endpoint(MasqueEndpoint::from_socket_addr(warp_server.addr)),
    );
    run_provider_session_suite(warp_provider, echo_addr).await;

    // Provider 2: TokenEnrolment (custom CA + Bearer token, RFC 9484 connect-ip).
    let token_server =
        StandinMasque::start("127.0.0.1:0".parse().unwrap(), Some("bearer-42".into()));
    let token_provider: Arc<dyn MasqueEnrolment> = Arc::new(
        TokenEnrolment::new(
            MasqueEndpoint::parse(&format!("https://{}", token_server.addr)).unwrap(),
        )
        .with_provider_name("token-masque")
        .with_bearer_token("bearer-42")
        .with_ca_der(token_server.ca_der.clone())
        .with_sni("localhost"),
    );
    run_provider_session_suite(token_provider, echo_addr).await;
    echo_task.abort();
}

fn start_refusing_api(status: u16) -> SocketAddr {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        use std::io::{Read as _, Write as _};
        while let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let body = r#"{"errors":[{"code":1000,"message":"enrolment refused"}]}"#;
            let resp = format!(
                "HTTP/1.1 {status} Refused\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(resp.as_bytes());
        }
    });
    addr
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn enrolment_and_transport_failures_are_distinct_in_library_and_cli() {
    pod_testkit::need_loopback!();

    // 1. Library enrolment failure: empty bearer token.
    let bad_token = Arc::new(
        TokenEnrolment::new(MasqueEndpoint::parse("https://127.0.0.1:8443").unwrap())
            .with_bearer_token(""),
    );
    let enrol_err = MasqueClient::new(bad_token).connect().await.err().unwrap();
    assert!(
        matches!(enrol_err, MasqueError::Enrolment(_)),
        "{enrol_err}"
    );
    assert_eq!(enrol_err.kind(), "enrolment");
    assert!(enrol_err.status_line().contains("enrolment failed:"));
    assert!(!enrol_err.status_line().contains("transport"));

    // 2. Library enrolment failure: WarpEnrolment against refusing HTTP 403 API.
    let refusing = start_refusing_api(403);
    let api_client = pod_proxy::Client::new(
        podnet::trust::context().clone(),
        podnet::egress::egress(None),
    );
    let api = pod_warp::Api::new(api_client).with_bases(vec![format!("http://{refusing}")]);
    let warp_err = MasqueClient::new(Arc::new(WarpEnrolment::from_api(
        api,
        DeviceHints::default(),
    )))
    .connect()
    .await
    .err()
    .unwrap();
    assert!(matches!(warp_err, MasqueError::Enrolment(_)), "{warp_err}");
    assert_eq!(warp_err.kind(), "enrolment");

    // 3. Library transport failure: valid enrolment, untrusted server cert.
    let server = StandinMasque::start("127.0.0.1:0".parse().unwrap(), None);
    let bad_transport = Arc::new(
        TokenEnrolment::new(MasqueEndpoint::from_socket_addr(server.addr))
            .with_ca_der(generate_cert().ca_der)
            .with_sni("localhost"),
    );
    let transport_err = MasqueClient::new(bad_transport)
        .with_ip_policy(IpPolicy::DualStack)
        .with_timeout(Duration::from_secs(2))
        .connect()
        .await
        .err()
        .unwrap();
    assert!(
        matches!(transport_err, MasqueError::Transport(_)),
        "{transport_err}"
    );
    assert_eq!(transport_err.kind(), "transport");
    assert!(transport_err.status_line().contains("transport failed:"));
    assert!(!transport_err.status_line().contains("enrolment"));

    // 4. CLI `warp status --json` distinguishes enrolment vs transport failures.
    let api_standin = stand::warp::Api::new("", "192.0.2.1", "2001:db8::1");
    let served = stand::http(api_standin.handler()).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let (state, run) = (dir.path().join("state"), dir.path().join("run"));
    std::fs::create_dir_all(&state).unwrap();
    std::fs::create_dir_all(&run).unwrap();
    let bin = env!("CARGO_BIN_EXE_podnet");
    let run_warp = |api_url: &str, extra: &[&str]| {
        Command::new(bin)
            .args(["warp", "--no-proxy"])
            .args(extra)
            .env("PODNET_STATE_DIR", &state)
            .env("PODNET_RUN_DIR", &run)
            .env("PODNET_WARP_API", api_url)
            .env_remove("HTTPS_PROXY")
            .env_remove("HTTP_PROXY")
            .env_remove("ALL_PROXY")
            .env_remove("PODNET_PROXY")
            .output()
            .unwrap()
    };

    let reg = run_warp(&format!("http://{}", served.addr()), &["register"]);
    assert!(
        reg.status.success(),
        "{}",
        String::from_utf8_lossy(&reg.stderr)
    );

    // 4a. Enrolment failure in `warp status --json` with `--masque-endpoint`.
    let out = run_warp(
        &format!("http://{refusing}"),
        &[
            "--masque-endpoint",
            "https://[::1]:8443",
            "status",
            "--json",
        ],
    );
    assert_eq!(out.status.code(), Some(77));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("state       enrolment failed:"), "{stderr}");
    assert!(!stderr.contains("transport failed"), "{stderr}");
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["state"], "enrolment failed");
    assert_eq!(doc["failure_kind"], "enrolment");

    // 4b. Transport failure in `warp status --json` with `--masque-endpoint`.
    let paths = podnet::instances::paths(&run, "warp").unwrap();
    let _held = podnet::instances::lock(&paths.lock).unwrap();
    let record = podnet::instances::Record {
        name: "warp".into(),
        pid: std::process::id(),
        state: "ready".into(),
        ready: Some(serde_json::json!({
            "listens": ["unix:/r/warp.sock"],
            "data_path": "tcp-dead",
            "last_probe": 0,
        })),
        ..Default::default()
    };
    podnet::instances::write(&run, &record).unwrap();
    let out = run_warp(
        &format!("http://{}", served.addr()),
        &[
            "--masque-endpoint",
            "https://127.0.0.1:8443",
            "status",
            "--json",
        ],
    );
    assert_eq!(out.status.code(), Some(69));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("state       transport failed:"), "{stderr}");
    assert!(!stderr.contains("enrolment failed"), "{stderr}");
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["state"], "transport failed");
    assert_eq!(doc["failure_kind"], "transport");
    assert_eq!(doc["masque_endpoint"], "https://127.0.0.1:8443");

    // 4c. Bad `--masque-endpoint` value exits 64.
    let bad = run_warp(
        &format!("http://{}", served.addr()),
        &["--masque-endpoint", "not-an-endpoint", "status"],
    );
    assert_eq!(bad.status.code(), Some(64));
}
