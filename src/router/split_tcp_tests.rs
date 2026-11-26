use super::*;

/// A display fingerprint that parses: the key of a new install.
fn far() -> String {
    let dir = tempfile::tempdir().unwrap();
    crate::identity::create(dir.path()).unwrap().fingerprint()
}

/// T-611: one rule reads each value of `splits.NAME`. A value with
/// `tcp=` gives the far end; the old forms keep their meaning; each bad
/// value names the key and the fault.
#[test]
fn route_config_split_tcp_reads_each_form() {
    let fp = far();
    let spec = |up_carrier: &str, down_carrier: &str| Spec {
        far: fp.clone(),
        up: "127.0.0.1:7001".into(),
        down: "127.0.0.1:7002".into(),
        up_carrier: up_carrier.into(),
        down_carrier: down_carrier.into(),
    };
    let parsed = |tcp: Option<Spec>| {
        Some(Parsed {
            up: "a".into(),
            down: "b".into(),
            tcp,
        })
    };
    let at = format!("tcp={fp}@127.0.0.1:7001|127.0.0.1:7002");
    let good: Vec<(String, Option<Parsed>)> = vec![
        ("off".into(), None),
        ("up=a down=b".into(), parsed(None)),
        ("up=a,down=b".into(), parsed(None)),
        (
            format!("up=a down=b {at}"),
            parsed(Some(spec("tcp", "tcp"))),
        ),
        (
            format!("{at} down=b up=a carriers=quic|ws"),
            parsed(Some(spec("quic", "ws"))),
        ),
    ];
    for (value, want) in good {
        assert_eq!(read("g", &value, "a, b").unwrap(), want, "{value}");
    }
    let bad: Vec<(String, &str)> = vec![
        ("tcp".into(), "TCP"),
        (at.clone(), "up=MEMBER"),
        (format!("off {at}"), "needs up=MEMBER"),
        (
            "up=a down=b tcp=127.0.0.1:7001|x".into(),
            "FINGERPRINT@UP|DOWN",
        ),
        (
            format!("up=a down=b tcp={fp}@127.0.0.1:7001"),
            "FINGERPRINT@UP|DOWN",
        ),
        (format!("up=a down=b tcp={fp}@|x"), "FINGERPRINT@UP|DOWN"),
        (format!("up=a down=b tcp={fp}@a|b|c"), "FINGERPRINT@UP|DOWN"),
        ("up=a down=b tcp=nokey@a|b".into(), "no key"),
        (format!("up=a down=b {at} {at}"), "once"),
        ("up=a down=b carriers=tcp|tcp".into(), "needs tcp="),
        (
            format!("up=a down=b {at} carriers=unix|tcp"),
            "unix carrier cannot",
        ),
        (
            format!("up=a down=b {at} carriers=tcp|icmp"),
            "icmp carrier cannot",
        ),
        (format!("up=a down=b {at} carriers=tcp"), "carriers=UP|DOWN"),
        (format!("up=a down=a {at}"), "needs two"),
        (format!("up=a down=z {at}"), "not a member"),
    ];
    for (value, word) in bad {
        let err = format!("{:#}", read("g", &value, "a, b").unwrap_err());
        assert!(
            err.starts_with("splits.g: ") && err.contains(word),
            "{value}: {err}"
        );
    }
    assert_eq!(
        spec("tcp", "tcp").address(),
        format!("{fp}@127.0.0.1:7001|{fp}@127.0.0.1:7002")
    );
}

/// T-611: a router file with `tcp=` reads, and its dry run shows the TCP
/// split in the status of the group with no session (nothing is dialed).
/// A flow of the dry run falls back to the pick and counts.
#[tokio::test]
async fn route_config_split_tcp_shows_in_the_status_of_a_dry_run() {
    let fp = far();
    let text = format!(
        "networks:\n  a: direct\n  b: direct\ngroups:\n  g: a, b\nsplits:\n  g: up=a down=b tcp={fp}@127.0.0.1:7001|127.0.0.1:7002\ndoors:\n  - listen: unix:/tmp/split-tcp.sock\n    via: g\n"
    );
    let plan = crate::router::Router::plan(&crate::router::parse(&text).unwrap()).unwrap();
    assert_eq!(plan.tcp_splits.len(), 1);
    let dial = plan.ways["g"].dial.clone();
    assert!(dial.label().contains("split tcp"), "{}", dial.label());
    let refused = dial
        .dial(&Target::new("192.0.2.1", 443), Instant::now())
        .await;
    assert!(refused.is_err(), "a dry run does not dial");
    let split = &plan.status()["groups"][0]["split"];
    assert_eq!(split["mode"], "tcp", "{split}");
    assert_eq!(split["up"], "a", "{split}");
    assert_eq!(split["session"]["state"], "idle", "{split}");
    assert_eq!(split["session"]["far"], fp.as_str(), "{split}");
    assert_eq!(split["session"]["carriers"], "tcp|tcp", "{split}");
    assert_eq!(split["streams"], 0, "{split}");
    assert_eq!(split["tcp_fallbacks"], 1, "{split}");
}
