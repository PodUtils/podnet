//! Integration tests for T-368 (GitHub #489): third-party roads, watch degradation,
//! staleness checks, `front=` health probes, fallback adapter class, and `status`
//! dependency counting.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use podnet::carriers;
use podnet::roads::third_party::irc::{IrcEndpoint, IrcStream};
use podnet::roads::third_party::mqtt::{
    read_packet, write_packet, MqttClient, MqttEndpoint, Packet,
};
use podnet::roads::third_party::syncthing::{
    dial_tcp, dial_tls, join_relay, join_session, read_message, write_message, RelayMessage,
    SyncthingEndpoint,
};
use podnet::roads::third_party::{
    check_front, classify_road, default_registry, parse_front_param, probe_road, select_road,
    RoadCandidate, RoadClass, SelectionPolicy, ThirdPartyKind, ThirdPartyRoad, VerifiedDate,
    WatchState, ADAPTER_REGISTRY,
};
use podnet::status::{self, DependencyClass, DependencyCollector};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc, Mutex};

#[test]
fn third_party_roads_appear_in_carriers_with_cost_and_verified_date() {
    let roads = carriers::third_party_roads();
    let expected = ["irc", "mqtt", "syncthing", "piping", "wormhole-transit"];
    let names: Vec<&str> = roads.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, expected);

    for road in &roads {
        assert!(!road.enabled, "{} must start disabled", road.name);
        assert!(
            matches!(road.cost_name(), "low" | "medium" | "high" | "highest"),
            "{} cost {}",
            road.name,
            road.cost_name()
        );
        let line = road.line();
        assert!(
            line.contains(&format!("cost {}", road.cost_name())),
            "{line}"
        );
        assert!(
            line.contains(&format!("last-verified {}", road.last_verified)),
            "{line}"
        );
    }

    // Switchable off and on via registry API.
    let mut reg = default_registry();
    assert!(reg.set_enabled(ThirdPartyKind::Irc, true));
    assert!(reg.get(ThirdPartyKind::Irc).unwrap().enabled);
    assert!(reg.set_enabled(ThirdPartyKind::Irc, false));
    assert!(!reg.get(ThirdPartyKind::Irc).unwrap().enabled);

    // CLI text output includes all five third-party roads with cost and verified date.
    let text_out = std::process::Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(["link", "carriers", "--third-party"])
        .env_remove("RUST_LOG")
        .output()
        .unwrap();
    assert_eq!(text_out.status.code(), Some(0), "{text_out:?}");
    let text = String::from_utf8(text_out.stdout).unwrap();
    for name in expected {
        let line = text
            .lines()
            .find(|l| l.starts_with(&format!("{name}: ")))
            .unwrap_or_else(|| panic!("missing carrier text line for {name}\n{text}"));
        assert!(line.contains("cost "), "{line}");
        assert!(line.contains("last-verified "), "{line}");
    }

    // CLI JSON output includes stamped third_party_roads with cost and last_verified.
    let json_out = std::process::Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(["link", "carriers", "--third-party", "--json"])
        .env_remove("RUST_LOG")
        .output()
        .unwrap();
    assert_eq!(json_out.status.code(), Some(0), "{json_out:?}");
    let doc: serde_json::Value = serde_json::from_slice(&json_out.stdout).unwrap();
    assert_eq!(doc["schema"], 1);
    let tp_rows = doc["third_party_roads"].as_array().unwrap();
    assert_eq!(tp_rows.len(), expected.len());
    for (row, name) in tp_rows.iter().zip(expected) {
        assert_eq!(row["name"], name);
        assert_eq!(row["enabled"], false);
        assert_eq!(row["default"], false);
        assert!(row["cost"].as_str().is_some(), "{row}");
        let verified = row["last_verified"].as_str().unwrap();
        assert!(VerifiedDate::parse(verified).is_ok(), "{verified}");
    }
}

