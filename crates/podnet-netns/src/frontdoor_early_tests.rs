//! The tests of `--early-reply` through the door of `pod --serve` and a
//! SOCKS5 upstream on loopback (T-237).

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};

use super::*;

/// What the fake upstream does with a flow.
#[derive(Clone, Copy)]
struct Behaviour {
    /// The reply code of the request (0 is success).
    code: u8,
    /// Ask for the user and password method (the client offers it).
    auth: bool,
}

/// A SOCKS5 upstream that echoes the bytes of each flow.
async fn upstream(behaviour: Behaviour) -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let _ = serve(&mut stream, behaviour).await;
            });
        }
    });
    port
}

async fn serve(stream: &mut tokio::net::TcpStream, b: Behaviour) -> std::io::Result<()> {
    let mut head = [0u8; 2];
    stream.read_exact(&mut head).await?;
    let mut methods = vec![0u8; head[1] as usize];
    stream.read_exact(&mut methods).await?;
    if b.auth {
        assert!(methods.contains(&2), "the client offers the user method");
        stream.write_all(&[5, 2]).await?;
        // The frame of RFC 1929. A client that sent its request before the
        // method reply would break this frame.
        let mut ver = [0u8; 2];
        stream.read_exact(&mut ver).await?;
        let mut user = vec![0u8; ver[1] as usize];
        stream.read_exact(&mut user).await?;
        let plen = stream.read_u8().await? as usize;
        let mut pass = vec![0u8; plen];
        stream.read_exact(&mut pass).await?;
        stream.write_all(&[1, 0]).await?;
    } else {
        stream.write_all(&[5, 0]).await?;
    }
    let mut request = [0u8; 4];
    stream.read_exact(&mut request).await?;
    match request[3] {
        1 => {
            let mut rest = [0u8; 6];
            stream.read_exact(&mut rest).await?;
        }
        _ => {
            let len = stream.read_u8().await? as usize;
            let mut rest = vec![0u8; len + 2];
            stream.read_exact(&mut rest).await?;
        }
    }
    stream
        .write_all(&[5, b.code, 0, 1, 0, 0, 0, 0, 0, 0])
        .await?;
    if b.code != 0 {
        return Ok(());
    }
    let (mut read, mut write) = stream.split();
    tokio::io::copy(&mut read, &mut write).await?;
    Ok(())
}

/// A TCP forwarder that delays each direction by `delay` and keeps the
/// order and the pace of the chunks (a link of high latency).
async fn delayed(to: u16, delay: Duration) -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((near, _)) = listener.accept().await {
            let Ok(far) = tokio::net::TcpStream::connect(("127.0.0.1", to)).await else {
                continue;
            };
            let (near_read, near_write) = near.into_split();
            let (far_read, far_write) = far.into_split();
            tokio::spawn(pipe(near_read, far_write, delay));
            tokio::spawn(pipe(far_read, near_write, delay));
        }
    });
    port
}

async fn pipe(mut from: OwnedReadHalf, mut to: OwnedWriteHalf, delay: Duration) {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(Instant, Vec<u8>)>();
    tokio::spawn(async move {
        while let Some((at, data)) = rx.recv().await {
            tokio::time::sleep_until(at).await;
            if to.write_all(&data).await.is_err() {
                return;
            }
        }
        let _ = to.shutdown().await;
    });
    let mut buf = vec![0u8; 16 * 1024];
    while let Ok(n) = from.read(&mut buf).await {
        if n == 0
            || tx
                .send((Instant::now() + delay, buf[..n].to_vec()))
                .is_err()
        {
            return;
        }
    }
}

fn door(port: u16, early: bool, user: Option<&str>) -> Arc<Door> {
    let spec = match user {
        Some(user) => format!("socks5://{user}:pw@127.0.0.1:{port}"),
        None => format!("socks5://127.0.0.1:{port}"),
    };
    let chain = vec![Proxy::parse(&spec).unwrap()];
    Arc::new(front_door_with(chain, Vec::new(), false, early))
}

/// The client has done the greeting; the request is sent at once.
async fn connect(door: &Arc<Door>) -> (tokio::io::DuplexStream, Vec<u8>) {
    let (mut client, server) = tokio::io::duplex(1 << 16);
    door.clone().spawn(server);
    client.write_all(&[5, 1, 0]).await.unwrap();
    let mut method = [0u8; 2];
    client.read_exact(&mut method).await.unwrap();
    let host = "echo.example";
    let mut request = vec![5, 1, 0, 3, host.len() as u8];
    request.extend_from_slice(host.as_bytes());
    request.extend_from_slice(&80u16.to_be_bytes());
    client.write_all(&request).await.unwrap();
    let mut reply = vec![0u8; 10];
    client.read_exact(&mut reply).await.unwrap();
    (client, reply)
}

