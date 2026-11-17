//! The tests of the limits of a door (T-405).

use super::*;

#[test]
fn sizes_are_decimal() {
    assert_eq!(parse_size("5MB").unwrap(), 5_000_000);
    assert_eq!(parse_size("1.5GB").unwrap(), 1_500_000_000);
    assert_eq!(parse_size("300k").unwrap(), 300_000);
    assert_eq!(parse_size("1000").unwrap(), 1000);
    for bad in ["", "MB", "5XB", "0", "-1MB", "1e3"] {
        assert!(parse_size(bad).is_err(), "{bad}");
    }
}

#[test]
fn the_keys_need_a_quota() {
    assert!(Limits::parse(None, None, Some("month"), None).is_err());
    assert!(Limits::parse(None, None, None, Some("close")).is_err());
    assert!(Limits::parse(None, Some("1GB"), Some("year"), None).is_err());
    assert!(Limits::parse(None, Some("1GB"), None, Some("stop")).is_err());
    let l = Limits::parse(Some("5MB"), Some("20GB"), Some("month"), Some("close")).unwrap();
    assert_eq!(l.rate, Some(5_000_000));
    assert_eq!(l.quota, Some(20_000_000_000));
    assert_eq!(l.period, Period::Month);
    assert_eq!(l.on_exhaust, Exhaust::Close);
    assert!(Limits::parse(None, None, None, None).unwrap().is_none());
}

#[test]
fn the_periods_start_at_utc_midnight() {
    // 2026-11-16 15:00:00 UTC is a Monday.
    let t = 1_794_841_200;
    assert_eq!(Period::Day.bounds(t), (1_794_787_200, 1_794_873_600));
    assert_eq!(Period::Week.bounds(t), (1_794_787_200, 1_795_392_000));
    // 2026-11-01 and 2026-12-01.
    assert_eq!(Period::Month.bounds(t), (1_793_491_200, 1_796_083_200));
    // December rolls to the next year: 2026-12-31 and 2027-01-01.
    assert_eq!(Period::Month.bounds(1_798_675_200).1, 1_798_761_600);
    assert_eq!(Period::None.bounds(t), (0, i64::MAX));
}

#[test]
fn the_quota_is_used_and_kept_in_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("state").join("router-usage.json");
    let limits = Limits::parse(None, Some("1000"), Some("day"), None).unwrap();
    {
        let l = Limiter::new(limits, Some((file.clone(), "127.0.0.1:1080".into())));
        assert!(!l.exhausted());
        l.count(600);
        assert!(!l.exhausted());
        assert!(!l.must_close());
        l.count(400);
        assert!(l.exhausted());
        assert!(!l.must_close(), "refuse lets the open flows finish");
        l.save().unwrap();
    }
    let l = Limiter::new(limits, Some((file.clone(), "127.0.0.1:1080".into())));
    assert_eq!(l.used(), 1000, "a restart keeps the usage of the period");
    assert!(l.exhausted());
    let other = Limiter::new(limits, Some((file, "127.0.0.1:1081".into())));
    assert_eq!(other.used(), 0, "each door has its own usage");
}

#[test]
fn the_rate_gives_a_burst_then_waits() {
    let limits = Limits::parse(Some("100KB"), None, None, None).unwrap();
    let l = Limiter::new(limits, None);
    // The bucket starts empty: a large take soon after gets little.
    assert!(l.take(Way::Out, 1_000_000).map_or(true, |n| n < 1000));
    std::thread::sleep(Duration::from_millis(50));
    let n = l.take(Way::Out, 1_000_000).unwrap();
    assert!((1000..=16384).contains(&n), "{n}");
    l.give_back(Way::Out, n);
    assert!(l.take(Way::Out, 1).is_ok());
    // The other direction has its own bucket, filled over the same time.
    assert!(l.take(Way::In, 1000).is_ok());
}

