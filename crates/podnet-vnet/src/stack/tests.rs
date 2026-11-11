//! The proof of `net doctor`, and the policy of this crate through the
//! hooks of `pod-netstack`. The tests of the stack itself are in
//! `pod-netstack` (PodUtils/podcrates), with their names.

use std::sync::Arc;
use std::time::Duration;

use super::*;
use crate::dns::Resolver;
use crate::policy::Acl;

#[test]
fn proof_completes() {
    let report = loopback_proof();
    assert!(report.passed(), "{}", report.render());
    assert!(
        report.link_layer == "in-process loopback",
        "the loopback proof must say which link it ran over: {:?}",
        report.link_layer
    );
    assert!(report.bytes_received > 20);
    assert!(report.render().contains("PROOF"));
    // Issue 53: the teardown ran.
    assert_eq!(report.server_exchange_state, "ESTABLISHED");
    assert_eq!(report.server_state, "CLOSED", "{}", report.render());
    assert!(
        matches!(report.client_state.as_str(), "TIME-WAIT" | "CLOSED"),
        "{}",
        report.render()
    );
    assert!(report.render().contains("states after the teardown"));
}

#[tokio::test]
async fn async_loopback_round_trip() {
    let (request_len, response) = loopback_round_trip().await.unwrap();
    assert_eq!(request_len, 23);
    let text = String::from_utf8_lossy(&response);
    assert!(text.contains("200 OK"), "{text:?}");
    assert!(text.contains("GET /hello"), "{text:?}");
}

/// The resolver of this crate is the UDP service at the gateway.
#[tokio::test]
async fn virtual_dns_answers_at_the_gateway() {
    use crate::dns::{build_query, parse_answers, TYPE_A};
    let resolver = Arc::new(Resolver::new().with("web", "10.66.0.2".parse().unwrap()));
    let config = StackConfig {
        gateway_service: Some(resolver),
        ..StackConfig::default()
    };
    let mut net = NetStack::loopback_with(config);
    let mut client = net
        .bind_udp(VirtAddr::new(net::LOCAL_IP, 40053))
        .await
        .unwrap();
    let gateway = VirtAddr::new("10.66.0.1".parse().unwrap(), 53);
    let query = build_query(0x1234, "web", TYPE_A).unwrap();
    client.send_to(&query, gateway).await.unwrap();
    let mut buffer = [0u8; 512];
    let (n, from) = tokio::time::timeout(Duration::from_secs(2), client.recv_from(&mut buffer))
        .await
        .expect("dns timed out")
        .unwrap();
    assert_eq!(VirtAddr::from(from), gateway);
    assert_eq!(
        parse_answers(&buffer[..n]).unwrap(),
        vec!["10.66.0.2".parse::<IpAddr>().unwrap()]
    );
    net.shutdown().await;
}

/// The ACL of this crate is the admit of the stack.
#[tokio::test]
async fn acl_refuses_connections() {
    let acl = Acl::default_deny();
    let config = StackConfig {
        admit: Some(Arc::new(acl)),
        ..StackConfig::default()
    };
    let mut net = NetStack::loopback_with(config);
    let err = net
        .connect(VirtAddr::new(net::LOCAL_IP, 8080))
        .await
        .unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(net.stats().refused, 1);
    net.shutdown().await;
}
