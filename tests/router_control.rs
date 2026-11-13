//! T-327: add and remove a door and a network on the control socket of a
//! router that runs, with the counters of each door.

mod test_host;

use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use podnet::router::control::{request, Answer};
use podnet::router::{parse, Router};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const FILE: &str = "networks:\n  home: direct\n  spare: direct\n  idle: direct\ngroups:\n  egress: spare*1\ndoors:\n  - listen: tcp://127.0.0.1:0\n    via: home\n";

/// An echo server on the loopback: the port.
async fn echo() -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut s, _)) = listener.accept().await {
            tokio::spawn(async move {
                let (mut r, mut w) = s.split();
                let _ = tokio::io::copy(&mut r, &mut w).await;
            });
        }
    });
    port
}

/// A SOCKS5 CONNECT to `127.0.0.1:port` through the door at `door`.
async fn flow(door: &str, port: u16) -> TcpStream {
    let addr = door.trim_start_matches("tcp://");
    let mut s = TcpStream::connect(addr).await.unwrap();
    s.write_all(&[5, 1, 0]).await.unwrap();
    let mut method = [0u8; 2];
    s.read_exact(&mut method).await.unwrap();
    let mut req = vec![5, 1, 0, 1, 127, 0, 0, 1];
    req.extend_from_slice(&port.to_be_bytes());
    s.write_all(&req).await.unwrap();
    let mut reply = [0u8; 10];
    s.read_exact(&mut reply).await.unwrap();
    assert_eq!(reply[1], 0, "the reply of the door {door}");
    s
}

async fn ping(s: &mut TcpStream, word: &[u8]) {
    s.write_all(word).await.unwrap();
    let mut back = vec![0u8; word.len()];
    tokio::time::timeout(Duration::from_secs(5), s.read_exact(&mut back))
        .await
        .expect("the flow answers")
        .unwrap();
    assert_eq!(back, word);
}

fn door_of<'a>(status: &'a Value, listen: &str) -> Option<&'a Value> {
    status["doors"]
        .as_array()?
        .iter()
        .find(|d| d["listen"] == listen)
}

async fn refused(path: &Path, route: &str, body: Value) -> u16 {
    let err = request(path, "POST", route, Some(&body)).await.unwrap_err();
    err.downcast_ref::<Answer>().expect("an answer").code
}

