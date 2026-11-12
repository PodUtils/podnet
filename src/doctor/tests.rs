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
        ssh_relay: SshRelay::none(),
        quick: false,
        probes: Probes::untested(),
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
        ssh_relay: SshRelay::none(),
        quick: false,
        probes: Probes::untested(),
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
        ssh_relay: SshRelay::none(),
        quick: false,
        probes: Probes::untested(),
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

fn ssh_target() -> SshRelayTarget {
    SshRelayTarget {
        address: "relay.example:443".into(),
        user: "tester".into(),
        proxy: None,
        refusal: None,
        source: "flag".into(),
    }
}

// T-165: no relay is given, so the row is unknown with a reason.
#[test]
fn the_ssh_relay_row_is_unknown_without_a_relay() {
    let row = SshRelay::none();
    assert_eq!(row.verdict, Verdict::Unknown);
    assert_eq!(row.detail, "no SSH relay is given");
    let json = row.to_json();
    assert_eq!(json["verdict"], "unknown");
    assert_eq!(json["status"], "unknown");
    assert_eq!(json["relay"], Value::Null);
    assert_eq!(json["auth"], Value::Null);
}

// T-165: a passed probe is ok; a refusal is fail; a stage with no answer
// in time is unknown.
#[test]
fn the_ssh_relay_verdict_follows_the_probe() {
    use crate::relay::probe::{Failure, Probe, Stage};
    let passed = Probe {
        connect: Some(Duration::from_millis(3)),
        banner: Some("SSH-2.0-test".into()),
        auth: Some(true),
        method: Some("publickey"),
        total: Duration::from_millis(20),
        failure: None,
    };
    let (verdict, detail) = ssh_relay_check::judge(&ssh_target(), &passed);
    assert_eq!(verdict, Verdict::Ok);
    assert!(
        detail.contains("SSH-2.0-test") && detail.contains("20"),
        "{detail}"
    );
    let failed = |timed_out| Probe {
        failure: Some(Failure {
            stage: Stage::Auth,
            timed_out,
            reason: "the relay rejected the user".into(),
        }),
        ..Probe::default()
    };
    let (verdict, detail) = ssh_relay_check::judge(&ssh_target(), &failed(false));
    assert_eq!(verdict, Verdict::Refused);
    assert!(detail.contains("auth failed"), "{detail}");
    let (verdict, _) = ssh_relay_check::judge(&ssh_target(), &failed(true));
    assert_eq!(verdict, Verdict::Unknown);
}

// T-165: the relay of a running ssh-relay instance comes from its record.
#[test]
fn the_ssh_relay_target_comes_from_the_record_of_an_instance() {
    let mut record = crate::instances::Record {
        name: "leg".into(),
        command: "ssh-relay".into(),
        args: [
            "--server",
            "r.example:443",
            "--user=bob",
            "--connect-proxy",
            "p:3128",
        ]
        .map(String::from)
        .to_vec(),
        ..Default::default()
    };
    let target = ssh_relay_check::target_of_record(&record).unwrap();
    assert_eq!(target.address, "r.example:443");
    assert_eq!(target.user, "bob");
    assert_eq!(target.proxy.as_deref(), Some("p:3128"));
    assert_eq!(target.source, "instance leg");
    record.command = "tunnel".into();
    assert_eq!(ssh_relay_check::target_of_record(&record), None);
    record.command = "ssh-relay".into();
    record.args.clear();
    assert_eq!(ssh_relay_check::target_of_record(&record), None);
}

/// T-260: the lanes `masque_udp443` and `ws_upgrade` hold their probes.
#[test]
fn the_lanes_hold_the_probes() {
    let mut report = Report {
        proxy: Value::Null,
        paths: Vec::new(),
        udp: ("1.2.3.4:7844".parse().unwrap(), Egress::Unknown("x".into())),
        names: None,
        legs: Vec::new(),
        clock: Clock {
            verdict: Verdict::Unknown,
            skew: None,
            detail: String::new(),
        },
        git: git_check::git("git"),
        ssh_relay: SshRelay::none(),
        quick: false,
        probes: Probes::untested(),
    };
    report.probes.masque_udp = LaneProbe {
        verdict: Verdict::Ok,
        time: Some(Duration::from_millis(7)),
        reason: "a datagram came back".into(),
    };
    report.probes.ws_upgrade = LaneProbe {
        verdict: Verdict::Refused,
        time: None,
        reason: "status 403".into(),
    };
    let json = report.to_json();
    assert_eq!(json["lanes"]["masque_udp443"]["verdict"], "ok");
    assert_eq!(json["lanes"]["masque_udp443"]["ms"], 7.0);
    assert_eq!(json["lanes"]["ws_upgrade"]["verdict"], "fail");
    assert_eq!(json["lanes"]["ws_upgrade"]["reason"], "status 403");
}

