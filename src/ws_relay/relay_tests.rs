//! Tests of the relay client over `pod-relay` (T-162, T-163): the order of
//! the hosts after a stalled host, and the budget under the limits of a
//! pool document. The relays are stand-ins on the loopback.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pod_relay::pool::Limits;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};

use super::*;

const TOKEN: &str = "testonly-token-0123456789";
/// The stagger of `pod-relay` before it starts the next host.
const STAGGER: Duration = Duration::from_secs(2);

/// A server that answers each byte with the same byte.
async fn echo_server() -> SocketAddr {
    let echo = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let at = echo.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = echo.accept().await {
            tokio::spawn(async move {
                let (mut r, mut w) = socket.split();
                let _ = tokio::io::copy(&mut r, &mut w).await;
            });
        }
    });
    at
}

/// A stand-in relay at `ip` that takes the forward upgrade with [`TOKEN`],
/// opens TCP to `target` and copies the bytes. While `stalled` is set, it
/// accepts the TCP connection and says nothing. Its port.
async fn stand_in(ip: &str, target: SocketAddr, stalled: Arc<AtomicBool>) -> u16 {
    let relay = TcpListener::bind(format!("{ip}:0")).await.unwrap();
    let port = relay.local_addr().unwrap().port();
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((socket, _)) = relay.accept().await {
            if stalled.load(Ordering::SeqCst) {
                held.push(socket);
            } else {
                tokio::spawn(serve(socket, target));
            }
        }
    });
    port
}

async fn serve(mut socket: TcpStream, target: SocketAddr) {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if socket.read_exact(&mut byte).await.is_err() {
            return;
        }
        head.push(byte[0]);
    }
    let head = String::from_utf8_lossy(&head).to_string();
    let header = |name: &str| {
        head.lines().find_map(|l| {
            l.split_once(':')
                .filter(|(n, _)| n.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.trim())
        })
    };
    let (Some(key), Some(TOKEN)) = (header("sec-websocket-key"), header("x-relay-token")) else {
        let _ = socket
            .write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await;
        return;
    };
    socket
        .write_all(&pod_ws::server::switching(key))
        .await
        .unwrap();
    let session = pod_ws::server::session(socket, Vec::new(), None);
    let (mut tcp_r, mut tcp_w) = TcpStream::connect(target).await.unwrap().into_split();
    let session = &session;
    let to_target = async {
        while let Ok(message) = session.read_frame().await {
            if message.opcode != pod_ws::frame::OPCODE_BINARY
                || session.send_binary(&[]).await.is_err()
            {
                break;
            }
            if tcp_w.write_all(&message.payload).await.is_err() {
                break;
            }
        }
    };
    let from_target = async {
        let mut buf = vec![0u8; 64 * 1024];
        while let Ok(n @ 1..) = tcp_r.read(&mut buf).await {
            if session.send_binary(&buf[..n]).await.is_err() {
                break;
            }
        }
    };
    tokio::select! { _ = to_target => {}, _ = from_target => {} }
}

/// A relay of the hosts `urls` in this order, with its own cache directory
/// and the token [`TOKEN`].
fn relay_of(urls: &[String], cache: Cache) -> Relay {
    let list = urls.join(",");
    let (hosts, plain) = parse(&list).unwrap();
    assert!(plain);
    Relay {
        list: RelayList {
            hosts,
            explicit: true,
        },
        plain,
        ctx: context_in(cache).with_settings(settings(Some(TOKEN.into()))),
    }
}

/// Open a session to `echo` through `relay`, check one echo, and give the
/// time of the open.
async fn timed_connect(relay: &Relay, echo: SocketAddr) -> Duration {
    let started = Instant::now();
    let mut io = relay
        .connect(echo, Duration::from_secs(20))
        .await
        .expect("a session through the relay");
    let took = started.elapsed();
    io.write_all(b"ping").await.unwrap();
    let mut got = [0u8; 4];
    tokio::time::timeout(Duration::from_secs(10), io.read_exact(&mut got))
        .await
        .expect("the echo in time")
        .unwrap();
    assert_eq!(&got, b"ping");
    took
}

/// The relay `a` stalls and `b` carries: a connect pays the stagger for `a`
/// once. The next connect tries `b` first and pays nothing (T-162).
#[tokio::test(flavor = "multi_thread")]
async fn a_stalled_relay_host_is_tried_last_on_the_next_connect() {
    crate::test_host::need_loopback!();
    let echo = echo_server().await;
    let a_stalls = Arc::new(AtomicBool::new(true));
    let a = stand_in("127.0.0.1", echo, a_stalls.clone()).await;
    let b = stand_in("127.0.0.2", echo, Arc::new(AtomicBool::new(false))).await;
    let dir = tempfile::tempdir().unwrap();
    let relay = relay_of(
        &[
            format!("http://127.0.0.1:{a}"),
            format!("http://127.0.0.2:{b}"),
        ],
        Cache::in_dirs(vec![dir.path().to_path_buf()]),
    );

    let first = timed_connect(&relay, echo).await;
    assert!(first >= STAGGER - Duration::from_millis(100), "{first:?}");
    let second = timed_connect(&relay, echo).await;
    assert!(
        second < STAGGER / 2,
        "the stalled host went last: {second:?}"
    );
}

