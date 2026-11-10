//! The reverse mode of the relay (issue 88) against a local fake relay:
//! two sessions at the same time through one node, a refused session, and
//! a node socket that the relay drops (the node opens it again).

mod test_host;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::{SinkExt as _, StreamExt as _};
use podnet::relay_reverse::{connect, node, run_node, save, Pair, Target};
use podnet::ws_relay::Relay;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::Message;

const NAME: &str = "testnode";

#[path = "relay_reverse/fake.rs"]
mod fake;
use fake::{fake_relay, State};

fn pair(url: &str) -> Pair {
    Pair {
        name: NAME.into(),
        node_token: "node".into(),
        connect_token: "connect".into(),
        stop_token: String::new(),
        expires: (std::time::SystemTime::now() + Duration::from_secs(3600))
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64,
        relay: url.into(),
    }
}

/// A TCP echo server; gives its address.
async fn echo() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            tokio::spawn(async move {
                let (mut r, mut w) = socket.split();
                let _ = tokio::io::copy(&mut r, &mut w).await;
            });
        }
    });
    addr
}

/// One operator session: send `data`, read the same bytes back.
async fn round_trip(relay: Arc<Relay>, data: Vec<u8>) -> Vec<u8> {
    let (ours, theirs) = tokio::io::duplex(1 << 20);
    let session = tokio::spawn(async move {
        let (read, write) = tokio::io::split(theirs);
        connect(&relay, NAME, "connect", read, write).await
    });
    let (mut read, mut write) = tokio::io::split(ours);
    let sent = data.clone();
    let writer = tokio::spawn(async move {
        write.write_all(&sent).await.unwrap();
        write
    });
    let mut back = vec![0u8; data.len()];
    tokio::time::timeout(Duration::from_secs(20), read.read_exact(&mut back))
        .await
        .expect("the bytes come back")
        .unwrap();
    let write = writer.await.unwrap();
    drop(write);
    drop(read);
    let _ = tokio::time::timeout(Duration::from_secs(10), session).await;
    back
}

/// A TCP target that reads one line, answers HELLO:<line> after 300 ms,
/// and closes.
async fn hello() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut line = Vec::new();
                let mut byte = [0u8; 1];
                while socket.read_exact(&mut byte).await.is_ok() {
                    line.push(byte[0]);
                    if byte[0] == b'\n' {
                        break;
                    }
                }
                tokio::time::sleep(Duration::from_millis(300)).await;
                let mut answer = b"HELLO:".to_vec();
                answer.extend_from_slice(&line);
                let _ = socket.write_all(&answer).await;
            });
        }
    });
    addr
}

// Issue 109: an input that ends at once (printf PING | podnet relay connect)
// still gets the answer of the target. Before, the close of the input
// closed the session, and the output was empty.
#[tokio::test(flavor = "multi_thread")]
async fn the_answer_comes_back_after_the_input_ends() {
    test_host::need_loopback!();
    let (url, state) = fake_relay(None).await;
    let target = Target::parse(&hello().await).unwrap();
    let node_relay = Relay::new(&url, None).unwrap();
    let node_pair = pair(&url);
    tokio::spawn(async move { node(&node_relay, &node_pair, &target).await });
    wait_for_node(&state, 1).await;
    let relay = Relay::new(&url, None).unwrap();
    let mut out = Vec::new();
    tokio::time::timeout(
        Duration::from_secs(20),
        connect(&relay, NAME, "connect", &b"PING\n"[..], &mut out),
    )
    .await
    .expect("the session ends")
    .unwrap();
    assert_eq!(out, b"HELLO:PING\n");
}

// Issue 109: a target that waits for the end of the input (it has no
// half-close) ends the session after EOF_IDLE.
#[tokio::test(flavor = "multi_thread")]
async fn a_session_ends_after_the_idle_time_when_the_input_ended() {
    test_host::need_loopback!();
    let (url, state) = fake_relay(None).await;
    let target = Target::parse(&echo().await).unwrap();
    let node_relay = Relay::new(&url, None).unwrap();
    let node_pair = pair(&url);
    tokio::spawn(async move { node(&node_relay, &node_pair, &target).await });
    wait_for_node(&state, 1).await;
    let relay = Relay::new(&url, None).unwrap();
    let mut out = Vec::new();
    let start = std::time::Instant::now();
    tokio::time::timeout(
        podnet::relay_reverse::EOF_IDLE + Duration::from_secs(10),
        connect(&relay, NAME, "connect", &b"PING\n"[..], &mut out),
    )
    .await
    .expect("the session ends")
    .unwrap();
    assert_eq!(out, b"PING\n");
    assert!(start.elapsed() >= podnet::relay_reverse::EOF_IDLE);
}