/// Spawn a minimal loopback IRC server that handles NICK, USER, JOIN, PING, and PRIVMSG fanout.
async fn spawn_irc_standin() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, _) = broadcast::channel::<(String, String)>(128);

    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            let tx = tx.clone();
            let mut rx = tx.subscribe();
            tokio::spawn(async move {
                let (read_half, mut write_half) = socket.into_split();
                let mut reader = BufReader::new(read_half);
                let mut nick = String::from("anon");
                let mut line = String::new();
                let mut joined = false;

                // Send a server PING first to prove client PONG handling.
                let _ = write_half.write_all(b"PING :loopback-probe\r\n").await;
                let _ = write_half.flush().await;

                loop {
                    tokio::select! {
                        res = reader.read_line(&mut line) => {
                            match res {
                                Ok(0) | Err(_) => break,
                                Ok(_) => {
                                    let trimmed = line.trim_end_matches(['\r', '\n']).to_string();
                                    line.clear();
                                    if let Some(n) = trimmed.strip_prefix("NICK ") {
                                        nick = n.trim().to_string();
                                    } else if let Some(ch) = trimmed.strip_prefix("JOIN ") {
                                        let ch = ch.trim();
                                        let reply = format!(":standin 001 {nick} :welcome\r\n:{nick}!u@h JOIN {ch}\r\n");
                                        if write_half.write_all(reply.as_bytes()).await.is_err() {
                                            break;
                                        }
                                        let _ = write_half.flush().await;
                                        joined = true;
                                    } else if trimmed.starts_with("PRIVMSG ") {
                                        let wire = format!(":{nick}!u@h {trimmed}\r\n");
                                        let _ = tx.send((nick.clone(), wire));
                                    }
                                }
                            }
                        }
                        msg = rx.recv(), if joined => {
                            if let Ok((from, wire)) = msg {
                                if from != nick {
                                    if write_half.write_all(wire.as_bytes()).await.is_err() {
                                        break;
                                    }
                                    let _ = write_half.flush().await;
                                }
                            }
                        }
                    }
                }
            });
        }
    });
    addr
}

async fn exchange_bidirectional<S>(mut a: S, mut b: S, send_a: Vec<u8>, send_b: Vec<u8>)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let want_a = send_b.clone();
    let want_b = send_a.clone();
    let len_a = want_a.len();
    let len_b = want_b.len();
    let task_a = tokio::spawn(async move {
        a.write_all(&send_a).await.unwrap();
        a.flush().await.unwrap();
        let mut got = vec![0u8; len_a];
        a.read_exact(&mut got).await.unwrap();
        got
    });
    let task_b = tokio::spawn(async move {
        b.write_all(&send_b).await.unwrap();
        b.flush().await.unwrap();
        let mut got = vec![0u8; len_b];
        b.read_exact(&mut got).await.unwrap();
        got
    });
    let (got_a, got_b) = tokio::time::timeout(Duration::from_secs(5), async {
        (task_a.await.unwrap(), task_b.await.unwrap())
    })
    .await
    .expect("bidirectional transfer within 5s");
    assert_eq!(got_a, want_a);
    assert_eq!(got_b, want_b);
}

#[tokio::test]
async fn irc_road_bidirectional_transfer_against_loopback_standin() {
    pod_testkit::need_loopback!();
    let addr = spawn_irc_standin().await;
    let url = format!("irc://{addr}/podnet-test");
    let ep_a = IrcEndpoint::parse(&url)
        .unwrap()
        .with_nick("alice")
        .with_stream_id(7);
    let ep_b = IrcEndpoint::parse(&url)
        .unwrap()
        .with_nick("bob")
        .with_stream_id(7);

    let a = IrcStream::connect(&ep_a).await.unwrap();
    let b = IrcStream::connect(&ep_b).await.unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;

    let payload_a: Vec<u8> = (0..1024).map(|i| ((i * 31) & 0xFF) as u8).collect();
    let payload_b: Vec<u8> = (0..1024).map(|i| ((i * 17 + 5) & 0xFF) as u8).collect();
    exchange_bidirectional(a, b, payload_a, payload_b).await;
}

