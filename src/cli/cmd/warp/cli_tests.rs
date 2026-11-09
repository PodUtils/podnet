use super::account::plain_value;
use super::args::non_empty_key;
use super::client::identity_pairs;
use super::probe::parse_door;
use super::up::{warp_up_args, warp_up_tun_args};
use super::WarpCommand;
use crate::cli::{Cli, Command};
use clap::Parser as _;
use std::path::PathBuf;

// The IPv6-only kit, 0002: warp up --tun gives its child the front door
// and the TLS name.
#[test]
fn the_pairs_of_listen_identity() {
    let main = std::path::Path::new("/s/warp.json");
    let pairs = identity_pairs(
        &[
            "tcp://127.0.0.1:40001=/s/b.json".into(),
            "unix:/run/c.sock=/s/c.json".into(),
        ],
        Some(main),
        &["tcp://127.0.0.1:40000".into()],
    )
    .unwrap();
    assert_eq!(pairs.len(), 2);
    // Issue 178: the listener of --listen.
    assert!(identity_pairs(
        &["tcp://127.0.0.1:40000=/s/b.json".into()],
        Some(main),
        &["tcp://127.0.0.1:40000".into()],
    )
    .is_err());
    assert_eq!(pairs[1].1, PathBuf::from("/s/c.json"));
    for bad in [
        vec!["127.0.0.1:40001=/s/warp.json"],
        vec!["127.0.0.1:40001=/s/b.json", "127.0.0.1:40002=/s/b.json"],
        vec!["127.0.0.1:40001=/s/b.json", "127.0.0.1:40001=/s/c.json"],
        vec!["127.0.0.1:40001"],
        vec!["=/s/b.json"],
        vec!["nothing=/s/b.json"],
    ] {
        let specs: Vec<String> = bad.iter().map(|s| s.to_string()).collect();
        assert!(identity_pairs(&specs, Some(main), &[]).is_err(), "{bad:?}");
    }
}

#[test]
fn the_front_doors_of_a_scan() {
    assert_eq!(
        parse_door("162.159.198.1").unwrap().to_string(),
        "162.159.198.1:443"
    );
    assert_eq!(parse_door("162.159.198.1:8443").unwrap().port(), 8443);
    assert_eq!(
        parse_door("2606:4700:103::1").unwrap().to_string(),
        "[2606:4700:103::1]:443"
    );
    assert_eq!(parse_door("[2606:4700:103::1]:443").unwrap().port(), 443);
    assert!(parse_door("engage.cloudflareclient.com").is_err());
    assert!(parse_door("162.159.198.0/24").is_err());
}

#[test]
fn warp_up_tun_keeps_the_front_door() {
    let cli = Cli::try_parse_from([
        "podnet",
        "warp",
        "--masque-endpoint",
        "[2606:4700:103::2]:443",
        "--masque-sni",
        "example.test",
        "--ephemeral",
        "up",
        "--tun",
    ])
    .unwrap();
    let Command::Warp(args) = cli.command else {
        panic!("not warp")
    };
    let argv = warp_up_tun_args(&args, 30, false);
    let has = |a: &str, b: &str| argv.windows(2).any(|w| w[0] == a && w[1] == b);
    assert!(
        has("--masque-endpoint", "[2606:4700:103::2]:443"),
        "{argv:?}"
    );
    assert!(has("--masque-sni", "example.test"), "{argv:?}");
    assert!(argv.contains(&"--ephemeral".to_string()));
    let tun = argv.iter().position(|a| a == "tun").unwrap();
    assert!(argv.iter().position(|a| a == "--masque-endpoint").unwrap() < tun);
    let parsed = Cli::try_parse_from(std::iter::once("podnet".to_string()).chain(argv));
    assert!(parsed.is_ok(), "the argv parses");
}

#[test]
fn the_warp_proxy_of_warp_up_parses() {
    let cli =
        Cli::try_parse_from(["podnet", "warp", "--no-relay", "up", "--port", "40100"]).unwrap();
    let Command::Warp(args) = cli.command else {
        panic!("not warp")
    };
    let argv = warp_up_args(&args, 40100, std::path::Path::new("/r/warp.sock"), 30);
    let parsed = Cli::try_parse_from(std::iter::once("podnet".to_string()).chain(argv)).unwrap();
    let Command::Warp(inner) = parsed.command else {
        panic!("not warp: the child command line")
    };
    assert!(inner.no_relay);
    match inner.command {
        WarpCommand::Proxy { listen, detach, .. } => {
            assert_eq!(listen, ["tcp://127.0.0.1:40100", "unix:/r/warp.sock"]);
            assert!(detach.detach);
            assert_eq!(detach.name.as_deref(), Some("warp"));
        }
        other => panic!("{other:?}"),
    }
}

// Issue 178: an empty license key is a usage error; a text field has no
// quotes; warp_plus_active follows the account.
#[test]
fn warp_license_and_status_fields() {
    let err = Cli::try_parse_from(["podnet", "warp", "license", ""]).unwrap_err();
    assert_eq!(err.exit_code(), 2, "clap error");
    assert!(non_empty_key("  ").is_err());
    assert_eq!(non_empty_key(" k ").unwrap(), "k");
    assert_eq!(plain_value(&serde_json::json!("free")), "free");
    assert_eq!(plain_value(&serde_json::json!(5)), "5");
    assert_eq!(plain_value(&serde_json::Value::Null), "-");
    let free = serde_json::json!({"account_type": "free", "warp_plus": true});
    assert_eq!(podnet::warp::identity::warp_plus_label(&free), "no");
    let plus = serde_json::json!({"account_type": "limited", "premium_data": 10});
    assert_eq!(podnet::warp::identity::warp_plus_label(&plus), "yes");
}
