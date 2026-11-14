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
    assert_eq!(from, local);
    answer.send_to(b"answer", from).await.unwrap();
    let (n, from) = tokio::time::timeout(Duration::from_secs(10), recv_from(&*datagrams, &mut buf))
        .await
        .expect("the answer in time")
        .unwrap();
    assert_eq!(&buf[..n], b"answer");
    assert_eq!(from, SocketAddr::new(server_ip, 5353));
    // Port 0 of the stack: a free port of the dynamic range. A second
    // socket takes another port.
    assert!(local.port() >= 49152, "{local}");
    let other = dialer.datagrams().await.unwrap();
    assert!(other.local_addr().unwrap().port() >= 49152);
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
    assert!(PodRule::parse("country:NL").is_err());
}

// T-328: a pod rule of a port, and of a host and a port.
#[test]
fn rules_port_pod_rule_matches_the_port_of_the_target() {
    let ssh = PodRule::parse("port:22").unwrap();
    assert!(ssh.matches(&Target::new("git.corp.test", 22)));
    assert!(ssh.matches(&Target::new("10.1.2.3", 22)));
    assert!(!ssh.matches(&Target::new("10.1.2.3", 23)));
    let range = PodRule::parse("port:8000-8010").unwrap();
    assert!(range.matches(&Target::new("a.test", 8010)));
    assert!(!range.matches(&Target::new("a.test", 8011)));
    let db = PodRule::parse("tcp:127.0.0.1:5432").unwrap();
    assert!(db.matches(&Target::new("127.0.0.1", 5432)));
    assert!(!db.matches(&Target::new("127.0.0.1", 5433)));
    assert!(!db.matches(&Target::new("localhost", 5432)));
    let named = PodRule::parse("tcp:db.corp.test:5432").unwrap();
    assert!(named.matches(&Target::new("db.corp.test", 5432)));
    assert!(!named.matches(&Target::new("db.corp.test", 5433)));
    assert!(PodRule::parse("tcp:127.0.0.1").is_err());
    // A datagram flow has no port for a rule: port and tcp rules miss it.
    let udp = Target::new("127.0.0.1", 5432);
    assert!(!db.matches_datagram(&udp));
    assert!(!PodRule::parse("port:5432").unwrap().matches_datagram(&udp));
    assert!(PodRule::parse("cidr:127.0.0.0/8")
        .unwrap()
        .matches_datagram(&udp));
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

/// T-096: a group of two SOCKS5 upstreams on loopback (unix sockets): the
/// flows go to both, and with each member down no upstream gets a flow.
#[tokio::test]
async fn router_group_splits_two_socks5_upstreams_and_fails_closed() {
    use podnet_core::network::{Balance, Health, Member, Way};
    let dir = tempfile::tempdir().unwrap();
    let (pa, pb) = (dir.path().join("a.sock"), dir.path().join("b.sock"));
    let (seen_a, seen_b) = (fake_socks5(&pa), fake_socks5(&pb));
    let way = |path: &Path| {
        let spec = format!("unix:{}", path.display());
        Way::destination(Arc::new(UpstreamChain::parse(&[spec.as_str()]).unwrap()))
    };
    let group = Balance::new(
        "egress",
        vec![Member::new("a", way(&pa), 1), Member::new("b", way(&pb), 1)],
    )
    .unwrap()
    .with_seed(11);
    for i in 0..20 {
        let mut conn = group
            .dial(&Target::new(&format!("h{i}.example"), 443), soon())
            .await
            .unwrap();
        ping(&mut conn).await;
    }
    let (a, b) = (seen_a.lock().unwrap().len(), seen_b.lock().unwrap().len());
    assert_eq!(a + b, 20);
    assert!(a > 0 && b > 0, "a {a}, b {b}");
    group.set_health(0, Health::Down);
    group.set_health(1, Health::Down);
    let err = group
        .dial(&Target::new("late.example", 443), soon())
        .await
        .expect_err("the kill switch");
    assert_eq!(err.class, Class::Policy);
    let (a2, b2) = (seen_a.lock().unwrap().len(), seen_b.lock().unwrap().len());
    assert_eq!((a2, b2), (a, b), "no upstream got the refused flow");
}

/// T-098: the rules of the router send each target to its network or
/// group, refuse with `reject`, and follow a change of the file with no
/// restart. Unix sockets only (the job restricted-host).
#[tokio::test]
async fn router_rules_pick_the_network() {
    use std::collections::HashMap;

    use podnet_core::network::{Balance, Member, Way};

    use crate::routes::RulesFile;

    let dir = tempfile::tempdir().unwrap();
    let (pw, pr, pd) = (
        dir.path().join("warp.sock"),
        dir.path().join("relay.sock"),
        dir.path().join("direct.sock"),
    );
    let (seen_w, seen_r, seen_d) = (fake_socks5(&pw), fake_socks5(&pr), fake_socks5(&pd));
    let way = |path: &Path| {
        let spec = format!("unix:{}", path.display());
        Way::destination(Arc::new(UpstreamChain::parse(&[spec.as_str()]).unwrap()))
    };
    let group = Balance::new(
        "both",
        vec![Member::new("w", way(&pw), 1), Member::new("r", way(&pr), 1)],
    )
    .unwrap();
    let mut ways = HashMap::new();
    ways.insert("warp".to_string(), way(&pw));
    ways.insert("relay".to_string(), way(&pr));
    ways.insert("both".to_string(), Way::destination(Arc::new(group)));
    let file = dir.path().join("rules");
    std::fs::write(
        &file,
        "# the rules\ndomain:.corp.test=relay\ncidr:10.9.0.0/16=warp\ndomain:.ads.test=reject\ndomain:.many.test=both\n",
    )
    .unwrap();
    let rules = RulesFile::open(&file, ways.clone(), Some(way(&pd))).unwrap();
    for host in ["git.corp.test", "10.9.1.1", "x.example", "a.many.test"] {
        let mut conn = rules.dial(&Target::new(host, 443), soon()).await.unwrap();
        ping(&mut conn).await;
    }
    assert!(seen_r
        .lock()
        .unwrap()
        .contains(&"git.corp.test:443".to_string()));
    assert!(!seen_w
        .lock()
        .unwrap()
        .contains(&"git.corp.test:443".to_string()));
    assert!(seen_w.lock().unwrap().contains(&"10.9.1.1:443".to_string()));
    assert_eq!(*seen_d.lock().unwrap(), ["x.example:443"]);
    let both = seen_w.lock().unwrap().len() + seen_r.lock().unwrap().len();
    assert_eq!(both, 3, "the group took a.many.test through one member");
    let many = |seen: &Arc<Mutex<Vec<String>>>| {
        seen.lock()
            .unwrap()
            .iter()
            .filter(|t| *t == "a.many.test:443")
            .count()
    };
    assert_eq!(many(&seen_w) + many(&seen_r), 1);
    let err = rules
        .dial(&Target::new("t.ads.test", 443), soon())
        .await
        .expect_err("reject");
    assert_eq!(err.class, Class::Policy);

    // A change of the file: the next flow reads it.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    std::fs::write(&file, "domain:.corp.test=warp\n").unwrap();
    let mut conn = rules
        .dial(&Target::new("new.corp.test", 443), soon())
        .await
        .unwrap();
    ping(&mut conn).await;
    assert!(seen_w
        .lock()
        .unwrap()
        .contains(&"new.corp.test:443".to_string()));
    // A broken file keeps the last rules.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    std::fs::write(&file, "domain:.corp.test=nowhere\n").unwrap();
    let mut conn = rules
        .dial(&Target::new("again.corp.test", 443), soon())
        .await
        .unwrap();
    ping(&mut conn).await;
    assert!(seen_w
        .lock()
        .unwrap()
        .contains(&"again.corp.test:443".to_string()));
    // A rule to an unknown way is an error at the start.
    let bad = dir.path().join("bad");
    std::fs::write(&bad, "domain:.x.test=nowhere\n").unwrap();
    let err = RulesFile::open(&bad, ways, None).err().unwrap();
    assert!(
        format!("{err:#}").contains("is not a network or a group"),
        "{err:#}"
    );
}

/// T-242: a rule of the router file that an earlier rule shadows. A
/// shadowed `reject` is an error; a shadowed way is a warning only.
#[test]
fn router_shadowed_reject_is_an_error() {
    use std::collections::HashMap;

    use podnet_core::network::Way;

    let way = Way::destination(Arc::new(
        UpstreamChain::parse(&["socks5://127.0.0.1:1"]).unwrap(),
    ));
    let mut ways = HashMap::new();
    ways.insert("warp".to_string(), way);
    let bad = "domain:.example.com=warp\n# the rule below never matches\ndomain:admin.example.com=reject\n";
    let err = crate::routes::parse(bad, &ways, None).err().unwrap();
    let text = format!("{err:#}");
    assert!(text.contains("line 3") && text.contains("line 1"), "{text}");
    assert!(crate::routes::parse_with(bad, &ways, None, true).is_ok());
    // The reverse order, and a shadowed rule that is not a reject.
    let good = "domain:admin.example.com=reject\ndomain:.example.com=warp\n";
    assert!(crate::routes::parse(good, &ways, None).is_ok());
    let warn = "domain:.example.com=reject\ndomain:a.example.com=warp\n";
    assert!(crate::routes::parse(warn, &ways, None).is_ok());
}
