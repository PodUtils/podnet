use super::clock_check::clock_of;
use super::*;

// Issue 180: a refused direct dial is an answer, not unknown.
#[tokio::test]
async fn a_direct_refusal_is_an_answer() {
    crate::test_host::need_loopback!();
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let addr: std::net::SocketAddr = ([127, 0, 0, 1], port).into();
    let err = crate::egress::dial_addrs(&[addr], Duration::from_secs(5))
        .await
        .unwrap_err();
    match classify(&err, Duration::from_millis(1)) {
        Sample::Refused(_, why) => assert!(why.contains("connection refused"), "{why}"),
        other => panic!("not a refusal: {:?}", std::mem::discriminant(&other)),
    }
}

// Issue 128: the Date of an answer gives the skew of the clock.
#[test]
fn the_date_header_gives_the_skew() {
    assert_eq!(
        parse_imf_fixdate("Sun, 06 Nov 1994 08:49:37 GMT"),
        Some(784111777)
    );
    let now = std::time::SystemTime::now();
    let unix = now.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
    assert_eq!(
        parse_imf_fixdate(&pod_core::clock::http_date(unix)),
        Some(unix)
    );
    assert_eq!(parse_imf_fixdate("yesterday"), None);
    assert_eq!(clock_of(1000, 990, "api").verdict, Verdict::Ok);
    let off = clock_of(4600, 1000, "api");
    assert_eq!((off.verdict, off.skew), (Verdict::Refused, Some(3600)));
    assert!(off.detail.contains("set the clock"));
}

fn path(id: &'static str, host: &str, port: u16, ok: bool) -> Path {
    Path {
        target: Target {
            id,
            host: host.into(),
            port,
            tls: Tls::Web,
        },
        via: None,
        verdict: if ok { Verdict::Ok } else { Verdict::Unknown },
        detail: if ok {
            "open".into()
        } else {
            "Network is unreachable".into()
        },
        sent: 1,
        answered: ok as usize,
        best: None,
        median: None,
        p95: None,
        jitter: None,
        tls: ok.then(|| TlsResult {
            verdict: Verdict::Ok,
            issuer: None,
            detail: "ok".into(),
        }),
        dns: None,
    }
}

// The IPv6-only kit: with no IPv4 route, the front door of IPv6 makes
// the leg quic+warp work.
#[test]
fn the_ipv6_front_door_makes_quic_warp_work() {
    let udp = Egress::Unknown("x".into());
    let paths = vec![
        path("masque", masque_address(), 443, false),
        path("masque-v6", masque_address_v6(), 443, true),
        path("warp-api", "api.cloudflareclient.com", 443, true),
    ];
    let warp = legs(&paths, &udp)
        .into_iter()
        .find(|l| l.name == "quic+warp")
        .unwrap();
    assert_eq!(warp.verdict, Verdict::Ok, "{}", warp.reason);
    assert!(warp.reason.contains("2606:4700:103::2"), "{}", warp.reason);
    let paths = vec![
        path("masque", masque_address(), 443, false),
        path("warp-api", "api.cloudflareclient.com", 443, true),
    ];
    let warp = legs(&paths, &udp)
        .into_iter()
        .find(|l| l.name == "quic+warp")
        .unwrap();
    assert_ne!(warp.verdict, Verdict::Ok);
}

// Issue 128: a road works when one of its legs works.
#[test]
fn the_roads_follow_the_legs() {
    let leg = |name, verdict| Leg {
        name,
        verdict,
        reason: "r".into(),
    };
    let report = Report {
        proxy: Value::Null,
        paths: Vec::new(),
        udp: ("1.2.3.4:7844".parse().unwrap(), Egress::Unknown("x".into())),
        names: None,
        legs: vec![
            leg("quic", Verdict::Refused),
            leg("http2", Verdict::Ok),
            leg("quic+warp", Verdict::Refused),
            leg("http2+relay", Verdict::Unknown),
        ],
        clock: Clock {
            verdict: Verdict::Unknown,
            skew: None,
            detail: String::new(),
        },
        git: git_check::git("git"),
        quick: false,
    };
    let roads = report.roads();
    assert_eq!(roads[0].0, "direct");
    assert_eq!(roads[0].1, Verdict::Ok);
    assert!(roads[0].2.starts_with("http2:"));
    assert_eq!((roads[1].0, roads[1].1), ("warp", Verdict::Refused));
    assert_eq!((roads[2].0, roads[2].1), ("relay", Verdict::Unknown));
    let json = report.to_json();
    assert_eq!(json["roads"][2]["road"], "relay");
    assert!(report.text().contains("roads\n  direct"));

    // E5: the last line names the step to take.
    assert!(report.text().ends_with(
        "next: podnet tunnel --url http://localhost:8080 (the default legs work here)\n"
    ));
    assert_eq!(json["next"], report.next());
    let mut relay_only = report.clone();
    relay_only.legs = vec![
        leg("http2", Verdict::Refused),
        leg("http2+relay", Verdict::Ok),
    ];
    assert!(relay_only.next().contains("--transport relay"));
    let mut none = report.clone();
    none.legs = vec![leg("http2", Verdict::Refused)];
    assert!(none.next().contains("podnet bug-report --doctor"));
    let mut clock = report.clone();
    clock.clock = clock_of(1000, 100, "api.example");
    assert!(clock.next().contains("set the clock"), "{}", clock.next());
}