/// A host that lost is restored when it opens: `a` stalls and loses, then
/// `a` answers and `b` stalls. The connect that follows pays the stagger for
/// `b` and `a` carries it; the next connect goes to `a` first (T-162).
#[tokio::test(flavor = "multi_thread")]
async fn a_demoted_relay_host_recovers_on_success() {
    crate::test_host::need_loopback!();
    let echo = echo_server().await;
    let (a_stalls, b_stalls) = (
        Arc::new(AtomicBool::new(true)),
        Arc::new(AtomicBool::new(false)),
    );
    let a = stand_in("127.0.0.1", echo, a_stalls.clone()).await;
    let b = stand_in("127.0.0.2", echo, b_stalls.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    let relay = relay_of(
        &[
            format!("http://127.0.0.1:{a}"),
            format!("http://127.0.0.2:{b}"),
        ],
        Cache::in_dirs(vec![dir.path().to_path_buf()]),
    );
    timed_connect(&relay, echo).await;
    assert!(timed_connect(&relay, echo).await < STAGGER / 2, "b leads");

    // The roles change: b stalls, a answers.
    a_stalls.store(false, Ordering::SeqCst);
    b_stalls.store(true, Ordering::SeqCst);
    let third = timed_connect(&relay, echo).await;
    assert!(third >= STAGGER - Duration::from_millis(100), "{third:?}");
    let fourth = timed_connect(&relay, echo).await;
    assert!(fourth < STAGGER / 2, "a leads again: {fourth:?}");
}

/// A cache with the pool document of the host `127.0.0.1` and `limits`.
fn cache_with_limits(dir: &std::path::Path, limits: Option<&str>) -> Cache {
    let cache = Cache::in_dirs(vec![dir.to_path_buf()]);
    let limits = limits.map_or(String::new(), |l| format!(r#","limits":{l}"#));
    let body = format!(r#"{{"fetched_ms":{},"hosts":[]{limits}}}"#, now_ms());
    cache
        .store_file(&pod_relay::pool::file_name("127.0.0.1"), body.as_bytes())
        .unwrap();
    cache
}

/// A session through a stand-in relay at `127.0.0.1` with `cache`, then the
/// budget that [`meter_in`] gives for the context of that relay.
async fn budget_after_a_session(cache: Cache) -> (BoxedIo, RelayBudget) {
    let echo = echo_server().await;
    let port = stand_in("127.0.0.1", echo, Arc::new(AtomicBool::new(false))).await;
    let relay = relay_of(&[format!("http://127.0.0.1:{port}")], cache);
    drop(
        relay
            .connect(echo, Duration::from_secs(20))
            .await
            .expect("a session"),
    );
    meter_in(
        relay.ctx(),
        Box::new(tokio::io::join(tokio::io::empty(), tokio::io::sink())),
    )
}

/// Write `bytes` to `io` in blocks.
async fn write_bytes(io: &mut BoxedIo, mut bytes: u64) {
    let block = vec![0u8; 1 << 20];
    while bytes > 0 {
        let n = bytes.min(block.len() as u64) as usize;
        io.write_all(&block[..n]).await.unwrap();
        bytes -= n as u64;
    }
}

/// A relay that publishes smaller caps than the constants gives a smaller
/// budget, with the margins of the constants: 4 MiB under the byte cap, 30
/// min under the time cap (T-163).
#[tokio::test(flavor = "multi_thread")]
async fn a_pool_with_limits_sets_the_budget() {
    crate::test_host::need_loopback!();
    let dir = tempfile::tempdir().unwrap();
    let cache = cache_with_limits(
        dir.path(),
        Some(r#"{"max_session_bytes":16777216,"max_session_ms":7200000}"#),
    );
    let (mut io, budget) = budget_after_a_session(cache).await;
    write_bytes(&mut io, (12 << 20) - 1).await;
    assert!(!budget.spent(), "12 MiB less 1 byte of 12 MiB");
    write_bytes(&mut io, 1).await;
    assert!(budget.spent(), "12 MiB is the budget under a cap of 16 MiB");

    let (_io, budget) = budget_after_a_session(cache_with_limits(
        dir.path(),
        Some(r#"{"max_session_ms":7200000}"#),
    ))
    .await;
    assert!(!budget.spent());
}

/// A relay that publishes no limits, or larger ones, keeps the constants
/// (T-163).
#[tokio::test(flavor = "multi_thread")]
async fn a_pool_with_no_limits_keeps_the_constants() {
    crate::test_host::need_loopback!();
    for limits in [
        None,
        Some(r#"{"max_session_bytes":1073741824,"max_session_ms":86400000}"#),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let (mut io, budget) = budget_after_a_session(cache_with_limits(dir.path(), limits)).await;
        write_bytes(&mut io, SESSION_BYTES - 1).await;
        assert!(!budget.spent(), "{limits:?}: the constant is 60 MiB");
        write_bytes(&mut io, 1).await;
        assert!(budget.spent(), "{limits:?}: spent at the constant");
    }
}

/// The age of the budget follows `max_session_ms`: 30 min under it, and the
/// constant of 11.5 h when the relay says nothing (T-163).
#[tokio::test(start_paused = true)]
async fn the_age_of_the_budget_follows_the_limits_of_the_relay() {
    let idle: fn() -> BoxedIo = || Box::new(tokio::io::join(tokio::io::empty(), tokio::io::sink()));
    let ctx = context_in(Cache::none());
    ctx.set_in_use(Limits {
        max_session_ms: Some(2 * 3600 * 1000),
        ..Limits::default()
    });
    let (_io, budget) = meter_in(&ctx, idle());
    tokio::time::advance(Duration::from_secs(89 * 60)).await;
    assert!(!budget.spent(), "89 min of 90 min");
    tokio::time::advance(Duration::from_secs(61)).await;
    assert!(budget.spent(), "spent 30 min under a cap of 2 h");

    let (_io, budget) = meter_in(&context_in(Cache::none()), idle());
    tokio::time::advance(SESSION_AGE - Duration::from_secs(1)).await;
    assert!(!budget.spent());
    tokio::time::advance(Duration::from_secs(1)).await;
    assert!(budget.spent(), "spent at the constant");
}

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis()
}