/// T-327, the Prove: add a door, send a flow through it, read its counters,
/// remove it, with no restart. A flow of another door goes on during the
/// change, and a flow that the removed door had open ends at its own end.
#[tokio::test]
async fn door_add_remove() {
    test_host::need_loopback!();
    let dir = tempfile::tempdir().unwrap();
    let router = Arc::new(Router::up(&parse(FILE).unwrap()).await.unwrap());
    router.open_doors(false).await.unwrap();
    let path = dir.path().join("r.ctl");
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    tokio::spawn(router.clone().serve_control(listener));
    let port = echo().await;

    let status = podnet::router::read_status(&path).await.unwrap();
    let first = status["doors"][0]["listen"].as_str().unwrap().to_string();
    assert!(!first.ends_with(":0"), "the bound port: {first}");
    let mut kept = flow(&first, port).await;
    ping(&mut kept, b"before").await;

    // Add a door; it listens at once.
    let status = request(
        &path,
        "POST",
        "/route/add-door",
        Some(&json!({"listen": "tcp://127.0.0.1:0", "via": "home"})),
    )
    .await
    .unwrap();
    let added = status["doors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["listen"].as_str().unwrap().to_string())
        .find(|l| *l != first)
        .expect("a second door");
    assert_eq!(door_of(&status, &added).unwrap()["flows"], 0);
    let mut one = flow(&added, port).await;
    let mut open = flow(&added, port).await;
    ping(&mut one, b"hello").await;
    ping(&mut kept, b"during").await;

    // The counters of the door: two flows; the bytes of "hello" both ways.
    let status = podnet::router::read_status(&path).await.unwrap();
    let door = door_of(&status, &added).unwrap();
    assert_eq!(door["flows"], 2, "{door}");
    assert_eq!(door["open"], 2, "{door}");
    assert!(door["bytes"].as_u64().unwrap() >= 10, "{door}");
    let first_door = door_of(&status, &first).unwrap();
    assert_eq!(first_door["flows"], 1, "{first_door}");
    let rows = podnet::router::read_metrics(&path).await.unwrap();
    assert!(
        rows.contains(&format!(
            "podnet_route_door_flows_total{{door=\"{added}\"}} 2"
        )),
        "{rows}"
    );
    assert!(rows.contains("podnet_route_door_bytes_total"), "{rows}");

    // The same listen twice is refused; a door needs a known `via`.
    assert_eq!(
        refused(
            &path,
            "/route/add-door",
            json!({"listen": added, "via": "home"})
        )
        .await,
        409
    );
    assert_eq!(
        refused(
            &path,
            "/route/add-door",
            json!({"listen": "tcp://127.0.0.1:0", "via": "nowhere"})
        )
        .await,
        404
    );
    assert_eq!(
        refused(
            &path,
            "/route/add-door",
            json!({"listen": "tcp://127.0.0.1:0", "via": "home", "bogus": 1})
        )
        .await,
        400
    );

    // Remove it: the listener stops, the open flow goes on, and the flow
    // of the other door is not cut.
    let status = request(
        &path,
        "POST",
        "/route/remove-door",
        Some(&json!({"listen": added})),
    )
    .await
    .unwrap();
    assert!(door_of(&status, &added).is_none());
    assert!(door_of(&status, &first).is_some());
    for _ in 0..100 {
        if TcpStream::connect(added.trim_start_matches("tcp://"))
            .await
            .is_err()
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(TcpStream::connect(added.trim_start_matches("tcp://"))
        .await
        .is_err());
    ping(&mut open, b"still open").await;
    ping(&mut kept, b"after").await;
    assert_eq!(
        refused(&path, "/route/remove-door", json!({"listen": added})).await,
        404
    );
}

/// T-327: add a network, give a door to it, and remove it. A network that
/// a door or a group uses is not removed.
#[tokio::test]
async fn network_add_remove() {
    test_host::need_loopback!();
    let dir = tempfile::tempdir().unwrap();
    let router = Arc::new(Router::up(&parse(FILE).unwrap()).await.unwrap());
    router.open_doors(false).await.unwrap();
    let path = dir.path().join("r.ctl");
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    tokio::spawn(router.clone().serve_control(listener));
    let port = echo().await;

    // The names and the specs are checked.
    let add = |name: &str, spec: &str| json!({"name": name, "spec": spec});
    assert_eq!(
        refused(&path, "/route/add-network", add("bad name", "direct")).await,
        400
    );
    assert_eq!(
        refused(&path, "/route/add-network", add("x", "ftp://nowhere")).await,
        400
    );
    assert_eq!(
        refused(&path, "/route/add-network", add("home", "direct")).await,
        409
    );
    assert_eq!(
        refused(&path, "/route/add-network", add("egress", "direct")).await,
        409
    );

    request(
        &path,
        "POST",
        "/route/add-network",
        Some(&add("extra", "direct")),
    )
    .await
    .unwrap();
    let status = request(
        &path,
        "POST",
        "/route/add-door",
        Some(&json!({"listen": "tcp://127.0.0.1:0", "via": "extra"})),
    )
    .await
    .unwrap();
    let listen = status["doors"][1]["listen"].as_str().unwrap().to_string();
    let mut s = flow(&listen, port).await;
    ping(&mut s, b"extra").await;

    // A door uses the network; a group uses `spare`; `home` has a door.
    assert_eq!(
        refused(&path, "/route/remove-network", json!({"name": "extra"})).await,
        409
    );
    assert_eq!(
        refused(&path, "/route/remove-network", json!({"name": "spare"})).await,
        409
    );
    assert_eq!(
        refused(&path, "/route/remove-network", json!({"name": "home"})).await,
        409
    );
    assert_eq!(
        refused(&path, "/route/remove-network", json!({"name": "nowhere"})).await,
        404
    );

    request(
        &path,
        "POST",
        "/route/remove-door",
        Some(&json!({"listen": listen})),
    )
    .await
    .unwrap();
    request(
        &path,
        "POST",
        "/route/remove-network",
        Some(&json!({"name": "extra"})),
    )
    .await
    .unwrap();
    // The flow that was open ends at its own end.
    ping(&mut s, b"later").await;
    assert_eq!(
        refused(
            &path,
            "/route/add-door",
            json!({"listen": "tcp://127.0.0.1:0", "via": "extra"})
        )
        .await,
        404
    );
    assert_eq!(
        refused(&path, "/route/remove-network", json!({"name": "extra"})).await,
        404
    );
    // A network of the file that nothing uses can go, too.
    assert_eq!(
        refused(&path, "/route/remove-network", json!({"name": "egress"})).await,
        404
    );
}

/// T-327: the flags of `podnet route --control` and the exit codes: 0 for a
/// change, 64 for a refusal.
#[test]
fn route_control_flags_add_and_remove() {
    test_host::need_loopback!();
    let dir = tempfile::tempdir().unwrap();
    let ctl = dir.path().join("r.ctl");
    let file = dir.path().join("r.yml");
    std::fs::write(&file, format!("{FILE}control: unix:{}\n", ctl.display())).unwrap();
    let bin = env!("CARGO_BIN_EXE_podnet");
    let mut router = Command::new(bin)
        .args(["route", "--config"])
        .arg(&file)
        .args(["--run-for", "60"])
        .env_remove("PODNET_ERROR_PROGRAM")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let at = format!("unix:{}", ctl.display());
    for _ in 0..300 {
        if ctl.exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let run = |args: &[&str]| {
        let out = Command::new(bin)
            .args(["route", "--control", &at])
            .args(args)
            .env_remove("PODNET_ERROR_PROGRAM")
            .output()
            .unwrap();
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };
    let (code, out, err) = run(&["--add-door", "listen: tcp://127.0.0.1:0\nvia: home"]);
    assert_eq!(code, Some(0), "{err}");
    assert_eq!(out.matches("door tcp://127.0.0.1:").count(), 2, "{out}");
    let (code, out, err) = run(&[
        "--add-door",
        r#"{"listen":"tcp://127.0.0.1:0","via":"spare"}"#,
    ]);
    assert_eq!(code, Some(0), "{err}");
    assert_eq!(out.matches("door tcp://127.0.0.1:").count(), 3, "{out}");
    let (code, _, err) = run(&["--add-network", "x=direct"]);
    assert_eq!(code, Some(0), "{err}");
    let (code, _, err) = run(&["--remove-network", "spare"]);
    assert_eq!(code, Some(64), "{err}");
    assert!(err.contains("uses it"), "{err}");
    let (code, _, err) = run(&["--add-door", "via: home"]);
    assert_eq!(code, Some(64), "{err}");
    let (code, _, err) = run(&["--remove-door", "tcp://127.0.0.1:9"]);
    assert_eq!(code, Some(64), "{err}");
    let (code, _, err) = run(&["--remove-network", "x"]);
    assert_eq!(code, Some(0), "{err}");
    let status = Command::new(bin)
        .args(["route", "--status", &at, "--json"])
        .env_remove("PODNET_ERROR_PROGRAM")
        .output()
        .unwrap();
    let doc: Value = serde_json::from_slice(&status.stdout).unwrap();
    let listen = doc["doors"][2]["listen"].as_str().unwrap().to_string();
    let (code, out, err) = run(&["--remove-door", &listen]);
    assert_eq!(code, Some(0), "{err}");
    assert_eq!(out.matches("door tcp://127.0.0.1:").count(), 2, "{out}");
    let _ = router.kill();
    let _ = router.wait();
}
