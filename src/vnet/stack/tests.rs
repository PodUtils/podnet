use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

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

#[test]
fn clock_is_monotonic() {
    let clock = Clock::starting_at(100);
    let a = clock.now_ms();
    std::thread::sleep(Duration::from_millis(2));
    let b = clock.now_ms();
    assert!(b > a);
}

#[tokio::test]
async fn async_loopback_round_trip() {
    let (request_len, response) = loopback_round_trip().await.unwrap();
    assert_eq!(request_len, 23);
    let text = String::from_utf8_lossy(&response);
    assert!(text.contains("200 OK"), "{text:?}");
    assert!(text.contains("GET /hello"), "{text:?}");
}

#[tokio::test]
async fn udp_loopback_round_trip() {
    let mut net = NetStack::loopback();
    let server_addr = VirtAddr::new(net::LOCAL_IP, 5353);
    let client_addr = VirtAddr::new(net::LOCAL_IP, 40000);
    let mut server = net.bind_udp(server_addr).await.unwrap();
    let client = net.bind_udp(client_addr).await.unwrap();
    client.send_to(b"ping", server_addr).await.unwrap();
    let mut buffer = [0u8; 64];
    let (n, from) = tokio::time::timeout(Duration::from_secs(2), server.recv_from(&mut buffer))
        .await
        .expect("udp timed out")
        .unwrap();
    assert_eq!(&buffer[..n], b"ping");
    assert_eq!(from, client_addr);
    net.shutdown().await;
}

#[tokio::test]
async fn virtual_dns_answers_at_the_gateway() {
    use crate::vnet::dns::{build_query, parse_answers, TYPE_A};
    let resolver = Arc::new(Resolver::new().with("web", "10.66.0.2".parse().unwrap()));
    let config = StackConfig {
        dns: Some(resolver),
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
    assert_eq!(from, gateway);
    assert_eq!(
        parse_answers(&buffer[..n]).unwrap(),
        vec!["10.66.0.2".parse::<IpAddr>().unwrap()]
    );
    net.shutdown().await;
}

/// A listener on an address other than the stack's own (a --forward
/// to 10.66.0.3) accepts a connection (issue 41). A connection to a
/// closed port is refused, and established() says so.
#[tokio::test]
async fn a_listener_on_another_address_accepts() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut net = NetStack::loopback();
    let other = VirtAddr::new("10.66.0.3".parse().unwrap(), 80);
    let mut listener = net.listen(other).await.unwrap();
    let server = tokio::spawn(async move {
        let mut stream = listener.accept().await.unwrap();
        stream.write_all(b"hi").await.unwrap();
        stream.shutdown();
    });
    let mut client = net.connect(other).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), client.established())
        .await
        .expect("no handshake in 3 s")
        .unwrap();
    let mut got = Vec::new();
    tokio::time::timeout(Duration::from_secs(3), client.read_to_end(&mut got))
        .await
        .expect("no EOF in 3 s")
        .unwrap();
    assert_eq!(got, b"hi");
    server.await.unwrap();
    let mut closed = net.connect(VirtAddr::new(net::LOCAL_IP, 81)).await.unwrap();
    let err = tokio::time::timeout(Duration::from_secs(3), closed.established())
        .await
        .expect("no answer in 3 s")
        .unwrap_err();
    assert!(
        matches!(
            err.kind(),
            io::ErrorKind::ConnectionRefused | io::ErrorKind::ConnectionReset
        ),
        "{err}"
    );
    net.shutdown().await;
}

#[tokio::test]
async fn acl_refuses_connections() {
    let acl = Acl::default_deny();
    let config = StackConfig {
        acl: Some(Arc::new(acl)),
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

#[tokio::test]
async fn outside_subnet_is_refused() {
    let mut net = NetStack::loopback();
    let err = net
        .connect(VirtAddr::new("10.99.0.2".parse().unwrap(), 80))
        .await
        .unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::AddrNotAvailable);
    net.shutdown().await;
}

#[tokio::test]
async fn tcp_nodelay_is_recorded() {
    let mut net = NetStack::loopback();
    let mut client = net.connect(VirtAddr::new(net::LOCAL_IP, 9)).await.unwrap();
    client.set_nodelay(true);
    assert!(client.nodelay());
    net.shutdown().await;
}

/// An application write must wake the reactor, not wait for the poll tick.
/// The timeout is below `max_poll_interval`, so a missing wakeup fails.
#[tokio::test]
async fn application_writes_wake_the_reactor() {
    let mut net = NetStack::loopback();
    let addr = VirtAddr::new(net::LOCAL_IP, 8123);
    let mut listener = net.listen(addr).await.unwrap();
    let server = tokio::spawn(async move {
        let mut stream = listener.accept().await.unwrap();
        let mut buffer = [0u8; 16];
        stream.read(&mut buffer).await.unwrap()
    });
    let mut client = net.connect(addr).await.unwrap();
    client.write_all(b"hello").await.unwrap();
    let read = tokio::time::timeout(Duration::from_millis(60), server).await;
    assert!(
        read.is_ok(),
        "the reactor did not service the write within 60ms"
    );
    assert_eq!(read.unwrap().unwrap(), 5);
    net.shutdown().await;
}

#[tokio::test]
async fn ipv6_loopback_round_trip() {
    let mut net = NetStack::loopback();
    let addr = VirtAddr::new("fd00:66::2".parse().unwrap(), 8090);
    let mut listener = net.listen(addr).await.unwrap();
    let server = tokio::spawn(async move {
        let mut stream = listener.accept().await.unwrap();
        let mut buffer = [0u8; 32];
        let n = stream.read(&mut buffer).await.unwrap();
        stream.write_all(b"ok").await.unwrap();
        n
    });
    let mut client = net.connect(addr).await.unwrap();
    assert!(
        client.local_addr().ip.is_ipv6(),
        "local is {:?}",
        client.local_addr()
    );
    client.write_all(b"v6 hello").await.unwrap();
    let mut reply = [0u8; 2];
    tokio::time::timeout(Duration::from_secs(2), client.read_exact(&mut reply))
        .await
        .expect("v6 read timed out")
        .unwrap();
    assert_eq!(&reply, b"ok");
    assert_eq!(server.await.unwrap(), 8);
    net.shutdown().await;
}
