use super::*;

fn scheme_of(spec: &str) -> &'static str {
    parse(spec)
        .unwrap_or_else(|e| panic!("{spec}: {e}"))
        .scheme()
}

#[test]
fn network_spec_each_form_of_the_two_old_parsers_is_taken() {
    // The forms of the proxy of the process (egress::Proxy::parse_trusted).
    for spec in [
        "http://proxy.example:3128",
        "http://user:pw@proxy.example",
        "socks5://proxy.example:1080",
        "socks5h://proxy.example",
        "socks5h+unix:/run/proxy.sock",
        "http://unix:/run/proxy.sock",
        "http+exec:ssh -W %h:%p jump",
        "socks5+unix:///run/proxy.sock",
    ] {
        let network = parse(spec).unwrap();
        assert!(
            matches!(network.kind, Kind::Proxy(_)),
            "{spec}: {network:?}"
        );
        assert_eq!(network.scheme(), "proxy");
    }
    // The forms that only the hop of the pod takes (`pod -x`).
    for spec in [
        "unix:/run/proxy.sock",
        "proxy.example:1080",
        "socks://proxy.example:1080",
        "https://proxy.example:3128",
    ] {
        let network = parse(spec).unwrap();
        assert!(
            matches!(network.kind, Kind::Hop { tailscale: false }),
            "{spec}: {network:?}"
        );
        assert_eq!(network.scheme(), "proxy");
    }
    assert_eq!(scheme_of("direct"), "direct");
    assert_eq!(
        scheme_of("tailscale:/run/tailscale/tailscaled.sock"),
        "tailscale"
    );
    assert_eq!(scheme_of("relay"), "relay");
    assert_eq!(scheme_of("relay:relay.example"), "relay");
    assert_eq!(scheme_of("relay:http://127.0.0.1:8787"), "relay");
}

#[test]
fn network_spec_a_bad_spec_is_one_usage_line_with_no_password() {
    for spec in [
        "",
        "   ",
        "tailscale:",
        "unix:",
        "relay:ftp://relay.example",
        "relay:https://relay.example/path",
        "gopher://user:secret@proxy.example:70",
    ] {
        let err = parse(spec)
            .err()
            .unwrap_or_else(|| panic!("{spec:?} parsed"));
        assert_eq!(err.class, Class::Usage, "{spec:?}: {err}");
        assert!(!err.text().contains('\n'), "{err}");
        assert!(!err.text().contains("secret"), "{err}");
    }
    let err = parse("gopher://user:secret@proxy.example:70")
        .err()
        .unwrap();
    assert!(
        err.text().starts_with("gopher://***@proxy.example:70: "),
        "{err}"
    );
}

#[test]
fn network_spec_warp_is_named_and_not_in_the_registry_yet() {
    for spec in ["warp", "warp:identity=/tmp/id.json"] {
        let err = parse(spec).expect_err("no warp yet");
        assert_eq!(err.class, Class::Unsupported);
        assert!(err.text().contains("podnet warp proxy"), "{err}");
    }
    assert!(networks().iter().all(|r| r.scheme != "warp"));
}

#[test]
fn network_spec_the_label_has_no_credentials() {
    assert_eq!(
        parse("socks5h://u:pw@proxy.example:1080").unwrap().label(),
        "socks5h://proxy.example:1080"
    );
    assert_eq!(
        parse("u:pw@proxy.example:1080").unwrap().label(),
        "***@proxy.example:1080"
    );
    assert_eq!(
        parse("https://u:pw@proxy.example:3128").unwrap().label(),
        "https://***@proxy.example:3128"
    );
}

#[test]
fn network_spec_the_table_has_one_row_for_each_scheme() {
    let schemes: Vec<&str> = networks().iter().map(|r| r.scheme).collect();
    assert_eq!(
        schemes,
        [
            "direct",
            "proxy",
            "relay",
            "tailscale",
            "netbird",
            "wg",
            "turn",
            "devtunnel",
            "peer"
        ]
    );
    for spec in ["direct", "http://p.example:3128", "relay", "tailscale:/s"] {
        let network = parse(spec).unwrap();
        assert_eq!(network.row().scheme, network.scheme());
        assert_eq!(network.caps(), network.row().caps);
    }
    assert!(parse("direct").unwrap().caps().contains(Caps::EGRESS_UDP));
    assert!(!parse("relay").unwrap().caps().contains(Caps::EGRESS_UDP));
}

#[test]
fn network_spec_the_checks_name_the_proxy_and_the_relay() {
    let checks = parse("http://u:pw@proxy.example:3128").unwrap().checks();
    assert_eq!(
        checks,
        [Check {
            name: "proxy".into(),
            target: Target::new("proxy.example", 3128),
            tls: false,
        }]
    );
    let checks = parse("relay:relay.example").unwrap().checks();
    assert_eq!(
        checks,
        [Check {
            name: "relay".into(),
            target: Target::new("relay.example", 443),
            tls: true,
        }]
    );
    let checks = parse("relay:http://127.0.0.1:8787").unwrap().checks();
    assert_eq!(checks[0].target, Target::new("127.0.0.1", 8787));
    assert!(!checks[0].tls);
    assert!(parse("socks5h+unix:/run/p.sock")
        .unwrap()
        .checks()
        .is_empty());
    assert!(parse("direct").unwrap().checks().is_empty());
}

