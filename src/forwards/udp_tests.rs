use std::net::SocketAddr;

use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};

use tokio::net::UdpSocket;

use super::*;
use crate::dialers::Direct;
use crate::network::{BoxFuture, Connected, Datagrams, Failure};

/// An echo server that adds the port of the sender to each answer.
async fn echo() -> SocketAddr {
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = socket.local_addr().unwrap();
    tokio::spawn(async move {
        let mut buf = [0u8; 2048];
        while let Ok((n, from)) = socket.recv_from(&mut buf).await {
            let mut back = format!("{}:", from.port()).into_bytes();
            back.extend_from_slice(&buf[..n]);
            let _ = socket.send_to(&back, from).await;
        }
    });
    addr
}

async fn ask(client: &UdpSocket, text: &[u8]) -> Vec<u8> {
    client.send(text).await.unwrap();
    let mut buf = [0u8; 2048];
    let n = tokio::time::timeout(Duration::from_secs(10), client.recv(&mut buf))
        .await
        .expect("an answer in time")
        .unwrap();
    buf[..n].to_vec()
}

/// T-099, the Prove: a datagram to the listener goes through the network
/// to the target, the answer comes back to the same client, and each
/// client has its own flow.
#[tokio::test]
async fn forward_udp_echo() {
    crate::test_host::need_loopback!();
    let far = echo().await;
    let listener = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
    let near = listener.local_addr().unwrap();
    tokio::spawn(serve_udp(listener, Arc::new(Direct), Target::Addr(far)));
    let mut ports = Vec::new();
    for text in [&b"one"[..], b"two"] {
        let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        client.connect(near).await.unwrap();
        for round in 0..2 {
            let back = ask(&client, text).await;
            let (port, body) = std::str::from_utf8(&back).unwrap().split_once(':').unwrap();
            assert_eq!(body.as_bytes(), text, "round {round}");
            if round == 0 {
                ports.push(port.to_string());
            } else {
                assert_eq!(port, ports.last().unwrap(), "one flow for one client");
            }
        }
    }
    assert_ne!(ports[0], ports[1], "one flow for each client");
}

/// A flow that is idle ends, and its place is free.
#[tokio::test]
async fn an_idle_udp_flow_ends() {
    crate::test_host::need_loopback!();
    let far = echo().await;
    let listener = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
    let near = listener.local_addr().unwrap();
    let flows = Flows::new(Duration::from_millis(100), 1);
    let count = flows.clone();
    tokio::spawn(serve_udp_with(
        listener,
        Arc::new(Direct),
        Target::Addr(far),
        flows,
    ));
    let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    client.connect(near).await.unwrap();
    ask(&client, b"x").await;
    assert_eq!(count.len(), 1);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(count.len(), 0);
    ask(&client, b"y").await;
}

/// A network that counts the TCP dials and the datagram sockets.
struct Counted(Direct, AtomicUsize, AtomicUsize);

impl Dial for Counted {
    fn label(&self) -> String {
        "counted".into()
    }

    fn dial<'a>(
        &'a self,
        target: &'a Target,
        deadline: tokio::time::Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        self.1.fetch_add(1, SeqCst);
        self.0.dial(target, deadline)
    }

    fn datagrams<'a>(&'a self) -> BoxFuture<'a, Result<Arc<dyn Datagrams>, Failure>> {
        self.2.fetch_add(1, SeqCst);
        self.0.datagrams()
    }
}

/// An upstream that answers a query on UDP and on TCP (one port): the
/// query with the response bit set.
async fn resolver() -> SocketAddr {
    loop {
        let tcp = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = tcp.local_addr().unwrap();
        let Ok(udp) = UdpSocket::bind(addr).await else {
            continue;
        };
        tokio::spawn(async move {
            let mut buf = [0u8; 512];
            while let Ok((n, from)) = udp.recv_from(&mut buf).await {
                buf[2] |= 0x80;
                let _ = udp.send_to(&buf[..n], from).await;
            }
        });
        tokio::spawn(async move {
            while let Ok((mut s, _)) = tcp.accept().await {
                tokio::spawn(async move {
                    let len = s.read_u16().await.unwrap() as usize;
                    let mut q = vec![0u8; len];
                    s.read_exact(&mut q).await.unwrap();
                    q[2] |= 0x80;
                    s.write_u16(len as u16).await.unwrap();
                    s.write_all(&q).await.unwrap();
                });
            }
        });
        return addr;
    }
}