async fn wait_for_node(state: &Arc<Mutex<State>>, sockets: usize) {
    for _ in 0..200 {
        if state.lock().unwrap().node_sockets >= sockets {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("no node socket {sockets}");
}

#[tokio::test(flavor = "multi_thread")]
async fn two_sessions_at_the_same_time_through_one_node() {
    test_host::need_loopback!();
    let (url, state) = fake_relay(None).await;
    let target = Target::parse(&echo().await).unwrap();
    let node_relay = Relay::new(&url, None).unwrap();
    let node_pair = pair(&url);
    tokio::spawn(async move { node(&node_relay, &node_pair, &target).await });
    wait_for_node(&state, 1).await;
    let relay = Arc::new(Relay::new(&url, None).unwrap());
    let a: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    let b: Vec<u8> = (0..200_000u32).map(|i| (i % 13) as u8 + 100).collect();
    let (back_a, back_b) = tokio::join!(
        round_trip(relay.clone(), a.clone()),
        round_trip(relay.clone(), b.clone())
    );
    assert!(back_a == a, "session a");
    assert!(back_b == b, "session b");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_to_a_closed_port_is_refused_with_the_reason() {
    test_host::need_loopback!();
    let (url, state) = fake_relay(None).await;
    // A port with no listener.
    let closed = {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        listener.local_addr().unwrap().to_string()
    };
    let target = Target::parse(&closed).unwrap();
    let node_relay = Relay::new(&url, None).unwrap();
    let node_pair = pair(&url);
    tokio::spawn(async move { node(&node_relay, &node_pair, &target).await });
    wait_for_node(&state, 1).await;
    let relay = Relay::new(&url, None).unwrap();
    let err = connect(
        &relay,
        NAME,
        "connect",
        tokio::io::empty(),
        tokio::io::sink(),
    )
    .await
    .unwrap_err();
    let text = format!("{err:#}");
    assert!(text.contains("refused the session"), "{text}");
    assert!(
        text.contains(&closed),
        "the reason names the target: {text}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dropped_node_socket_is_opened_again() {
    test_host::need_loopback!();
    let (url, state) = fake_relay(Some(300)).await;
    let target = Target::parse(&echo().await).unwrap();
    let node_relay = Relay::new(&url, None).unwrap();
    let node_pair = pair(&url);
    tokio::spawn(async move { node(&node_relay, &node_pair, &target).await });
    // The relay drops the first socket; the node comes back in 1 to 2 s.
    wait_for_node(&state, 2).await;
    let relay = Arc::new(Relay::new(&url, None).unwrap());
    let data = b"after the reconnect".to_vec();
    assert_eq!(round_trip(relay, data.clone()).await, data);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_wrong_connect_token_is_a_config_error() {
    test_host::need_loopback!();
    let (url, _state) = fake_relay(None).await;
    let relay = Relay::new(&url, None).unwrap();
    let err = connect(&relay, NAME, "wrong", tokio::io::empty(), tokio::io::sink())
        .await
        .unwrap_err();
    assert_eq!(podnet::exit::code_of(&err), podnet::exit::CONFIG);
}

/// A relay that sends ready, part of an answer, then the close `code`.
async fn closing_relay(code: u16) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            tokio::spawn(async move {
                let Ok(mut ws) = tokio_tungstenite::accept_async(socket).await else {
                    return;
                };
                let _ = ws.send(Message::text(r#"{"type":"ready"}"#)).await;
                let _ = ws.send(Message::binary(b"part".to_vec())).await;
                let _ = ws
                    .send(Message::Close(Some(CloseFrame {
                        code: CloseCode::from(code),
                        reason: "limit".into(),
                    })))
                    .await;
                while ws.next().await.is_some() {}
            });
        }
    });
    url
}

// podssh #84: a close other than 1000 after the end of the input is an
// error (exit 69): the output can be cut. Before, it exited 0.
#[tokio::test(flavor = "multi_thread")]
async fn a_close_with_an_error_code_after_the_input_ends_is_an_error() {
    test_host::need_loopback!();
    let url = closing_relay(1009).await;
    let relay = Relay::new(&url, None).unwrap();
    let mut out = Vec::new();
    let err = tokio::time::timeout(
        Duration::from_secs(20),
        connect(&relay, NAME, "connect", &b""[..], &mut out),
    )
    .await
    .expect("the session ends")
    .unwrap_err();
    assert_eq!(out, b"part");
    assert_eq!(podnet::exit::code_of(&err), 69, "{err:#}");
    assert!(format!("{err:#}").contains("1009"), "{err:#}");
    // A normal close is a success.
    let url = closing_relay(1000).await;
    let relay = Relay::new(&url, None).unwrap();
    let mut out = Vec::new();
    connect(&relay, NAME, "connect", &b""[..], &mut out)
        .await
        .unwrap();
    assert_eq!(out, b"part");
}

/// A TCP target: the first connection is never read (it is held open), the
/// others are echoed.
async fn stuck_then_echo() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    tokio::spawn(async move {
        let mut first = true;
        while let Ok((mut socket, _)) = listener.accept().await {
            if std::mem::take(&mut first) {
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(120)).await;
                    drop(socket);
                });
                continue;
            }
            tokio::spawn(async move {
                let (mut r, mut w) = socket.split();
                let _ = tokio::io::copy(&mut r, &mut w).await;
            });
        }
    });
    addr
}

