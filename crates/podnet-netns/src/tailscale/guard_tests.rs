//! Tests of the tailnet guard (T-243) against a fake LocalAPI.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

use super::*;

/// The body that the fake LocalAPI gives for `/localapi/v0/status`.
type Body = Arc<Mutex<String>>;

fn status_body(state: &str, name: &str, suffix: &str) -> String {
    format!(
        r#"{{"BackendState":"{state}","CurrentTailnet":{{"Name":"{name}","MagicDNSSuffix":"{suffix}"}}}}"#
    )
}

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

/// A fake `tailscaled`: the status is the current `body`; a dial gets a
/// `101` and an echo.
fn fake(dir: &std::path::Path, body: Body) -> Dialer {
    let path = dir.join("tailscaled.sock");
    let listener = UnixListener::bind(&path).unwrap();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let body = body.clone();
            tokio::spawn(async move {
                let head = read_head(&mut stream).await;
                if head.starts_with("GET /localapi/v0/status") {
                    let text = body.lock().unwrap().clone();
                    let _ = stream.write_all(b"HTTP/1.0 200 OK\r\n\r\n").await;
                    let _ = stream.write_all(text.as_bytes()).await;
                    return;
                }
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
    Dialer::new(path)
}

fn body(text: String) -> Body {
    Arc::new(Mutex::new(text))
}

fn config(expected: &str) -> GuardConfig {
    GuardConfig {
        expected: expected.into(),
        wait: Duration::from_millis(300),
        recheck: Duration::from_millis(50),
    }
}

async fn socks_connect(front: &std::path::Path) -> (UnixStream, u8) {
    let mut client = UnixStream::connect(front).await.unwrap();
    client.write_all(&[5, 1, 0]).await.unwrap();
    let mut method = [0u8; 2];
    client.read_exact(&mut method).await.unwrap();
    client
        .write_all(&[5, 1, 0, 1, 100, 82, 22, 3, 0x1f, 0x90])
        .await
        .unwrap();
    let mut reply = [0u8; 10];
    client.read_exact(&mut reply).await.unwrap();
    (client, reply[1])
}

#[tokio::test]
async fn tailscale_tailnet_mismatch_refuses_before_the_bind() {
    let dir = tempfile::tempdir().unwrap();
    let dialer = fake(
        dir.path(),
        body(status_body("Running", "lab", "lab.ts.net")),
    );
    let front = dir.path().join("front.sock");
    let error = serve_guarded(
        dialer,
        ProxyListen::Unix(front.clone()),
        80,
        Some(config("office")),
    )
    .await
    .err()
    .expect("a refusal");
    let guard = error.downcast_ref::<GuardError>().expect("a GuardError");
    assert!(matches!(guard, GuardError::Mismatch { .. }), "{guard}");
    let text = error.to_string();
    assert!(
        text.contains("\"lab\"")
            && text.contains("\"office\"")
            && text.contains("nothing was bound"),
        "{text}"
    );
    assert!(!front.exists(), "the listener was bound");
}

#[tokio::test]
async fn tailscale_tailnet_match_opens_the_door() {
    for expected in ["lab", "LAB", "lab.ts.net"] {
        let dir = tempfile::tempdir().unwrap();
        let dialer = fake(
            dir.path(),
            body(status_body("Running", "lab", "lab.ts.net")),
        );
        let front = dir.path().join("front.sock");
        let proxy = serve_guarded(
            dialer,
            ProxyListen::Unix(front.clone()),
            80,
            Some(config(expected)),
        )
        .await
        .unwrap_or_else(|e| panic!("{expected}: {e}"));
        let (mut client, code) = socks_connect(&front).await;
        assert_eq!(code, 0, "{expected}");
        client.write_all(b"ping").await.unwrap();
        let mut echoed = [0u8; 4];
        client.read_exact(&mut echoed).await.unwrap();
        assert_eq!(&echoed, b"ping");
        proxy.abort();
    }
}

#[tokio::test]
async fn tailscale_needs_login_ends_with_69() {
    let dir = tempfile::tempdir().unwrap();
    let dialer = fake(dir.path(), body(status_body("NeedsLogin", "", "")));
    let front = dir.path().join("front.sock");
    let error = serve_guarded(
        dialer,
        ProxyListen::Unix(front.clone()),
        80,
        Some(config("lab")),
    )
    .await
    .err()
    .expect("a refusal");
    match error.downcast_ref::<GuardError>() {
        Some(GuardError::NotReady(why)) => assert!(why.contains("NeedsLogin"), "{why}"),
        other => panic!("{other:?}"),
    }
    assert!(!front.exists());
}

#[tokio::test]
async fn tailscale_tailnet_change_refuses_new_dials() {
    let dir = tempfile::tempdir().unwrap();
    let state = body(status_body("Running", "office", "office.ts.net"));
    let dialer = fake(dir.path(), state.clone());
    let front = dir.path().join("front.sock");
    let proxy = serve_guarded(
        dialer,
        ProxyListen::Unix(front.clone()),
        80,
        Some(config("office")),
    )
    .await
    .unwrap();
    let (mut open, code) = socks_connect(&front).await;
    assert_eq!(code, 0);

    *state.lock().unwrap() = status_body("Running", "lab", "lab.ts.net");
    tokio::time::sleep(Duration::from_millis(400)).await;

    let (_, code) = socks_connect(&front).await;
    assert_eq!(code, 0x02, "SOCKS5 refusal");
    let mut http = UnixStream::connect(&front).await.unwrap();
    http.write_all(b"CONNECT 100.82.22.3:80 HTTP/1.1\r\nHost: x\r\n\r\n")
        .await
        .unwrap();
    let mut answer = vec![0u8; 64];
    let n = http.read(&mut answer).await.unwrap();
    assert!(
        String::from_utf8_lossy(&answer[..n]).starts_with("HTTP/1.1 403"),
        "{}",
        String::from_utf8_lossy(&answer[..n])
    );
    // A connection that was open before the change stays up.
    open.write_all(b"ping").await.unwrap();
    let mut echoed = [0u8; 4];
    open.read_exact(&mut echoed).await.unwrap();
    assert_eq!(&echoed, b"ping");

    // The daemon returns to the tailnet: the door opens again.
    *state.lock().unwrap() = status_body("Running", "office", "office.ts.net");
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(socks_connect(&front).await.1, 0);
    proxy.abort();
}

#[tokio::test]
async fn tailscale_unreadable_status_is_not_ready() {
    // Not JSON, an error code, a body over the limit.
    assert!(status::status_of(b"HTTP/1.0 200 OK\r\n\r\nnot json").is_err());
    assert!(status::status_of(b"HTTP/1.0 500 Oops\r\n\r\n{}").is_err());
    let mut large = b"HTTP/1.0 200 OK\r\n\r\n{\"BackendState\":\"Running\"}".to_vec();
    large.resize(4 << 20, b' ');
    assert!(status::status_of(&large).is_err());

    let dir = tempfile::tempdir().unwrap();
    let dialer = fake(dir.path(), body("not json".into()));
    let error = wait_ready(&dialer, "lab", Duration::from_millis(300))
        .await
        .unwrap_err();
    assert!(
        matches!(&error, GuardError::NotReady(why) if why.contains("unreadable")),
        "{error}"
    );
}

#[tokio::test]
async fn tailscale_guard_rules_and_describe() {
    let status = |state: &str, name: Option<&str>, suffix: Option<&str>| Status {
        backend_state: state.into(),
        tailnet_name: name.map(Into::into),
        magic_dns_suffix: suffix.map(Into::into),
    };
    assert_eq!(guard(None, &status("Running", None, None)), Verdict::Ready);
    assert_eq!(
        guard(Some("a"), &status("Stopped", Some("a"), None)),
        Verdict::Wait
    );
    assert_eq!(
        guard(Some("a"), &status("Running", None, None)),
        Verdict::Wait
    );
    assert_eq!(
        guard(
            Some("a.ts.net."),
            &status("Running", None, Some("A.ts.net"))
        ),
        Verdict::Ready
    );
    assert_eq!(
        guard(Some("a"), &status("Running", None, Some("b.ts.net"))),
        Verdict::Mismatch("b.ts.net".into())
    );
    // An old daemon: only the top-level suffix.
    let old = status::status_of(
        b"HTTP/1.0 200 OK\r\n\r\n{\"BackendState\":\"Running\",\"MagicDNSSuffix\":\"x.ts.net\"}",
    )
    .unwrap();
    assert_eq!(old.magic_dns_suffix.as_deref(), Some("x.ts.net"));

    let dir = tempfile::tempdir().unwrap();
    let dialer = fake(
        dir.path(),
        body(status_body("Running", "lab", "lab.ts.net")),
    );
    let line = dialer.describe().await.unwrap();
    assert!(
        line.contains("BackendState=Running")
            && line.contains("tailnet=lab")
            && line.contains("suffix=lab.ts.net")
            && line.contains("TS_FORCE_NOISE_443="),
        "{line}"
    );
    let gone = Dialer::new(dir.path().join("none.sock"));
    assert!(gone
        .describe()
        .await
        .unwrap_err()
        .contains("does not answer"));
}