/// T-099, the Prove: a DNS door answers a query on UDP and on TCP, and
/// each query goes through the network.
#[tokio::test]
async fn dns_door_over_a_network() {
    crate::test_host::need_loopback!();
    let upstream = resolver().await;
    let door = loop {
        let probe = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let addr = probe.local_addr().unwrap();
        if tokio::net::TcpListener::bind(addr).await.is_ok() {
            break addr;
        }
    };
    let net = Arc::new(Counted(Direct, Default::default(), Default::default()));
    tokio::spawn(serve_dns(door, net.clone(), Target::Addr(upstream)));
    let query = [
        0x12u8, 0x34, 0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0, 1, b'a', 0, 0, 1, 0, 1,
    ];
    let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    client.connect(door).await.unwrap();
    // The door starts in a task: send again until it answers.
    let mut answer = None;
    for _ in 0..50 {
        client.send(&query).await.unwrap();
        let mut buf = [0u8; 512];
        if let Ok(Ok(n)) =
            tokio::time::timeout(Duration::from_millis(200), client.recv(&mut buf)).await
        {
            answer = Some(buf[..n].to_vec());
            break;
        }
    }
    let answer = answer.expect("a UDP answer");
    assert_eq!(&answer[..2], &query[..2]);
    assert_eq!(answer[2] & 0x80, 0x80, "the response bit");
    assert_eq!(net.2.load(SeqCst), 1, "the UDP query used the network");
    let mut tcp = tokio::net::TcpStream::connect(door).await.unwrap();
    tcp.write_u16(query.len() as u16).await.unwrap();
    tcp.write_all(&query).await.unwrap();
    let len = tokio::time::timeout(Duration::from_secs(10), tcp.read_u16())
        .await
        .unwrap()
        .unwrap() as usize;
    let mut back = vec![0u8; len];
    tcp.read_exact(&mut back).await.unwrap();
    assert_eq!(back[2] & 0x80, 0x80);
    assert_eq!(net.1.load(SeqCst), 1, "the TCP query used the network");
}

/// T-404: the door answers a name of its table itself, on UDP and on TCP,
/// with no query through the network; another name goes through it.
#[tokio::test]
async fn dns_door_answers_its_names() {
    crate::test_host::need_loopback!();
    let upstream = resolver().await;
    let door = loop {
        let probe = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let addr = probe.local_addr().unwrap();
        if tokio::net::TcpListener::bind(addr).await.is_ok() {
            break addr;
        }
    };
    let names = super::super::aliases(&["svc.internal=10.0.0.5:22".to_string()]).unwrap();
    let table = Arc::new(super::super::alias_table(&names));
    let net = Arc::new(Counted(Direct, Default::default(), Default::default()));
    tokio::spawn(serve_dns_with(
        door,
        net.clone(),
        Target::Addr(upstream),
        Some(table),
    ));
    // A query of svc.internal, type A.
    let mut query = vec![0x56u8, 0x78, 0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0];
    for label in ["svc", "internal"] {
        query.push(label.len() as u8);
        query.extend_from_slice(label.as_bytes());
    }
    query.extend_from_slice(&[0, 0, 1, 0, 1]);
    let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    client.connect(door).await.unwrap();
    let mut answer = None;
    for _ in 0..50 {
        client.send(&query).await.unwrap();
        let mut buf = [0u8; 512];
        if let Ok(Ok(n)) =
            tokio::time::timeout(Duration::from_millis(200), client.recv(&mut buf)).await
        {
            answer = Some(buf[..n].to_vec());
            break;
        }
    }
    let answer = answer.expect("a UDP answer");
    assert_eq!(&answer[answer.len() - 4..], &[127, 0, 0, 2]);
    let mut tcp = tokio::net::TcpStream::connect(door).await.unwrap();
    tcp.write_u16(query.len() as u16).await.unwrap();
    tcp.write_all(&query).await.unwrap();
    let len = tokio::time::timeout(Duration::from_secs(10), tcp.read_u16())
        .await
        .unwrap()
        .unwrap() as usize;
    let mut back = vec![0u8; len];
    tcp.read_exact(&mut back).await.unwrap();
    assert_eq!(&back[back.len() - 4..], &[127, 0, 0, 2]);
    assert_eq!(
        net.1.load(SeqCst) + net.2.load(SeqCst),
        0,
        "the names need no network"
    );
    // Another name goes through the network.
    let other = [
        0x12u8, 0x34, 0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0, 1, b'a', 0, 0, 1, 0, 1,
    ];
    client.send(&other).await.unwrap();
    let mut buf = [0u8; 512];
    tokio::time::timeout(Duration::from_secs(10), client.recv(&mut buf))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(net.2.load(SeqCst), 1, "the other name used the network");
}

