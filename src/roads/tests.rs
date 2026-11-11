use std::net::{IpAddr, Ipv4Addr};
use std::time::Duration;

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use super::*;
use crate::vnet::proxy::ProxyListen;
use crate::vnet::stack::StackConfig;
use crate::vnet::switch::{attach, serve, Faults};

fn stack(last: u8) -> Arc<NetStack> {
    Arc::new(NetStack::over_link(StackConfig {
        local_v4: Ipv4Addr::new(10, 66, 0, last),
        ..StackConfig::default()
    }))
}

/// T-062: a table of services through the vnet road. A second stack on the
/// same switch reaches it.
#[tokio::test]
async fn ingress_vnet_road_serves_a_table_to_another_stack() {
    let dir = tempfile::tempdir().unwrap();
    let listen = ProxyListen::Unix(dir.path().join("switch.sock"));
    let _switch = serve(listen.clone(), None).await.unwrap();
    let (server, client) = (stack(2), stack(3));
    let _a = attach(&server, &listen, Faults::default(), None)
        .await
        .unwrap();
    let _b = attach(&client, &listen, Faults::default(), None)
        .await
        .unwrap();
    let addr = VirtAddr::new(IpAddr::V4(Ipv4Addr::new(10, 66, 0, 2)), 8080);
    let services = Services::new().rule(
        None,
        crate::ingress::PathPattern::prefix("/"),
        axum::Router::new().fallback(|| async { "over the vnet" }),
    );
    let road = Vnet {
        stack: server.clone(),
        addr,
    };
    assert_eq!(road.name(), "vnet");
    let published = road.publish(services.router()).await.unwrap();
    assert_eq!(published.url.as_deref(), Some("http://10.66.0.2:8080/"));
    let running = tokio::spawn(published.run());
    let mut stream = client.connect(addr).await.unwrap();
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: a.vnet\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut answer = String::new();
    tokio::time::timeout(Duration::from_secs(10), stream.read_to_string(&mut answer))
        .await
        .expect("the answer in time")
        .unwrap();
    assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
    assert!(answer.ends_with("over the vnet"), "{answer}");
    running.abort();
}

#[test]
fn ingress_the_roads_have_their_names() {
    assert_eq!(Relay::default().name(), "relay");
    let tunnel = Tunnel {
        config: Config::default(),
    };
    assert_eq!(tunnel.name(), "tunnel");
}

#[test]
fn ingress_the_table_of_roads_parses_each_form() {
    let names: Vec<&str> = roads().iter().map(|r| r.name).collect();
    assert_eq!(names, ["local", "tunnel", "relay", "vnet"]);
    let config = Config::default();
    for (spec, name) in [
        ("local:127.0.0.1:0", "local"),
        ("local:unix:/tmp/x.sock", "local"),
        ("tunnel", "tunnel"),
        ("relay", "relay"),
        ("relay:p-1", "relay"),
    ] {
        assert_eq!(parse(spec, &config).unwrap().name(), name, "{spec}");
    }
    for bad in ["", "local:", "relay:", "vnet", "warp", "tunnel:x"] {
        let err = parse(bad, &config).err().expect(bad).to_string();
        assert!(!err.contains('\n'), "{err}");
    }
}
