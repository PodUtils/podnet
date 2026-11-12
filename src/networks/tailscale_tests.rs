//! Tests of the tailnet network against a fake `tailscaled` (T-115).

use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{UnixListener, UnixStream};

use super::*;

async fn read_head(stream: &mut UnixStream) -> String {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while stream.read(&mut byte).await.unwrap_or(0) == 1 {
        head.push(byte[0]);
        if head.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    String::from_utf8_lossy(&head).into_owned()
}

/// A fake `tailscaled`: `/localapi/v0/status` gives `state`; a dial gets a
/// `101` and an echo. The `Dial-Host` of each dial goes in the list.
fn fake(dir: &std::path::Path, state: &'static str) -> (String, Arc<Mutex<Vec<String>>>) {
    let path = dir.join("tailscaled.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let log = log.clone();
            tokio::spawn(async move {
                let head = read_head(&mut stream).await;
                if head.starts_with("GET /localapi/v0/status") {
                    let body = format!(
                        r#"{{"BackendState":"{state}","CurrentTailnet":{{"Name":"example.ts.net"}}}}"#
                    );
                    let _ = stream.write_all(b"HTTP/1.0 200 OK\r\n\r\n").await;
                    let _ = stream.write_all(body.as_bytes()).await;
                    return;
                }
                let host = head
                    .split("\r\n")
                    .find_map(|l| l.strip_prefix("Dial-Host: "))
                    .unwrap_or("")
                    .to_string();
                log.lock().unwrap().push(host);
                let _ = stream
                    .write_all(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: ts-dial\r\nConnection: upgrade\r\n\r\n")
                    .await;
                let mut buf = [0u8; 1024];
                while let Ok(n) = stream.read(&mut buf).await {
                    if n == 0 || stream.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                }
            });
        }
    });
    (path.display().to_string(), seen)
}

#[tokio::test]
async fn network_spec_tailscale_state_names_the_login_state() {
    crate::test_host::need_loopback!();
    let dir = tempfile::tempdir().unwrap();
    let (sock, _) = fake(dir.path(), "Running");
    let line = tailscale_state(&sock).await.unwrap();
    assert!(line.contains("Running"), "{line}");
    assert!(line.contains("example.ts.net"), "{line}");

    let other = tempfile::tempdir().unwrap();
    let (sock, _) = fake(other.path(), "NeedsLogin");
    let why = tailscale_state(&sock).await.unwrap_err();
    assert!(why.contains("NeedsLogin"), "{why}");
    assert!(why.contains("tailscale up"), "{why}");
    assert!(!why.contains('\n'), "{why}");

    let gone = dir.path().join("none.sock").display().to_string();
    let why = tailscale_state(&gone).await.unwrap_err();
    assert!(why.contains("does not answer"), "{why}");
}

#[test]
fn network_spec_tailscale_gives_its_socket() {
    let network = parse("tailscale:/run/ts/tailscaled.sock").unwrap();
    assert_eq!(
        network.tailscale_socket().as_deref(),
        Some("/run/ts/tailscaled.sock")
    );
    assert_eq!(parse("direct").unwrap().tailscale_socket(), None);
    assert_eq!(parse("unix:/run/p.sock").unwrap().tailscale_socket(), None);
}

#[tokio::test]
async fn network_spec_tailscale_up_reports_the_state() {
    crate::test_host::need_loopback!();
    let dir = tempfile::tempdir().unwrap();
    let (sock, _) = fake(dir.path(), "NeedsLogin");
    let network = parse(&format!("tailscale:{sock}")).unwrap();
    let link = network.up(&Ctx::default()).await.unwrap();
    assert!(link.way.is_some());
    let status = link.status.clone();
    assert_eq!(status["scheme"], "tailscale");
    assert_eq!(status["state"], "NeedsLogin");
}

#[tokio::test]
async fn network_spec_tailscale_rows_name_each_tailnet_of_a_file() {
    crate::test_host::need_loopback!();
    let dir = tempfile::tempdir().unwrap();
    let (sock, _) = fake(dir.path(), "NeedsLogin");
    let mut networks = std::collections::BTreeMap::new();
    networks.insert("home".to_string(), "direct".to_string());
    networks.insert("tail".to_string(), format!("tailscale:{sock}"));
    let rows = tailscale_rows(&networks).await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert!(rows[0].starts_with("tail: "), "{rows:?}");
    assert!(rows[0].contains("NeedsLogin"), "{rows:?}");
}

#[tokio::test]
async fn network_spec_tailscale_is_a_network_of_the_router_and_of_a_group() {
    crate::test_host::need_loopback!();
    let dir = tempfile::tempdir().unwrap();
    let (sock, seen) = fake(dir.path(), "Running");
    let text = format!(
        "networks:\n  tail: tailscale:{sock}\n  home: direct\ngroups:\n  g: tail*1\ndoors:\n  - listen: unix:/tmp/t115-a.sock\n    via: tail\n  - listen: unix:/tmp/t115-b.sock\n    via: g\n"
    );
    let config = crate::router::parse(&text).unwrap();
    let router = crate::router::Router::up(&config).await.unwrap();
    for door in &router.doors {
        let target = Target::Name {
            host: "peer.example.ts.net".into(),
            port: 22,
        };
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut conn = door.dial.dial(&target, deadline).await.unwrap();
        conn.io.write_all(b"ping").await.unwrap();
        let mut back = [0u8; 4];
        conn.io.read_exact(&mut back).await.unwrap();
        assert_eq!(&back, b"ping", "{}", door.via);
    }
    assert_eq!(
        seen.lock().unwrap().as_slice(),
        ["peer.example.ts.net", "peer.example.ts.net"]
    );
}
