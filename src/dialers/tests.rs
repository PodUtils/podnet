use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::{SinkExt as _, StreamExt as _};
use pod_core::{Found, Resolve, Source};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};
use tokio::time::Instant;
use tokio_tungstenite::tungstenite;

use super::*;
use crate::network::{recv_from, send_to, Connected, Target};
use crate::ws_relay::Relay;

fn soon() -> Instant {
    Instant::now() + Duration::from_secs(10)
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

/// An echo server on 127.0.0.1.
async fn echo_server() -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((s, _)) = listener.accept().await {
            tokio::spawn(echo(s));
        }
    });
    addr
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

#[tokio::test]
async fn dialer_direct_carries_tcp_and_udp_on_the_loopback() {
    crate::test_host::need_loopback!();
    let addr = echo_server().await;
    let direct = Direct;
    assert_eq!(direct.label(), "direct");
    assert!(!direct.names_inside());
    let mut conn = direct.dial(&Target::Addr(addr), soon()).await.unwrap();
    assert_eq!(conn.peer, Some(addr));
    ping(&mut conn).await;

    let far = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let datagrams = direct.datagrams().await.unwrap();
    send_to(&*datagrams, b"query", far.local_addr().unwrap())
        .await
        .unwrap();
    let mut buf = [0u8; 64];
    let (n, from) = tokio::time::timeout(Duration::from_secs(10), far.recv_from(&mut buf))
        .await
        .expect("the datagram in time")
        .unwrap();
    assert_eq!(&buf[..n], b"query");
    far.send_to(b"answer", from).await.unwrap();
    let (n, _) = tokio::time::timeout(Duration::from_secs(10), recv_from(&*datagrams, &mut buf))
        .await
        .expect("the answer in time")
        .unwrap();
    assert_eq!(&buf[..n], b"answer");
}

#[tokio::test]
async fn dialer_direct_gives_a_refusal_at_the_stage_connect() {
    crate::test_host::need_loopback!();
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = closed.local_addr().unwrap();
    drop(closed);
    let err = Direct
        .dial(&Target::Addr(addr), soon())
        .await
        .expect_err("nothing listens");
    assert_eq!(err.stage, Stage::Connect);
    assert!(err.to_string().contains(&addr.to_string()), "{err}");
}

/// A SOCKS5 proxy with no authentication on a Unix socket: it keeps the
/// target of each CONNECT, then echoes the bytes.
fn fake_socks5(path: &Path) -> Arc<Mutex<Vec<String>>> {
    let listener = tokio::net::UnixListener::bind(path).unwrap();
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
async fn dialer_proxy_sends_the_name_to_a_socks5h_proxy() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("socks.sock");
    let seen = fake_socks5(&path);
    let spec = format!("socks5h+unix:{}", path.display());
    let dial = proxy(crate::egress::parse(&spec).unwrap());
    assert!(dial.names_inside());
    assert!(dial.label().contains("socks.sock"), "{}", dial.label());
    let mut conn = dial
        .dial(&Target::new("svc.example", 443), soon())
        .await
        .unwrap();
    ping(&mut conn).await;
    assert_eq!(*seen.lock().unwrap(), ["svc.example:443"]);
    let err = dial.datagrams().await.expect_err("TCP only");
    assert_eq!(err.class, Class::Unavailable);
}

/// A relay on 127.0.0.1 with the protocol of `/connect/IP/PORT`: it opens
/// TCP to the address of the path and copies the bytes.
#[allow(clippy::result_large_err)] // the callback type of tungstenite
async fn fake_relay() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            tokio::spawn(async move {
                let path = Arc::new(Mutex::new(String::new()));
                let seen = path.clone();
                let check =
                    move |request: &tungstenite::handshake::server::Request,
                          response: tungstenite::handshake::server::Response| {
                        *seen.lock().unwrap() = request.uri().path().to_string();
                        Ok(response)
                    };
                let Ok(mut ws) = tokio_tungstenite::accept_hdr_async(socket, check).await else {
                    return;
                };
                let path = path.lock().unwrap().clone();
                let mut parts = path.trim_start_matches("/connect/").split('/');
                let ip: IpAddr = parts.next().unwrap().parse().unwrap();
                let port: u16 = parts.next().unwrap().parse().unwrap();
                let mut tcp = tokio::net::TcpStream::connect((ip, port)).await.unwrap();
                let mut buf = vec![0u8; 16 * 1024];
                loop {
                    tokio::select! {
                        message = ws.next() => match message {
                            Some(Ok(tungstenite::Message::Binary(bytes))) => {
                                if tcp.write_all(&bytes).await.is_err() { return; }
                            }
                            Some(Ok(_)) => {}
                            _ => return,
                        },
                        n = tcp.read(&mut buf) => match n {
                            Ok(0) | Err(_) => return,
                            Ok(n) => {
                                let frame = tungstenite::Message::Binary(buf[..n].to_vec().into());
                                if ws.send(frame).await.is_err() { return; }
                            }
                        },
                    }
                }
            });
        }
    });
    url
}

/// A resolver with a table of names.
struct Table(Vec<(&'static str, IpAddr)>);

impl Resolve for Table {
    fn label(&self) -> String {
        "table".into()
    }

    fn resolve_from<'a>(
        &'a self,
        host: &'a str,
        port: u16,
        _deadline: Instant,
        _after: Option<Source>,
    ) -> crate::network::BoxFuture<'a, Result<Found, Failure>> {
        Box::pin(async move {
            let addrs: Vec<SocketAddr> = self
                .0
                .iter()
                .filter(|(name, _)| *name == host)
                .map(|(_, ip)| SocketAddr::new(*ip, port))
                .collect();
            Ok(Found {
                addrs,
                source: Source {
                    rank: 0,
                    name: "table",
                },
                more: false,
            })
        })
    }
}

#[tokio::test]
async fn dialer_relay_carries_tcp_to_an_address_and_to_a_name() {
    crate::test_host::need_loopback!();
    let echo = echo_server().await;
    let url = fake_relay().await;
    let relay = Arc::new(Relay::new(&url, Some("good".into())).unwrap());
    let table = Table(vec![
        ("echo.test", IpAddr::V6(Ipv6Addr::LOCALHOST)),
        ("echo.test", IpAddr::V4(Ipv4Addr::LOCALHOST)),
        ("v6.test", IpAddr::V6(Ipv6Addr::LOCALHOST)),
    ]);
    let dial = RelayDialer::new(relay, Arc::new(table));
    assert_eq!(dial.label(), format!("relay {url}"));
    assert!(!dial.names_inside());

    let mut conn = dial.dial(&Target::Addr(echo), soon()).await.unwrap();
    assert_eq!(conn.peer, Some(echo));
    ping(&mut conn).await;
    let mut conn = dial
        .dial(&Target::new("echo.test", echo.port()), soon())
        .await
        .unwrap();
    ping(&mut conn).await;

    let err = dial
        .dial(&Target::new("v6.test", echo.port()), soon())
        .await
        .expect_err("no IPv4 address");
    assert_eq!((err.stage, err.class), (Stage::Dns, Class::NotFound));
    let err = dial
        .dial(
            &Target::Addr(SocketAddr::new(Ipv6Addr::LOCALHOST.into(), 80)),
            soon(),
        )
        .await
        .expect_err("no IPv6 egress");
    assert_eq!(err.stage, Stage::Relay);
    assert!(err.to_string().contains("IPv6"), "{err}");
    let err = dial.datagrams().await.expect_err("TCP only");
    assert_eq!(err.class, Class::Unavailable);
}
