use std::net::Ipv4Addr;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use podnet_core::network::{recv_from, send_to};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};
use tokio::net::UnixListener;

use super::*;
use crate::stack::StackConfig;
use crate::switch::{attach, serve, Faults};
use crate::ProxyListen;

fn soon() -> Instant {
    Instant::now() + Duration::from_secs(10)
}

fn config(last: u8) -> StackConfig {
    StackConfig {
        local_v4: Ipv4Addr::new(10, 66, 0, last),
        ..StackConfig::default()
    }
}

/// Copy each byte that `io` reads back to it, until the end.
async fn echo<S: AsyncRead + AsyncWrite + Unpin>(mut io: S) {
    let mut buf = [0u8; 1024];
    while let Ok(n) = io.read(&mut buf).await {
        if n == 0 || io.write_all(&buf[..n]).await.is_err() {
            break;
        }
    }
}

async fn ping(conn: &mut Connected) {
    conn.io.write_all(b"ping").await.unwrap();
    let mut back = [0u8; 4];
    tokio::time::timeout(Duration::from_secs(10), conn.io.read_exact(&mut back))
        .await
        .expect("the echo in time")
        .unwrap();
    assert_eq!(&back, b"ping");
}

/// Two stacks on one switch: the dialer of one stack carries TCP (to a
/// name of the table) and UDP to the other stack.
#[tokio::test]
async fn dialer_vnet_carries_tcp_and_udp_between_two_stacks() {
    let dir = tempfile::tempdir().unwrap();
    let listen = ProxyListen::Unix(dir.path().join("switch.sock"));
    let switch = serve(listen.clone(), None).await.unwrap();
    let server = Arc::new(NetStack::over_link(config(2)));
    let client = Arc::new(NetStack::over_link(config(3)));
    let _a = attach(&server, &listen, Faults::default(), None)
        .await
        .unwrap();
    let _b = attach(&client, &listen, Faults::default(), None)
        .await
        .unwrap();

    let server_ip = IpAddr::V4(Ipv4Addr::new(10, 66, 0, 2));
    let mut listener = server.listen(VirtAddr::new(server_ip, 8080)).await.unwrap();
    tokio::spawn(async move {
        while let Ok(stream) = listener.accept().await {
            tokio::spawn(echo(stream));
        }
    });
    let names = Resolver::new().with("server.vnet", server_ip);
    let dialer = VnetDialer::new(client.clone(), IpAddr::V4(Ipv4Addr::new(10, 66, 0, 3)))
        .with_names(Arc::new(names));
    assert_eq!(dialer.label(), "vnet:10.66.0.3");
    assert!(dialer.names_inside());
    let mut conn = dialer
        .dial(&Target::new("server.vnet", 8080), soon())
        .await
        .unwrap();
    assert_eq!(conn.peer, Some(SocketAddr::new(server_ip, 8080)));
    ping(&mut conn).await;

    let mut answer = server
        .bind_udp(VirtAddr::new(server_ip, 5353))
        .await
        .unwrap();
    let datagrams = dialer.datagrams().await.unwrap();
    let local = datagrams.local_addr().unwrap();
    assert_eq!(local.ip(), IpAddr::V4(Ipv4Addr::new(10, 66, 0, 3)));
    send_to(&*datagrams, b"query", SocketAddr::new(server_ip, 5353))
        .await
        .unwrap();
    let mut buf = [0u8; 64];
    let (n, from) = tokio::time::timeout(Duration::from_secs(10), answer.recv_from(&mut buf))
        .await
        .expect("the datagram in time")
        .unwrap();
    assert_eq!(&buf[..n], b"query");
    assert_eq!(SocketAddr::new(from.ip, from.port), local);
    answer.send_to(b"answer", from).await.unwrap();
    let (n, from) = tokio::time::timeout(Duration::from_secs(10), recv_from(&*datagrams, &mut buf))
        .await
        .expect("the answer in time")
        .unwrap();
    assert_eq!(&buf[..n], b"answer");
    assert_eq!(from, SocketAddr::new(server_ip, 5353));
    // A second socket takes the next port.
    let other = dialer.datagrams().await.unwrap();
    assert_ne!(other.local_addr().unwrap().port(), local.port());
    switch.abort();
}

#[tokio::test]
async fn dialer_vnet_refuses_a_name_that_is_not_in_its_table() {
    let stack = Arc::new(NetStack::loopback());
    let dialer = VnetDialer::new(stack, IpAddr::V4(Ipv4Addr::new(10, 66, 0, 2)));
    assert!(!dialer.names_inside());
    let err = dialer
        .dial(&Target::new("nowhere.vnet", 80), soon())
        .await
        .expect_err("no table of names");
    assert_eq!((err.stage, err.class), (Stage::Dns, Class::NotFound));
    assert!(err.to_string().contains("nowhere.vnet"), "{err}");
}

/// A SOCKS5 proxy with no authentication on a Unix socket: it keeps the
/// target of each CONNECT, then echoes the bytes.
fn fake_socks5(path: &Path) -> Arc<Mutex<Vec<String>>> {
    let listener = UnixListener::bind(path).unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    tokio::spawn(async move {
        while let Ok((mut s, _)) = listener.accept().await {
            let log = log.clone();
            tokio::spawn(async move {
                let mut head = [0u8; 2];
                s.read_exact(&mut head).await.unwrap();
                let mut methods = vec![0u8; head[1] as usize];
                s.read_exact(&mut methods).await.unwrap();
                s.write_all(&[5, 0]).await.unwrap();
                let mut req = [0u8; 4];
                s.read_exact(&mut req).await.unwrap();
                let host = match req[3] {
                    1 => {
                        let mut ip = [0u8; 4];
                        s.read_exact(&mut ip).await.unwrap();
                        Ipv4Addr::from(ip).to_string()
                    }
                    3 => {
                        let len = s.read_u8().await.unwrap();
                        let mut name = vec![0u8; len as usize];
                        s.read_exact(&mut name).await.unwrap();
                        String::from_utf8(name).unwrap()
                    }
                    other => panic!("address type {other}"),
                };
                let port = s.read_u16().await.unwrap();
                log.lock().unwrap().push(format!("{host}:{port}"));
                s.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).await.unwrap();
                echo(s).await;
            });
        }
    });
    seen
}