/// Spawn a minimal loopback MQTT 3.1.1 broker handling CONNECT, SUBSCRIBE, PUBLISH, and PINGREQ.
async fn spawn_mqtt_standin() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    type SubMap = Arc<Mutex<HashMap<String, Vec<mpsc::UnboundedSender<Packet>>>>>;
    let subs: SubMap = Arc::new(Mutex::new(HashMap::new()));

    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            let subs = subs.clone();
            tokio::spawn(async move {
                let (mut reader, mut writer) = socket.into_split();
                let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Packet>();
                let write_task = tokio::spawn(async move {
                    while let Some(pkt) = out_rx.recv().await {
                        if write_packet(&mut writer, &pkt).await.is_err() {
                            break;
                        }
                    }
                });
                while let Ok(pkt) = read_packet(&mut reader).await {
                    match pkt {
                        Packet::Connect { .. } => {
                            let _ = out_tx.send(Packet::ConnAck {
                                session_present: false,
                                return_code: 0,
                            });
                        }
                        Packet::Subscribe {
                            packet_id,
                            topic,
                            qos,
                        } => {
                            subs.lock()
                                .await
                                .entry(topic)
                                .or_default()
                                .push(out_tx.clone());
                            let _ = out_tx.send(Packet::SubAck {
                                packet_id,
                                return_code: qos,
                            });
                        }
                        Packet::Publish { topic, payload } => {
                            let targets =
                                subs.lock().await.get(&topic).cloned().unwrap_or_default();
                            for target in targets {
                                let _ = target.send(Packet::Publish {
                                    topic: topic.clone(),
                                    payload: payload.clone(),
                                });
                            }
                        }
                        Packet::PingReq => {
                            let _ = out_tx.send(Packet::PingResp);
                        }
                        Packet::Disconnect => break,
                        _ => {}
                    }
                }
                write_task.abort();
            });
        }
    });
    addr
}

#[tokio::test]
async fn mqtt_road_bidirectional_transfer_against_loopback_standin() {
    pod_testkit::need_loopback!();
    let addr = spawn_mqtt_standin().await;
    let ep_a = MqttEndpoint::parse(&format!("mqtt://{addr}/podnet/session"))
        .unwrap()
        .with_topics("podnet/session/a2b", "podnet/session/b2a")
        .with_client_id("peer-a");
    let ep_b = MqttEndpoint::parse(&format!("mqtt://{addr}/podnet/session"))
        .unwrap()
        .with_topics("podnet/session/b2a", "podnet/session/a2b")
        .with_client_id("peer-b");

    let mut client_a = MqttClient::connect(&ep_a).await.unwrap();
    let mut client_b = MqttClient::connect(&ep_b).await.unwrap();
    client_a.ping().await.unwrap();
    client_b.ping().await.unwrap();

    let payload_a: Vec<u8> = (0..4096).map(|i| ((i * 7 + 3) & 0xFF) as u8).collect();
    let payload_b: Vec<u8> = (0..4096).map(|i| ((i * 13 + 9) & 0xFF) as u8).collect();
    exchange_bidirectional(
        client_a.into_stream(),
        client_b.into_stream(),
        payload_a,
        payload_b,
    )
    .await;
}

/// Spawn a minimal loopback Syncthing Relay Protocol v1 stand-in server over TCP.
async fn spawn_syncthing_standin() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let invite_txs: Arc<Mutex<Vec<mpsc::UnboundedSender<RelayMessage>>>> =
        Arc::new(Mutex::new(Vec::new()));
    let pending: Arc<Mutex<HashMap<Vec<u8>, TcpStream>>> = Arc::new(Mutex::new(HashMap::new()));

    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let invite_txs = invite_txs.clone();
            let pending = pending.clone();
            tokio::spawn(async move {
                let Ok(first) = read_message(&mut socket).await else {
                    return;
                };
                match first {
                    RelayMessage::JoinRelayRequest { .. } => {
                        let _ = write_message(
                            &mut socket,
                            &RelayMessage::Response {
                                code: 0,
                                message: "success".into(),
                            },
                        )
                        .await;
                        let (tx, mut rx) = mpsc::unbounded_channel();
                        invite_txs.lock().await.push(tx);
                        while let Some(inv) = rx.recv().await {
                            if write_message(&mut socket, &inv).await.is_err() {
                                break;
                            }
                        }
                    }
                    RelayMessage::ConnectRequest { id } => {
                        let key = b"syncthing-session-key-0123456789".to_vec();
                        let inv = RelayMessage::SessionInvitation {
                            from: id,
                            key: key.clone(),
                            address: vec![127, 0, 0, 1],
                            port: addr.port(),
                            server_socket: false,
                        };
                        for tx in invite_txs.lock().await.iter() {
                            let _ = tx.send(inv.clone());
                        }
                        let _ = write_message(&mut socket, &inv).await;
                    }
                    RelayMessage::JoinSessionRequest { key } => {
                        let _ = write_message(
                            &mut socket,
                            &RelayMessage::Response {
                                code: 0,
                                message: "success".into(),
                            },
                        )
                        .await;
                        let peer = pending.lock().await.remove(&key);
                        if let Some(mut other) = peer {
                            let _ = tokio::io::copy_bidirectional(&mut socket, &mut other).await;
                        } else {
                            pending.lock().await.insert(key, socket);
                        }
                    }
                    _ => {}
                }
            });
        }
    });
    addr
}

