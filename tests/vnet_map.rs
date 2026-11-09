//! T-193: the names of `--map` answer at the gateway of the stack.

use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use podnet::vnet::dns::{build_query, parse_answers, Resolver, TYPE_A};
use podnet::vnet::{NetStack, StackConfig, VirtAddr};

#[tokio::test]
async fn a_mapped_name_is_answered_at_the_gateway() {
    let subnet = StackConfig::default().subnet;
    let resolver =
        Resolver::from_maps(&["web=10.66.0.2".to_string()], &[], &subnet).expect("the map");
    let config = StackConfig {
        dns: Some(Arc::new(resolver)),
        ..StackConfig::default()
    };
    let mut net = NetStack::loopback_with(config);
    let mut client = net
        .bind_udp(VirtAddr::new("10.66.0.2".parse().unwrap(), 40053))
        .await
        .unwrap();
    let gateway = VirtAddr::new("10.66.0.1".parse().unwrap(), 53);
    let query = build_query(7, "web", TYPE_A).unwrap();
    client.send_to(&query, gateway).await.unwrap();
    let mut buffer = [0u8; 512];
    let (n, _) = tokio::time::timeout(Duration::from_secs(2), client.recv_from(&mut buffer))
        .await
        .expect("no answer in 2 s")
        .unwrap();
    assert_eq!(
        parse_answers(&buffer[..n]).unwrap(),
        vec!["10.66.0.2".parse::<IpAddr>().unwrap()]
    );
    net.shutdown().await;
}

#[test]
fn a_map_outside_the_subnet_or_with_a_bad_form_is_refused() {
    let subnet = StackConfig::default().subnet;
    assert!(Resolver::from_maps(&["web=192.168.1.1".to_string()], &[], &subnet).is_err());
    assert!(Resolver::from_maps(&["web".to_string()], &[], &subnet).is_err());
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("hosts");
    std::fs::write(&file, "# names\n10.66.0.5 db cache\napi=10.66.0.6\n").unwrap();
    let resolver = Resolver::from_maps(&[], &[file], &subnet).unwrap();
    assert_eq!(resolver.get("cache").unwrap().len(), 1);
    assert_eq!(resolver.get("API").unwrap()[0].to_string(), "10.66.0.6");
}