#[tokio::test]
async fn dialer_upstream_chain_sends_the_name_to_a_socks5_hop() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("socks.sock");
    let seen = fake_socks5(&path);
    let spec = format!("unix:{}", path.display());
    let chain = UpstreamChain::parse(&[spec.as_str()]).unwrap();
    assert_eq!(chain.label(), format!("socks5://unix:{}", path.display()));
    assert!(chain.names_inside());
    let mut conn = chain
        .dial(&Target::new("svc.example", 443), soon())
        .await
        .unwrap();
    ping(&mut conn).await;
    assert_eq!(*seen.lock().unwrap(), ["svc.example:443"]);
    let err = chain.datagrams().await.expect_err("TCP only");
    assert_eq!(err.class, Class::Unavailable);
}

#[test]
fn dialer_upstream_chain_needs_a_hop_and_hides_the_password() {
    let err = UpstreamChain::new(Vec::new())
        .err()
        .expect("an empty chain");
    assert_eq!(err.class, Class::Usage);
    let chain = UpstreamChain::parse(&["socks5://user:secret@10.0.0.1:1080", "direct"]);
    assert!(chain.is_ok());
    let chain = UpstreamChain::parse(&["http://user:secret@10.0.0.1:3128"]).unwrap();
    assert_eq!(chain.label(), "http://10.0.0.1:3128");
    assert!(!UpstreamChain::parse(&["direct"]).unwrap().names_inside());
}

/// A `tailscaled` LocalAPI on a Unix socket: it keeps the head of each
/// request, answers `status`, then echoes the bytes when the status is 101.
fn fake_tailscaled(path: &Path, status: u16) -> Arc<Mutex<Vec<String>>> {
    let listener = UnixListener::bind(path).unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    tokio::spawn(async move {
        while let Ok((mut s, _)) = listener.accept().await {
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") {
                if s.read(&mut byte).await.unwrap() == 0 {
                    break;
                }
                head.push(byte[0]);
            }
            log.lock()
                .unwrap()
                .push(String::from_utf8_lossy(&head).into_owned());
            let answer = format!("HTTP/1.1 {status} Answer\r\nContent-Length: 0\r\n\r\n");
            s.write_all(answer.as_bytes()).await.unwrap();
            if status == 101 {
                tokio::spawn(echo(s));
            }
        }
    });
    seen
}

#[tokio::test]
async fn dialer_tailnet_dials_through_the_local_api() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tailscaled.sock");
    let seen = fake_tailscaled(&path, 101);
    let tailnet = Tailnet::new(&path);
    assert_eq!(tailnet.label(), format!("tailscale:{}", path.display()));
    assert!(tailnet.names_inside());
    let mut conn = tailnet
        .dial(&Target::new("office", 22), soon())
        .await
        .unwrap();
    ping(&mut conn).await;
    let head = seen.lock().unwrap()[0].clone();
    assert!(head.contains("Dial-Host: office\r\n"), "{head}");
    assert!(head.contains("Dial-Port: 22\r\n"), "{head}");
}

#[tokio::test]
async fn dialer_tailnet_gives_the_refusal_of_the_daemon() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tailscaled.sock");
    let _seen = fake_tailscaled(&path, 403);
    let err = Tailnet::new(&path)
        .dial(&Target::new("office", 22), soon())
        .await
        .expect_err("a refusal");
    assert_eq!(err.stage, Stage::Connect);
    assert!(err.to_string().contains("status 403"), "{err}");
}

#[test]
fn a_pod_rule_matches_a_name_or_an_address_as_pod_r_does() {
    let corp = PodRule::parse("domain:.corp.test").unwrap();
    assert_eq!(corp.label(), "domain:.corp.test");
    assert!(corp.matches(&Target::new("git.corp.test", 443)));
    assert!(corp.matches(&Target::new("corp.test", 443)));
    assert!(!corp.matches(&Target::new("corp.test.example", 443)));
    assert!(!corp.matches(&Target::new("10.1.2.3", 443)));
    let lan = PodRule::parse("cidr:10.0.0.0/8").unwrap();
    assert!(lan.matches(&Target::new("10.1.2.3", 443)));
    assert!(!lan.matches(&Target::new("192.0.2.1", 443)));
    assert!(!lan.matches(&Target::new("git.corp.test", 443)));
    assert!(PodRule::parse("port:22").is_err());
}

#[tokio::test]
async fn the_rules_combinator_takes_pod_rules() {
    use podnet_core::network::{Rule, Rules};
    let refuse = Rule {
        matcher: Arc::new(PodRule::parse("cidr:10.0.0.0/8").unwrap()),
        way: None,
    };
    let rules = Rules::new(vec![refuse], None).unwrap();
    let err = rules
        .dial(
            &Target::new("10.0.0.1", 22),
            Instant::now() + Duration::from_secs(5),
        )
        .await
        .expect_err("refused");
    assert_eq!(err.text(), "the rule cidr:10.0.0.0/8 refuses 10.0.0.1:22");
}