#[tokio::test]
async fn syncthing_relay_road_bidirectional_transfer_against_loopback_standin() {
    pod_testkit::need_loopback!();
    let addr = spawn_syncthing_standin().await;
    let url = format!("syncthing://{addr}/?id=TESTDEV-1234567-ABCDEFG");
    let endpoint = SyncthingEndpoint::parse(&url).unwrap();

    // Target peer joins the relay and waits for a SessionInvitation.
    let mut relay_ctrl = TcpStream::connect(endpoint.authority()).await.unwrap();
    join_relay(&mut relay_ctrl, vec!["bep/1.0".into()])
        .await
        .unwrap();

    let target_authority = endpoint.authority();
    let target_task = tokio::spawn(async move {
        let inv = read_message(&mut relay_ctrl).await.unwrap();
        let key = match inv {
            RelayMessage::SessionInvitation { key, .. } => key,
            other => panic!("expected SessionInvitation, got {other:?}"),
        };
        let data = TcpStream::connect(&target_authority).await.unwrap();
        let mut session = join_session(data, &key).await.unwrap();
        let mut buf = vec![0u8; 2048];
        session.read_exact(&mut buf).await.unwrap();
        let reply: Vec<u8> = buf.iter().map(|b| b ^ 0xA5).collect();
        session.write_all(&reply).await.unwrap();
        session.flush().await.unwrap();
        buf
    });

    // Initiator dials via ConnectRequest + JoinSessionRequest and exchanges bytes.
    let mut client = dial_tcp(&endpoint).await.unwrap();
    let payload: Vec<u8> = (0..2048).map(|i| ((i * 29 + 1) & 0xFF) as u8).collect();
    client.write_all(&payload).await.unwrap();
    client.flush().await.unwrap();

    let mut reply = vec![0u8; payload.len()];
    tokio::time::timeout(Duration::from_secs(5), client.read_exact(&mut reply))
        .await
        .expect("Syncthing relay transfer within 5s")
        .unwrap();
    let received_by_target = target_task.await.unwrap();
    assert_eq!(received_by_target, payload);
    let expected_reply: Vec<u8> = payload.iter().map(|b| b ^ 0xA5).collect();
    assert_eq!(reply, expected_reply);
}