// podssh: a local side that does not read ends its own session. Before,
// the node waited for its queue, and each other session of the node
// stopped.
#[tokio::test(flavor = "multi_thread")]
async fn a_local_side_that_does_not_read_stops_no_other_session() {
    test_host::need_loopback!();
    let (url, state) = fake_relay(None).await;
    let target = Target::parse(&stuck_then_echo().await).unwrap();
    let node_relay = Relay::new(&url, None).unwrap();
    let node_pair = pair(&url);
    tokio::spawn(async move { node(&node_relay, &node_pair, &target).await });
    wait_for_node(&state, 1).await;
    let relay = Arc::new(Relay::new(&url, None).unwrap());
    // Session A goes to the connection that is never read.
    let flood = {
        let relay = relay.clone();
        tokio::spawn(async move {
            let input = vec![7u8; 48 << 20];
            let mut out = Vec::new();
            connect(&relay, NAME, "connect", &input[..], &mut out).await
        })
    };
    tokio::time::sleep(Duration::from_secs(2)).await;
    // Session B goes through the same node.
    let b: Vec<u8> = (0..100_000u32).map(|i| (i % 17) as u8).collect();
    let back = round_trip(relay.clone(), b.clone()).await;
    assert!(back == b, "session b");
    // The node closed session A.
    tokio::time::timeout(Duration::from_secs(30), flood)
        .await
        .expect("session a is closed")
        .unwrap()
        .unwrap();
}

// podssh: at the expiry, a node with no pair name takes the next stored
// pair, with no new start.
#[tokio::test(flavor = "multi_thread")]
async fn a_node_takes_the_next_pair_at_the_expiry() {
    test_host::need_loopback!();
    let (url, state) = fake_relay(None).await;
    let target = Target::parse(&echo().await).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let mut ending = pair(&url);
    ending.expires = now + 3000;
    save(&ending, dir.path()).unwrap();
    let path = dir.path().to_path_buf();
    let node = tokio::spawn(async move { run_node(Some(&path), ending, &target).await });
    wait_for_node(&state, 1).await;
    // The next pair is made while the node runs.
    let mut next = pair(&url);
    next.name = "testnode2".into();
    save(&next, dir.path()).unwrap();
    for _ in 0..400 {
        if state.lock().unwrap().node_sockets >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(
        state.lock().unwrap().node_sockets >= 2,
        "the node opened the next pair"
    );
    assert!(!node.is_finished(), "the node goes on");
    node.abort();
}

// A node of a named pair ends at the expiry (exit 78).
#[tokio::test(flavor = "multi_thread")]
async fn a_named_node_ends_at_the_expiry() {
    test_host::need_loopback!();
    let (url, _state) = fake_relay(None).await;
    let target = Target::parse(&echo().await).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let mut ending = pair(&url);
    ending.expires = now + 2000;
    let err = tokio::time::timeout(Duration::from_secs(20), run_node(None, ending, &target))
        .await
        .expect("the node ends")
        .unwrap_err();
    assert_eq!(podnet::exit::code_of(&err), 78, "{err:#}");
    assert!(format!("{err:#}").contains("expired"), "{err:#}");
}

/// A unix target that does not read for `hold`, then reads to the end. It
/// gives its path and the count of bytes that it read. A unix socket has a
/// fixed buffer, so the node blocks in a write at once.
async fn late_reader(
    dir: &std::path::Path,
    hold: Duration,
) -> (String, tokio::sync::oneshot::Receiver<usize>) {
    let path = dir.join("late.sock");
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let addr = format!("unix:{}", path.display());
    let (done, count) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let Ok((mut socket, _)) = listener.accept().await else {
            return;
        };
        tokio::time::sleep(hold).await;
        let mut total = 0;
        let mut buf = vec![0u8; 64 * 1024];
        while let Ok(Ok(n)) =
            tokio::time::timeout(Duration::from_secs(5), socket.read(&mut buf)).await
        {
            if n == 0 {
                break;
            }
            total += n;
        }
        let _ = done.send(total);
    });
    (addr, count)
}