/// A door with a rate carries a transfer at about its rate; a door at its
/// quota refuses a new flow, and with `close` it ends the open flow.
#[tokio::test]
async fn a_door_keeps_its_rate_and_its_quota() {
    use crate::network::{Dial as _, Target};
    use crate::router::door_stats::{Counting, DoorStats};
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    crate::test_host::need_loopback!();
    // A sink that reads each byte.
    let sink = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let far = sink.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (mut c, _) = sink.accept().await.unwrap();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 65536];
                while c.read(&mut buf).await.is_ok_and(|n| n > 0) {}
            });
        }
    });
    let deadline = || tokio::time::Instant::now() + Duration::from_secs(10);

    // 200 kB at 400 kB/s takes about 0.5 s (the burst is 40 kB).
    let limits = Limits::parse(Some("400KB"), None, None, None).unwrap();
    let stats = Arc::new(DoorStats::limited(Arc::new(Limiter::new(limits, None))));
    let door = Counting::new(Arc::new(crate::dialers::Direct), stats.clone());
    let mut c = door.dial(&Target::Addr(far), deadline()).await.unwrap();
    let start = std::time::Instant::now();
    c.io.write_all(&vec![7u8; 200_000]).await.unwrap();
    let took = start.elapsed();
    assert!(took >= Duration::from_millis(350), "{took:?}");
    assert!(took <= Duration::from_secs(3), "{took:?}");
    assert_eq!(stats.bytes(), 200_000);

    // A quota of 10 kB, close: the open flow ends at the quota, and a new
    // flow is refused.
    let limits = Limits::parse(None, Some("10KB"), Some("day"), Some("close")).unwrap();
    let stats = Arc::new(DoorStats::limited(Arc::new(Limiter::new(limits, None))));
    let door = Counting::new(Arc::new(crate::dialers::Direct), stats.clone());
    let mut c = door.dial(&Target::Addr(far), deadline()).await.unwrap();
    c.io.write_all(&[1u8; 10_000]).await.unwrap();
    let err = c.io.write_all(&[1u8; 10]).await.unwrap_err();
    assert!(err.to_string().contains("quota"), "{err}");
    let refused = door
        .dial(&Target::Addr(far), deadline())
        .await
        .err()
        .unwrap();
    assert!(refused.to_string().contains("quota"), "{refused}");
    assert_eq!(stats.json()["limits"]["used"], 10_000);
    assert_eq!(stats.json()["limits"]["exhausted"], true);
}

/// GitHub #637: with `close`, an open flow ends at the quota. A read does
/// not carry a full buffer past it.
#[tokio::test]
async fn a_read_stops_at_the_quota() {
    use crate::network::{Dial as _, Target};
    use crate::router::door_stats::{Counting, DoorStats};
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    crate::test_host::need_loopback!();
    // A source that sends 135 091 bytes.
    let source = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let far = source.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut c, _) = source.accept().await.unwrap();
        let _ = c.write_all(&vec![7u8; 135_091]).await;
    });
    let limits = Limits::parse(None, Some("1KB"), Some("day"), Some("close")).unwrap();
    let stats = Arc::new(DoorStats::limited(Arc::new(Limiter::new(limits, None))));
    let door = Counting::new(Arc::new(crate::dialers::Direct), stats.clone());
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let mut c = door.dial(&Target::Addr(far), deadline).await.unwrap();
    // Wait for the bytes, so that one read could take a full buffer.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let mut got = 0;
    let mut buf = vec![0u8; 65536];
    let err = loop {
        match c.io.read(&mut buf).await {
            Ok(0) => panic!("the flow ended with no error after {got} bytes"),
            Ok(n) => got += n,
            Err(err) => break err,
        }
    };
    assert!(err.to_string().contains("quota"), "{err}");
    assert_eq!(got, 1000);
    assert_eq!(stats.json()["limits"]["used"], 1000);
}

// GitHub #619: the usage file is in the `state_dir` of the router file when
// the file has one, so two routers with two state directories do not share
// one count.
#[test]
fn the_usage_file_follows_the_state_dir_of_the_router() {
    let dir = std::path::Path::new("/r/state");
    assert_eq!(usage_file(Some(dir)), Some(dir.join("router-usage.json")));
    if let Some(global) = usage_file(None) {
        assert!(global.ends_with("router-usage.json"), "{global:?}");
        assert_ne!(global, dir.join("router-usage.json"));
    }
}