#[tokio::test]
async fn syncthing_relay_road_supports_tls_transport() {
    pod_testkit::need_loopback!();
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let cert_der = cert.cert.der().clone();
    let key_der = rustls::pki_types::PrivateKeyDer::Pkcs8(cert.signing_key.serialize_der().into());

    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let server_cfg = rustls::ServerConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der.clone()], key_der)
        .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server_cfg));

    let mut roots = rustls::RootCertStore::empty();
    roots.add(cert_der).unwrap();
    let client_cfg = Arc::new(
        rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        // First connection: ConnectRequest -> SessionInvitation.
        let (tcp1, _) = listener.accept().await.unwrap();
        let mut s1 = acceptor.accept(tcp1).await.unwrap();
        let req = read_message(&mut s1).await.unwrap();
        assert!(matches!(req, RelayMessage::ConnectRequest { .. }));
        write_message(
            &mut s1,
            &RelayMessage::SessionInvitation {
                from: b"tls-peer".to_vec(),
                key: b"tls-key-01234567".to_vec(),
                address: vec![127, 0, 0, 1],
                port: addr.port(),
                server_socket: false,
            },
        )
        .await
        .unwrap();
        // Second connection: JoinSessionRequest -> Response(0) -> echo bytes.
        let (tcp2, _) = listener.accept().await.unwrap();
        let mut s2 = acceptor.accept(tcp2).await.unwrap();
        let join = read_message(&mut s2).await.unwrap();
        assert!(matches!(join, RelayMessage::JoinSessionRequest { .. }));
        write_message(
            &mut s2,
            &RelayMessage::Response {
                code: 0,
                message: "success".into(),
            },
        )
        .await
        .unwrap();
        let mut buf = [0u8; 16];
        s2.read_exact(&mut buf).await.unwrap();
        s2.write_all(&buf).await.unwrap();
        s2.flush().await.unwrap();
    });

    let ep = SyncthingEndpoint::parse(&format!("syncthing+tls://{addr}/?id=TLS-DEV")).unwrap();
    assert!(ep.tls);
    let server_name = rustls::pki_types::ServerName::try_from("localhost").unwrap();
    let mut tls_stream = dial_tls(&ep, client_cfg, server_name).await.unwrap();
    tls_stream.write_all(b"0123456789abcdef").await.unwrap();
    tls_stream.flush().await.unwrap();
    let mut echoed = [0u8; 16];
    tls_stream.read_exact(&mut echoed).await.unwrap();
    assert_eq!(&echoed, b"0123456789abcdef");
}

#[tokio::test]
async fn broken_endpoint_degrades_road_names_it_and_keeps_session_running() {
    pod_testkit::need_loopback!();

    // Start an active bidirectional TCP session to prove watch failures do not abort it.
    let echo_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let echo_addr = echo_listener.local_addr().unwrap();
    tokio::spawn(async move {
        if let Ok((mut s, _)) = echo_listener.accept().await {
            let mut buf = [0u8; 64];
            while let Ok(n) = s.read(&mut buf).await {
                if n == 0 {
                    break;
                }
                if s.write_all(&buf[..n]).await.is_err() {
                    break;
                }
            }
        }
    });
    let mut active_session = TcpStream::connect(echo_addr).await.unwrap();
    active_session.write_all(b"before").await.unwrap();
    let mut reply = [0u8; 6];
    active_session.read_exact(&mut reply).await.unwrap();
    assert_eq!(&reply, b"before");

    // Obtain a closed loopback port for a deliberately broken third-party road endpoint.
    let dead_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dead_addr = dead_listener.local_addr().unwrap();
    drop(dead_listener);

    let today = VerifiedDate::new(2026, 11, 27).unwrap();
    let mut road = ThirdPartyRoad::new(ThirdPartyKind::Mqtt, pod_link::Cost::Medium, today, 30)
        .with_endpoint(format!("mqtt://{dead_addr}/broken"));

    let state = probe_road(&mut road, Duration::from_millis(500), today).await;
    match &state {
        WatchState::Degraded { reason } => {
            assert!(
                reason.contains("mqtt"),
                "degraded reason must name road: {reason}"
            );
        }
        other => panic!("expected WatchState::Degraded, got {other:?}"),
    }
    assert_eq!(road.state, state);
    assert!(!road.is_working(today));

    // The active session continues running cleanly after road degradation.
    active_session.write_all(b"after!").await.unwrap();
    active_session.read_exact(&mut reply).await.unwrap();
    assert_eq!(&reply, b"after!");
}