// T-194: a `close` of the relay ends a session whose target does not read.
// Before, the task stayed in its write and kept its queued frames.
#[tokio::test(flavor = "multi_thread")]
async fn a_relay_close_ends_a_session_whose_target_does_not_read() {
    test_host::need_loopback!();
    let (url, state) = fake_relay(None).await;
    let dir = tempfile::tempdir().unwrap();
    let (addr, count) = late_reader(dir.path(), Duration::from_secs(4)).await;
    let target = Target::parse(&addr).unwrap();
    let node_relay = Relay::new(&url, None).unwrap();
    let node_pair = pair(&url);
    tokio::spawn(async move { node(&node_relay, &node_pair, &target).await });
    wait_for_node(&state, 1).await;
    let relay = Relay::new(&url, None).unwrap();
    let sent = 3 << 20;
    let operator = tokio::spawn(async move {
        let input = vec![5u8; sent];
        let mut out = Vec::new();
        let _ = connect(&relay, NAME, "connect", &input[..], &mut out).await;
    });
    tokio::time::sleep(Duration::from_secs(2)).await;
    // The operator goes away: the relay sends `close` to the node.
    operator.abort();
    let read = tokio::time::timeout(Duration::from_secs(20), count)
        .await
        .expect("the target sees the end")
        .unwrap();
    assert!(read < sent, "the target read {read} of {sent} bytes");
}

/// Run `podnet` with a state directory and a run directory.
fn podnet_in(
    state: &std::path::Path,
    run: &std::path::Path,
    args: &[&str],
) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(args)
        .env("PODNET_STATE_DIR", state)
        .env("PODNET_RUN_DIR", run)
        .env_remove("PODNET_RELAY")
        .output()
        .unwrap()
}

// T-195: a pair with no stop token stops no local instance. The command
// exits 69 and keeps the pair file. Before, it stopped the instance first.
#[test]
fn relay_stop_checks_the_pair_before_it_stops_an_instance() {
    use std::os::fd::AsRawFd as _;
    let state = tempfile::tempdir().unwrap();
    let run = tempfile::tempdir().unwrap();
    let relay_dir = state.path().join("relay");
    let mut with_no_token = pair("http://127.0.0.1:1");
    with_no_token.name = "notoken".into();
    save(&with_no_token, &relay_dir).unwrap();
    // A live instance of the node of this pair: a held lock and a record.
    let mut child = std::process::Command::new("sleep")
        .arg("60")
        .spawn()
        .unwrap();
    std::fs::set_permissions(
        run.path(),
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .unwrap();
    let lock = std::fs::File::create(run.path().join("n1.lock")).unwrap();
    // SAFETY: flock on a descriptor of this process.
    assert_eq!(unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) }, 0);
    let record = serde_json::json!({
        "name": "n1", "pid": child.id(), "started": 1,
        "command": "relay node", "args": ["relay", "node", "notoken"],
        "hash": "x", "cwd": "/", "state": "ready",
        "ready": {"pair": "notoken"}, "log": "n1.log",
    });
    std::fs::write(run.path().join("n1.json"), record.to_string()).unwrap();
    let out = podnet_in(state.path(), run.path(), &["relay", "stop", "notoken"]);
    let err = String::from_utf8_lossy(&out.stderr);
    let alive = child.try_wait().unwrap().is_none();
    let _ = child.kill();
    let _ = child.wait();
    assert_eq!(out.status.code(), Some(69), "{err}");
    assert!(err.contains("stop token"), "{err}");
    assert!(alive, "the instance was stopped: {err}");
    assert!(relay_dir.join("notoken.json").exists(), "the file stays");
    assert!(!String::from_utf8_lossy(&out.stdout).contains("stopped"));
}