/// The time from the request of the client to the first echoed byte.
async fn first_echo(early: bool, delay: Duration) -> Duration {
    let port = delayed(
        upstream(Behaviour {
            code: 0,
            auth: false,
        })
        .await,
        delay,
    )
    .await;
    let door = door(port, early, None);
    let (mut client, server) = tokio::io::duplex(1 << 16);
    door.clone().spawn(server);
    client.write_all(&[5, 1, 0]).await.unwrap();
    let mut method = [0u8; 2];
    client.read_exact(&mut method).await.unwrap();
    let start = Instant::now();
    let host = "echo.example";
    let mut request = vec![5, 1, 0, 3, host.len() as u8];
    request.extend_from_slice(host.as_bytes());
    request.extend_from_slice(&80u16.to_be_bytes());
    client.write_all(&request).await.unwrap();
    let mut reply = [0u8; 10];
    client.read_exact(&mut reply).await.unwrap();
    assert_eq!(reply[1], 0);
    client.write_all(b"ping").await.unwrap();
    let mut echo = [0u8; 4];
    client.read_exact(&mut echo).await.unwrap();
    assert_eq!(&echo, b"ping");
    start.elapsed()
}

/// With a link of delay D in each direction, a flow without the option needs
/// the method reply and the reply of the request before the client sends
/// its first byte: 3 round trips to the first echo. With the option, the
/// client sends at once behind the request: 1 round trip.
#[tokio::test]
async fn early_reply_cuts_the_round_trips() {
    pod_testkit::need_loopback!();
    let delay = Duration::from_millis(100);
    let off = first_echo(false, delay).await;
    let on = first_echo(true, delay).await;
    // About 6 D against 2 D; the margin is for a slow host.
    assert!(
        off.as_secs_f64() > 2.0 * on.as_secs_f64(),
        "off {off:?}, on {on:?}"
    );
    assert!(on >= delay * 2 - Duration::from_millis(20), "on {on:?}");
}

/// An upstream that refuses with code 5: with the option off, the client
/// gets a failure reply; with it on, the client gets the success reply and
/// then a closed connection.
#[tokio::test]
async fn early_reply_shows_a_refusal_as_a_closed_connection() {
    pod_testkit::need_loopback!();
    let port = upstream(Behaviour {
        code: 5,
        auth: false,
    })
    .await;
    let (_, reply) = connect(&door(port, false, None)).await;
    assert_eq!(reply[0], 5);
    assert_ne!(reply[1], 0, "no early reply: the client gets an error code");

    let (mut client, reply) = connect(&door(port, true, None)).await;
    assert_eq!(reply[1], 0, "the early reply says yes");
    client.write_all(b"data").await.unwrap();
    let mut rest = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), client.read_to_end(&mut rest))
        .await
        .expect("the connection ends")
        .unwrap();
    assert!(rest.is_empty(), "no byte comes back: {rest:?}");
}

/// A rule that rejects a target keeps its real answer with the option on.
#[tokio::test]
async fn early_reply_keeps_the_pod_rule_refusal() {
    pod_testkit::need_loopback!();
    let port = upstream(Behaviour {
        code: 0,
        auth: false,
    })
    .await;
    let chain = vec![Proxy::parse(&format!("socks5://127.0.0.1:{port}")).unwrap()];
    let rules = vec![Rule::parse("domain:.ads.example=reject").unwrap()];
    let door = Arc::new(front_door_with(chain, rules, false, true));
    let (mut client, server) = tokio::io::duplex(4096);
    door.clone().spawn(server);
    client.write_all(&[5, 1, 0]).await.unwrap();
    let mut method = [0u8; 2];
    client.read_exact(&mut method).await.unwrap();
    let host = "x.ads.example";
    let mut request = vec![5, 1, 0, 3, host.len() as u8];
    request.extend_from_slice(host.as_bytes());
    request.extend_from_slice(&443u16.to_be_bytes());
    client.write_all(&request).await.unwrap();
    let mut reply = [0u8; 10];
    client.read_exact(&mut reply).await.unwrap();
    assert_eq!(reply[1], 0x02, "{reply:?}");
}

/// An upstream with a user and a password runs the serial handshake, and
/// the option changes nothing for it.
#[tokio::test]
async fn early_reply_runs_the_serial_path_for_a_user_hop() {
    pod_testkit::need_loopback!();
    let port = upstream(Behaviour {
        code: 0,
        auth: true,
    })
    .await;
    let (mut client, reply) = connect(&door(port, true, Some("user"))).await;
    assert_eq!(reply[1], 0);
    client.write_all(b"ping").await.unwrap();
    let mut echo = [0u8; 4];
    tokio::time::timeout(Duration::from_secs(5), client.read_exact(&mut echo))
        .await
        .expect("the echo comes")
        .unwrap();
    assert_eq!(&echo, b"ping");
}
