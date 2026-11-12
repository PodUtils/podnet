use std::sync::atomic::{AtomicUsize, Ordering};

use podnet_netns::dns::{build_response, ParsedQuery, TYPE_A};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use super::*;

fn file(extra: &str) -> String {
    format!(
        "networks:\n  home: direct\n  spare: direct\ngroups:\n  egress: home*1, spare*1\n{extra}doors:\n  - listen: unix:/tmp/r-res.sock\n    via: egress\n"
    )
}

/// T-098 item 3: `resolvers` is operator input; each error names the key.
#[test]
fn route_config_resolvers_errors_name_the_key() {
    let ok = parse(&file("resolvers:\n  home: 10.0.0.53:53\n")).unwrap();
    assert_eq!(ok.resolvers["home"], "10.0.0.53:53");
    let cases = [
        ("resolvers:\n  nowhere: 10.0.0.53:53\n", "resolvers.nowhere"),
        ("resolvers:\n  egress: 10.0.0.53:53\n", "resolvers.egress"),
        ("resolvers:\n  home: nohost\n", "resolvers.home"),
        ("resolvers:\n  home: 10.0.0.53:0\n", "resolvers.home"),
    ];
    for (extra, key) in cases {
        let err = format!("{:#}", parse(&file(extra)).expect_err(key));
        assert!(err.contains(key), "{key}: {err}");
    }
}

/// A DNS server over TCP that answers `127.0.0.1` for each A query, and counts
/// the queries; a TCP server that says `hello` to each client.
async fn servers() -> (u16, Arc<AtomicUsize>, u16) {
    let dns = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dns_port = dns.local_addr().unwrap().port();
    let queries = Arc::new(AtomicUsize::new(0));
    let counted = queries.clone();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = dns.accept().await {
            let counted = counted.clone();
            tokio::spawn(async move {
                while let Ok(len) = stream.read_u16().await {
                    let mut msg = vec![0u8; len as usize];
                    if stream.read_exact(&mut msg).await.is_err() {
                        return;
                    }
                    counted.fetch_add(1, Ordering::Relaxed);
                    let q = ParsedQuery::parse(&msg).unwrap();
                    let ips: Vec<std::net::IpAddr> = if q.qtype == TYPE_A && q.name == "svc.test" {
                        vec!["127.0.0.1".parse().unwrap()]
                    } else {
                        Vec::new()
                    };
                    let answer = build_response(&q, &ips, 0);
                    let _ = stream.write_u16(answer.len() as u16).await;
                    let _ = stream.write_all(&answer).await;
                }
            });
        }
    });
    let hello = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let hello_port = hello.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = hello.accept().await {
            let _ = stream.write_all(b"hello").await;
            let _ = stream.shutdown().await;
        }
    });
    (dns_port, queries, hello_port)
}

/// A SOCKS5 CONNECT to the name `svc.test` through `door`: the reply code
/// and what the flow says.
async fn connect(door: &Arc<podnet_vnet::door::Door>, port: u16) -> (u8, String) {
    let (mut client, server) = tokio::io::duplex(4096);
    door.clone().spawn(server);
    client.write_all(&[5, 1, 0]).await.unwrap();
    let mut method = [0u8; 2];
    client.read_exact(&mut method).await.unwrap();
    let mut request = vec![5, 1, 0, 3, 8];
    request.extend_from_slice(b"svc.test");
    request.extend_from_slice(&port.to_be_bytes());
    client.write_all(&request).await.unwrap();
    let mut reply = [0u8; 10];
    client.read_exact(&mut reply).await.unwrap();
    let mut said = String::new();
    if reply[1] == 0 {
        client.read_to_string(&mut said).await.unwrap();
    }
    (reply[1], said)
}

/// T-098 item 3, GitHub #374: a flow that the group gives to a member
/// resolves its name through the network of that member (the upstream of
/// `resolvers`), and the host resolver is not asked: `svc.test` has no
/// address on this host.
#[tokio::test]
async fn route_resolver_pinned_member_resolves_through_its_network() {
    crate::test_host::need_loopback!();
    let (dns_port, queries, hello_port) = servers().await;
    let text = file(&format!("resolvers:\n  home: 127.0.0.1:{dns_port}\n"));
    let router = Router::up(&parse(&text).unwrap()).await.unwrap();
    router.groups[0].1.pin(Some("home")).unwrap();
    let (code, said) = connect(&router.doors[0].door, hello_port).await;
    assert_eq!((code, said.as_str()), (0, "hello"));
    assert_eq!(
        queries.load(Ordering::Relaxed),
        2,
        "A and AAAA over the network of the member"
    );
}