#[test]
fn status_reports_oldest_date_flags_stale_and_deduplicates_six_roads_on_one_domain() {
    // 1. Six roads sharing one domain report 1 relay_domain dependency, not 6.
    let mut collector = DependencyCollector::new();
    for url in [
        "irc://a.shared.test:6667/chan",
        "mqtt://b.shared.test:1883/topic",
        "syncthing://c.shared.test:22067/?id=DEV",
        "https://d.shared.test:443/pipe",
        "wss://e.shared.test:443/ws",
        "tcp://shared.test:4001",
    ] {
        collector.add_road_url(url);
    }
    collector.add(DependencyClass::CertificateAuthority, "system-roots");
    collector.add(DependencyClass::CertificateAuthority, "system-roots");
    collector.add(DependencyClass::DnsResolver, "127.0.0.53");

    let today = VerifiedDate::new(2026, 11, 27).unwrap();
    let fresh_roads = vec![
        ThirdPartyRoad::new(
            ThirdPartyKind::Irc,
            pod_link::Cost::High,
            VerifiedDate::new(2026, 11, 20).unwrap(),
            30,
        ),
        ThirdPartyRoad::new(
            ThirdPartyKind::Mqtt,
            pod_link::Cost::Medium,
            VerifiedDate::new(2026, 11, 10).unwrap(),
            30,
        ),
    ];

    let summary = collector.summarize(&fresh_roads, today);
    assert_eq!(summary.counts.relay_domain, 1, "6 roads on 1 domain => 1");
    assert_eq!(summary.counts.relay_host, 6);
    assert_eq!(summary.counts.carrier_protocol, 6);
    assert_eq!(summary.counts.carrier_destination, 6);
    assert_eq!(summary.counts.certificate_authority, 1);
    assert_eq!(summary.counts.dns_resolver, 1);
    assert_eq!(summary.oldest_verified.as_deref(), Some("2026-11-10"));
    assert!(!summary.stale);
    assert!(summary.ok());
    assert!(summary.check().is_ok());

    // 2. A stale road entry sets oldest_verified, marks stale=true, and fails check().
    let stale_roads = vec![
        ThirdPartyRoad::new(
            ThirdPartyKind::Irc,
            pod_link::Cost::High,
            VerifiedDate::new(2026, 11, 25).unwrap(),
            30,
        ),
        ThirdPartyRoad::new(
            ThirdPartyKind::Piping,
            pod_link::Cost::Medium,
            VerifiedDate::new(2026, 9, 1).unwrap(),
            30,
        ),
    ];
    let stale_summary = collector.summarize(&stale_roads, today);
    assert_eq!(stale_summary.oldest_verified.as_deref(), Some("2026-09-01"));
    assert!(stale_summary.stale);
    assert!(!stale_summary.ok());
    assert!(stale_summary
        .stale_roads
        .iter()
        .any(|s| s.contains("piping")));
    let err = stale_summary.check().unwrap_err().to_string();
    assert!(
        err.contains("2026-09-01") && err.contains("piping"),
        "{err}"
    );
    let text = stale_summary.text();
    assert!(text.contains("oldest_verified: 2026-09-01"), "{text}");
    assert!(text.contains("relay_domain=1"), "{text}");
    assert!(text.contains("stale"), "{text}");

    // 3. `status::document()` preserves existing fields and adds `dependencies`.
    let doc = status::document();
    assert_eq!(doc["schema"], 1);
    let deps = &doc["dependencies"];
    assert!(deps.is_object(), "{doc}");
    for key in [
        "relay_host",
        "relay_domain",
        "certificate_authority",
        "carrier_protocol",
        "carrier_destination",
        "dns_resolver",
        "third_party_service",
        "total",
    ] {
        assert!(deps["counts"][key].is_u64(), "missing counts.{key}: {deps}");
    }
    assert!(deps["oldest_verified"].is_string(), "{deps}");
    assert!(deps["stale"].is_boolean(), "{deps}");
}