/// A TCP target that writes `HELLO`, shuts down its write half, and reads
/// to the end. It gives its address and the bytes that it read.
async fn half_closer() -> (String, tokio::sync::oneshot::Receiver<Vec<u8>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let (done, read) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let Ok((mut socket, _)) = listener.accept().await else {
            return;
        };
        socket.write_all(b"HELLO").await.unwrap();
        socket.shutdown().await.unwrap();
        let mut all = Vec::new();
        let _ = tokio::time::timeout(Duration::from_secs(20), socket.read_to_end(&mut all)).await;
        let _ = done.send(all);
    });
    (addr, read)
}

// T-196: a target that shuts down its write half and still reads gets the
// whole input of the operator. Before, the node sent `close` at the end of
// the read side, and the later input was lost.
#[tokio::test(flavor = "multi_thread")]
async fn a_half_closing_target_gets_the_whole_input() {
    test_host::need_loopback!();
    let (url, state) = fake_relay(None).await;
    let (addr, read) = half_closer().await;
    let target = Target::parse(&addr).unwrap();
    let node_relay = Relay::new(&url, None).unwrap();
    let node_pair = pair(&url);
    tokio::spawn(async move { node(&node_relay, &node_pair, &target).await });
    wait_for_node(&state, 1).await;
    let relay = Relay::new(&url, None).unwrap();
    let input: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    let (ours, theirs) = tokio::io::duplex(1 << 20);
    let sent = input.clone();
    tokio::spawn(async move {
        // The input comes after the target closed its write half.
        tokio::time::sleep(Duration::from_millis(700)).await;
        let mut ours = ours;
        ours.write_all(&sent).await.unwrap();
    });
    let mut out = Vec::new();
    let (reader, _keep) = tokio::io::split(theirs);
    tokio::time::timeout(
        Duration::from_secs(30),
        connect(&relay, NAME, "connect", reader, &mut out),
    )
    .await
    .expect("the session ends")
    .unwrap();
    assert_eq!(out, b"HELLO");
    let got = tokio::time::timeout(Duration::from_secs(10), read)
        .await
        .expect("the target sees the end")
        .unwrap();
    assert!(
        got == input,
        "the target read {} of {} bytes",
        got.len(),
        input.len()
    );
}

// T-091: `serve DIR --via relay --pair NAME` serves the files through the
// node of the pair: an operator session gets the HTTP answer of the file.
#[tokio::test(flavor = "multi_thread")]
async fn serve_via_relay_answers_through_the_relay() {
    test_host::need_loopback!();
    let (url, state) = fake_relay(None).await;
    let (home, run, www) = (
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
    );
    std::fs::set_permissions(
        run.path(),
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .unwrap();
    save(&pair(&url), &home.path().join("relay")).unwrap();
    std::fs::write(www.path().join("a.txt"), "over the relay").unwrap();
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args([
            "serve",
            www.path().to_str().unwrap(),
            "--via",
            "relay",
            "--pair",
            NAME,
            "--list",
        ])
        .env("PODNET_STATE_DIR", home.path())
        .env("PODNET_RUN_DIR", run.path())
        .env_remove("PODNET_RELAY")
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    wait_for_node(&state, 1).await;
    let relay = Relay::new(&url, None).unwrap();
    let (ours, theirs) = tokio::io::duplex(1 << 16);
    let session = tokio::spawn(async move {
        let (read, write) = tokio::io::split(theirs);
        connect(&relay, NAME, "connect", read, write).await
    });
    let (mut read, mut write) = tokio::io::split(ours);
    write
        .write_all(b"GET /a.txt HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut reply = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(20), read.read_to_end(&mut reply)).await;
    let reply = String::from_utf8_lossy(&reply).into_owned();
    drop((read, write));
    let _ = tokio::time::timeout(Duration::from_secs(5), session).await;
    unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
    let status = child.wait().unwrap();
    assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
    assert!(reply.ends_with("over the relay"), "{reply}");
    assert_eq!(status.code(), Some(0), "{status:?}");
    let left: Vec<_> = std::fs::read_dir(run.path())
        .unwrap()
        .flatten()
        .map(|e| e.file_name())
        .collect();
    assert!(
        !left.iter().any(|n| n.to_string_lossy().ends_with(".sock")),
        "{left:?}"
    );
}