/// A network that carries no UDP and refuses each TCP dial.
struct NoWay;

impl Dial for NoWay {
    fn label(&self) -> String {
        "noway".into()
    }

    fn dial<'a>(
        &'a self,
        _target: &'a Target,
        _deadline: tokio::time::Instant,
    ) -> BoxFuture<'a, Result<Connected, Failure>> {
        Box::pin(async { Err(no_way("TCP")) })
    }

    fn datagrams<'a>(&'a self) -> BoxFuture<'a, Result<Arc<dyn Datagrams>, Failure>> {
        Box::pin(async { Err(no_way("UDP")) })
    }
}

fn no_way(what: &str) -> Failure {
    use crate::network::{Class, Stage};
    Failure::new(
        Stage::Connect,
        Class::Unavailable,
        format!("noway carries no {what}"),
    )
}

/// GitHub #638: a DNS door whose upstream cannot be reached answers
/// SERVFAIL on UDP and on TCP, not a silent timeout. A second door on the
/// same address exits 69 at the bind.
#[tokio::test]
async fn dns_door_answers_servfail_when_the_upstream_cannot_be_reached() {
    crate::test_host::need_loopback!();
    let (udp, tcp) = bind_dns("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let door = udp.local_addr().unwrap();
    let err = bind_dns(door).await.unwrap_err();
    assert_eq!(
        crate::exit::code_of(&err),
        crate::exit::UNAVAILABLE,
        "{err:#}"
    );
    let names = super::super::aliases(&["svc.internal=10.0.0.5:22".to_string()]).unwrap();
    let table = Arc::new(super::super::alias_table(&names));
    let upstream = Target::Addr("192.0.2.1:53".parse().unwrap());
    tokio::spawn(serve_dns_on(
        udp,
        tcp,
        Arc::new(NoWay),
        upstream,
        Some(table),
    ));
    let query = [
        0x12u8, 0x34, 0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0, 1, b'a', 0, 0, 1, 0, 1,
    ];
    let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    client.connect(door).await.unwrap();
    let answer = ask(&client, &query).await;
    assert_eq!(&answer[..2], &query[..2]);
    assert_eq!(answer[2] & 0x80, 0x80, "the response bit");
    assert_eq!(answer[3] & 0x0f, 2, "SERVFAIL");
    let mut tcp = tokio::net::TcpStream::connect(door).await.unwrap();
    tcp.write_u16(query.len() as u16).await.unwrap();
    tcp.write_all(&query).await.unwrap();
    let len = tokio::time::timeout(Duration::from_secs(10), tcp.read_u16())
        .await
        .unwrap()
        .unwrap() as usize;
    let mut back = vec![0u8; len];
    tcp.read_exact(&mut back).await.unwrap();
    assert_eq!(back[3] & 0x0f, 2, "SERVFAIL on TCP");
}