#[test]
fn the_statistics() {
    let ms = Duration::from_millis;
    let (best, median, p95, jitter) = stats(&[ms(10), ms(30), ms(20), ms(20)]);
    assert_eq!(best, Some(ms(10)));
    assert_eq!(median, Some(ms(20)));
    assert_eq!(p95, Some(ms(30)));
    // |30-10| + |20-30| + |20-20| = 30, over 3.
    assert_eq!(jitter, Some(ms(10)));
    assert_eq!(stats(&[]).0, None);
}

#[test]
fn a_proxy_refusal_is_an_answer() {
    let err = anyhow::Error::new(crate::egress::Refusal {
        status: 403,
        text: "the proxy refused CONNECT with HTTP 403".into(),
    })
    .context("x through the proxy");
    assert!(
        matches!(classify(&err, Duration::ZERO), Sample::Refused(_, t) if t.starts_with("proxy 403"))
    );
    let lost = anyhow::anyhow!("connection reset by peer");
    assert!(matches!(classify(&lost, Duration::ZERO), Sample::Lost(_)));
}

/// T-094: the doctor reports a missing git, and the version of git.
#[test]
fn doctor_reports_a_missing_git() {
    let missing = git_check::git("podnet-no-such-git");
    assert_eq!(missing.verdict, Verdict::Refused);
    assert!(missing.detail.contains("not on PATH"), "{}", missing.detail);
    assert!(
        missing.line().contains("git    fail:"),
        "{}",
        missing.line()
    );
    assert_eq!(missing.to_json()["status"], "fail");
    let found = git_check::git("git");
    if found.verdict == Verdict::Ok {
        assert!(found.version.as_deref().unwrap().starts_with("git version"));
    }
}

/// T-216: the keys of the `dns` object of a path are fixed in schema 1.
#[test]
fn doctor_json_dns_keys_are_fixed() {
    let keys = |v: &Value| {
        let mut k: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
        k.sort();
        k
    };
    let mut ok = path("api", "api.example", 443, true);
    ok.dns = Some(Ok((
        vec!["192.0.2.1".parse().unwrap()],
        Duration::from_millis(3),
    )));
    let mut failed = path("warp-api", "warp.example", 443, false);
    failed.dns = Some(Err("no such name".into()));
    let report = Report {
        proxy: Value::Null,
        paths: vec![ok, failed],
        udp: ("1.2.3.4:7844".parse().unwrap(), Egress::Unknown("x".into())),
        names: None,
        legs: Vec::new(),
        clock: Clock {
            verdict: Verdict::Unknown,
            skew: None,
            detail: String::new(),
        },
        git: git_check::git("git"),
        quick: false,
    };
    let json = report.to_json();
    assert_eq!(keys(&json["paths"][0]["dns"]), ["addresses", "ms"]);
    assert_eq!(keys(&json["paths"][1]["dns"]), ["error"]);
}

/// T-217: the lanes follow the legs and the paths.
#[test]
fn the_lanes_follow_the_legs_and_paths() {
    let mut edge = path("edge-address", "192.0.2.1", 7844, true);
    edge.best = Some(Duration::from_millis(12));
    let mut api = path("api", "api.example", 443, true);
    api.via = Some("http://proxy.example:3128".into());
    api.best = Some(Duration::from_millis(40));
    let leg = |name, verdict| Leg {
        name,
        verdict,
        reason: "r".into(),
    };
    let mut report = Report {
        proxy: Value::Null,
        paths: vec![edge, path("masque", masque_address(), 443, false)],
        udp: ("1.2.3.4:7844".parse().unwrap(), Egress::Unknown("x".into())),
        names: None,
        legs: vec![
            leg("quic", Verdict::Refused),
            leg("http2", Verdict::Ok),
            leg("quic+warp", Verdict::Unknown),
        ],
        clock: Clock {
            verdict: Verdict::Unknown,
            skew: None,
            detail: String::new(),
        },
        git: git_check::git("git"),
        quick: false,
    };
    let lane =
        |report: &Report, name: &str| lanes(report).into_iter().find(|l| l.name == name).unwrap();
    let names: Vec<&str> = lanes(&report).iter().map(|l| l.name).collect();
    assert_eq!(names, LANES);
    let http2 = lane(&report, "tunnel_http2");
    assert_eq!(
        (http2.verdict, http2.time),
        (Verdict::Ok, Some(Duration::from_millis(12)))
    );
    assert_eq!(lane(&report, "tunnel_quic").verdict, Verdict::Refused);
    assert_eq!(lane(&report, "masque_tcp443").verdict, Verdict::Unknown);
    assert_eq!(lane(&report, "proxy").reason, "no proxy is set");
    report.paths.push(api);
    let proxy = lane(&report, "proxy");
    assert_eq!(
        (proxy.verdict, proxy.time),
        (Verdict::Ok, Some(Duration::from_millis(40)))
    );
    assert_eq!(report.to_json()["lanes"]["proxy"]["ms"], 40.0);
    assert_eq!(report.to_json()["lanes"]["tunnel_quic"]["verdict"], "fail");
}