#[test]
fn default_selection_never_picks_fallback_adapter() {
    // Verify all four adapters are registered in RoadClass::FallbackAdapter.
    assert_eq!(ADAPTER_REGISTRY.len(), 4);
    for name in ["tor", "iroh", "zerotier", "tailscale"] {
        assert_eq!(classify_road(name), RoadClass::FallbackAdapter);
        assert!(
            ADAPTER_REGISTRY
                .iter()
                .any(|e| e.name == name && !e.description.is_empty()),
            "missing adapter metadata for {name}"
        );
    }

    // Even when fallback adapters have priority 0, default selection skips them.
    let mixed = vec![
        RoadCandidate::new("tor", 0, true),
        RoadCandidate::new("iroh", 1, true),
        RoadCandidate::new("zerotier", 2, true),
        RoadCandidate::new("tailscale", 3, true),
        RoadCandidate::new("irc", 4, true),
        RoadCandidate::new("relay", 10, true),
    ];
    let picked = select_road(&mixed, &SelectionPolicy::default()).unwrap();
    assert_eq!(picked.name, "relay");
    assert_ne!(picked.class, RoadClass::FallbackAdapter);

    // When ONLY fallback adapters are available, default selection returns None.
    let adapters_only = vec![
        RoadCandidate::new("tor", 0, true),
        RoadCandidate::new("iroh", 1, true),
        RoadCandidate::new("zerotier", 2, true),
        RoadCandidate::new("tailscale", 3, true),
    ];
    assert_eq!(
        select_road(&adapters_only, &SelectionPolicy::default()),
        None
    );

    // An explicit flag or policy rule naming an adapter permits selecting it.
    let explicit = SelectionPolicy {
        explicit_adapter: Some("tailscale".into()),
        ..Default::default()
    };
    let chosen = select_road(&adapters_only, &explicit).unwrap();
    assert_eq!(chosen.name, "tailscale");

    let by_rule = SelectionPolicy {
        allowed_adapters: vec!["iroh".into()],
        ..Default::default()
    };
    let chosen_rule = select_road(&adapters_only, &by_rule).unwrap();
    assert_eq!(chosen_rule.name, "iroh");
}

#[tokio::test]
async fn dead_front_refuses_naming_front_within_one_second() {
    pod_testkit::need_loopback!();
    assert_eq!(
        parse_front_param("wss://origin.test/ws?front=front.test:443"),
        Some("front.test:443".into())
    );

    // Case 1: Refused port.
    let closed = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let closed_addr = closed.local_addr().unwrap();
    drop(closed);

    // Case 2: HTTP 404 from a stand-in front that no longer routes the target.
    let bad_status_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let bad_status_addr = bad_status_listener.local_addr().unwrap();
    tokio::spawn(async move {
        if let Ok((mut s, _)) = bad_status_listener.accept().await {
            let mut buf = [0u8; 512];
            let _ = s.read(&mut buf).await;
            let _ = s
                .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n")
                .await;
        }
    });

    // Case 3: Stalled front that accepts TCP and never sends a status line (timeout clamped <= 1s).
    let stall_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let stall_addr = stall_listener.local_addr().unwrap();
    tokio::spawn(async move {
        if let Ok((_s, _)) = stall_listener.accept().await {
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
    });

    // Case 4: Healthy front returning HTTP 200 OK.
    let ok_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ok_addr = ok_listener.local_addr().unwrap();
    tokio::spawn(async move {
        if let Ok((mut s, _)) = ok_listener.accept().await {
            let mut buf = [0u8; 512];
            let _ = s.read(&mut buf).await;
            let _ = s
                .write_all(b"HTTP/1.1 200 OK\r\nX-Front-Routed-Host: target.test\r\n\r\n")
                .await;
        }
    });

    for (label, front_addr, requested_timeout) in [
        ("refused", closed_addr.to_string(), Duration::from_secs(1)),
        (
            "http-404",
            bad_status_addr.to_string(),
            Duration::from_secs(1),
        ),
        ("stalled", stall_addr.to_string(), Duration::from_secs(30)),
    ] {
        let start = Instant::now();
        let err = check_front(&front_addr, "target.test", requested_timeout)
            .await
            .expect_err(label);
        let elapsed = start.elapsed();
        assert!(
            elapsed <= Duration::from_millis(1200),
            "{label}: took {elapsed:?}, must refuse within ~1s"
        );
        let msg = err.to_string();
        assert!(
            msg.contains(&front_addr),
            "{label}: refusal must name dead front {front_addr}: {msg}"
        );
    }

    assert!(
        check_front(&ok_addr.to_string(), "target.test", Duration::from_secs(1))
            .await
            .is_ok()
    );
}