fn parsed(spec: &str) -> pod_proxy::Proxy {
    crate::egress::parse(spec).unwrap_or_else(|e| panic!("{spec}: {e}"))
}

// T-322: the probe dials through the flag, else through the proxy that the
// resolver gives for the relay, else directly.
#[test]
fn the_ssh_relay_proxy_is_the_flag_then_the_resolved_proxy() {
    use ssh_relay_check::choose_proxy;
    let resolved = parsed("http://resolved.example:3128");
    // The flag wins over the resolver, and the resolver over nothing.
    assert_eq!(
        choose_proxy(Some("flag.example:8080"), Some(&resolved)),
        Ok(Some("flag.example:8080".into()))
    );
    assert_eq!(
        choose_proxy(None, Some(&resolved)),
        Ok(Some("resolved.example:3128".into()))
    );
    // No proxy is a direct dial.
    assert_eq!(choose_proxy(None, None), Ok(None));
}

// T-322: the probe sends CONNECT with no credentials, so it takes an HTTP
// proxy on TCP with no credentials only. It names the proxy that it refuses
// and never shows the credentials.
#[test]
fn the_ssh_relay_proxy_refuses_socks5_credentials_unix_and_exec() {
    use ssh_relay_check::choose_proxy;
    let refused = |spec: &str| {
        let proxy = parsed(spec);
        choose_proxy(None, Some(&proxy)).unwrap_err()
    };
    let socks = refused("socks5h://socks.example:1080");
    assert!(
        socks.contains("HTTP CONNECT proxy") && socks.contains("socks5h://socks.example:1080"),
        "{socks}"
    );
    let creds = refused("http://user:s3cret@auth.example:3128");
    assert!(
        creds.contains("unauthenticated HTTP CONNECT proxy")
            && creds.contains("http://auth.example:3128"),
        "{creds}"
    );
    assert!(
        !creds.contains("s3cret") && !creds.contains("user"),
        "{creds}"
    );
    let unix = refused("http+unix:/run/proxy.sock");
    assert!(
        unix.contains("HTTP CONNECT proxy") && unix.contains("unix"),
        "{unix}"
    );
    // The flag is a plain authority: it needs no check of the resolver.
    assert_eq!(
        choose_proxy(Some("p.example:1"), Some(&parsed("socks5://x:1"))),
        Ok(Some("p.example:1".into()))
    );
}

// T-322: the row names the proxy, or says that there is none, and the JSON
// document has the key `proxy`.
#[test]
fn the_ssh_relay_row_shows_the_proxy() {
    let mut target = ssh_target();
    let probe = crate::relay::probe::Probe::default();
    let (_, none) = ssh_relay_check::judge(&target, &probe);
    assert!(none.contains("proxy none"), "{none}");
    target.proxy = Some("p.example:3128".into());
    let (_, with) = ssh_relay_check::judge(&target, &probe);
    assert!(with.contains("proxy p.example:3128"), "{with}");
    let row = SshRelay {
        verdict: Verdict::Ok,
        target: Some(target),
        probe: None,
        detail: String::new(),
    };
    assert_eq!(row.to_json()["proxy"], "http://p.example:3128");
    assert_eq!(SshRelay::none().to_json()["proxy"], Value::Null);
}

// T-322: a refused proxy gives the verdict `fail` with its reason, and the
// probe does not dial (the relay address is a name that does not exist).
#[tokio::test]
async fn the_ssh_relay_row_of_a_refused_proxy_does_not_dial() {
    let mut target = ssh_target();
    target.refusal = Some("the SSH relay probe needs an HTTP CONNECT proxy".into());
    let row = ssh_relay_check::ssh_relay(target, Duration::from_secs(5)).await;
    assert_eq!(row.verdict, Verdict::Refused);
    assert!(row.probe.is_none());
    assert!(
        row.detail.contains("needs an HTTP CONNECT proxy"),
        "{}",
        row.detail
    );
    assert_eq!(row.to_json()["proxy"], Value::Null);
}