#[tokio::test]
async fn network_spec_up_gives_a_way_and_refuses_a_way_below() {
    let direct = parse("direct").unwrap();
    let link = direct.up(&Ctx::default()).await.unwrap();
    assert_eq!(link.way.as_ref().unwrap().label(), "direct");
    assert_eq!(
        link.status,
        serde_json::json!({"scheme": "direct", "spec": "direct"})
    );
    let hop = parse("unix:/run/proxy.sock").unwrap();
    let below = Ctx {
        below: Some(link.way.as_ref().unwrap().dial.clone()),
        ..Ctx::default()
    };
    let err = hop.up(&below).await.expect_err("no way below yet");
    assert_eq!(
        format!("{err:#}"),
        "unix:/run/proxy.sock cannot go over another way (direct) yet"
    );
    let link = hop.up(&Ctx::default()).await.unwrap();
    assert!(link.way.is_some());
}

/// A fake SOCKS5 listener on the loopback: no authentication, one CONNECT
/// to an IPv4 address, then an echo. It sends the address it was asked for
/// on the channel.
async fn fake_socks5() -> (u16, tokio::sync::mpsc::UnboundedReceiver<[u8; 6]>) {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Ok((mut io, _)) = listener.accept().await {
            let tx = tx.clone();
            tokio::spawn(async move {
                let mut greeting = [0u8; 3];
                io.read_exact(&mut greeting).await.unwrap();
                io.write_all(&[5, 0]).await.unwrap();
                let mut request = [0u8; 10];
                if io.read_exact(&mut request).await.is_err() {
                    return;
                }
                let _ = tx.send(request[..6].try_into().unwrap());
                io.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).await.unwrap();
                let mut buf = [0u8; 64];
                while let Ok(n) = io.read(&mut buf).await {
                    if n == 0 || io.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                }
            });
        }
    });
    (port, rx)
}

#[test]
fn network_spec_netbird_takes_a_port_and_has_a_row() {
    for (spec, label) in [
        ("netbird", "netbird:1080"),
        ("netbird:", "netbird:1080"),
        ("netbird:1081", "netbird:1081"),
    ] {
        let network = parse(spec).unwrap_or_else(|e| panic!("{spec}: {e}"));
        assert_eq!(network.scheme(), "netbird", "{spec}");
        assert_eq!(network.label(), label);
        assert_eq!(network.caps(), Caps::EGRESS_TCP);
        assert!(!network.is_process_proxy());
    }
    let checks = parse("netbird:1081").unwrap().checks();
    assert_eq!(checks[0].target, Target::new("127.0.0.1", 1081));
    for spec in ["netbird:0", "netbird:x", "netbird:70000", "netbird:1:2"] {
        let err = parse(spec).err().unwrap_or_else(|| panic!("{spec} parsed"));
        assert_eq!(err.class, Class::Usage, "{spec}: {err}");
        assert!(!err.text().contains('\n'), "{err}");
    }
}

#[tokio::test]
async fn network_spec_netbird_dials_an_address_through_the_listener() {
    crate::test_host::need_loopback!();
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let (port, mut asked) = fake_socks5().await;
    let network = parse(&format!("netbird:{port}")).unwrap();
    let link = network.up(&Ctx::default()).await.unwrap();
    let way = link.way.as_ref().unwrap();
    assert_eq!(way.label(), format!("netbird:{port}"));
    let target = Target::Addr("100.64.0.7:8080".parse().unwrap());
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut conn = way.dial.dial(&target, deadline).await.unwrap();
    assert_eq!(asked.recv().await.unwrap(), [5, 1, 0, 1, 100, 64]);
    conn.io.write_all(b"ping").await.unwrap();
    let mut back = [0u8; 4];
    conn.io.read_exact(&mut back).await.unwrap();
    assert_eq!(&back, b"ping");
}

#[tokio::test]
async fn netbird_refuses_a_name_with_one_line_and_no_dial() {
    crate::test_host::need_loopback!();
    let (port, mut asked) = fake_socks5().await;
    let network = parse(&format!("netbird:{port}")).unwrap();
    let link = network.up(&Ctx::default()).await.unwrap();
    let dial = link.way.as_ref().unwrap().dial.clone();
    let target = Target::Name {
        host: "peer.netbird.cloud".into(),
        port: 22,
    };
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    let err = dial.dial(&target, deadline).await.expect_err("no DNS");
    assert_eq!(err.class, Class::Unsupported);
    assert!(!err.text().contains('\n'), "{err}");
    assert!(err.text().contains("no DNS"), "{err}");
    assert!(err.text().contains("IP address"), "{err}");
    assert!(asked.try_recv().is_err(), "the listener got a request");
}

#[tokio::test]
async fn network_spec_netbird_doctor_greeting_answers_or_names_the_fault() {
    crate::test_host::need_loopback!();
    let (port, _asked) = fake_socks5().await;
    let line = netbird_greeting(port).await.unwrap();
    assert!(line.contains("answers the SOCKS5 greeting"), "{line}");
    // A port with no listener.
    let free = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let why = netbird_greeting(free).await.unwrap_err();
    assert!(why.contains("does not accept"), "{why}");
}
