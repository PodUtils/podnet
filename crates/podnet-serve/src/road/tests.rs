use std::time::Duration;

use axum::routing::get;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use super::*;

fn text(body: &'static str) -> axum::Router {
    axum::Router::new().fallback(get(move || async move { body }))
}

fn table() -> Services {
    Services::new()
        .rule(Some("Git.Example"), PathPattern::prefix("/"), text("git"))
        .rule(None, PathPattern::prefix("/files"), text("files"))
        .rule(None, PathPattern::prefix("/"), text("root"))
}

async fn get_on(router: axum::Router, host: &str, path: &str) -> (StatusCode, String) {
    let request = Request::builder()
        .uri(path)
        .header("host", host)
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 16)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

#[tokio::test]
async fn ingress_services_route_by_host_and_path_in_order() {
    let router = table().router();
    assert_eq!(
        get_on(router.clone(), "git.example:443", "/x").await.1,
        "git"
    );
    assert_eq!(
        get_on(router.clone(), "other.example", "/files/a").await.1,
        "files"
    );
    assert_eq!(get_on(router.clone(), "other.example", "/").await.1, "root");
    let only_files = Services::new().rule(None, PathPattern::prefix("/files"), text("files"));
    let (status, _) = get_on(only_files.router(), "a", "/other").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(table().find("GIT.example", "/"), Some(0));
}

/// T-062, the Prove: a table of services through the local road on a unix
/// socket.
#[tokio::test]
async fn ingress_local_road_serves_a_table_on_a_unix_socket() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("road.sock");
    let road = Local {
        listen: format!("unix:{}", path.display()),
        replace: false,
    };
    assert_eq!(road.name(), "local");
    let published = road.publish(table().router()).await.unwrap();
    assert_eq!(
        published.url.as_deref(),
        Some(format!("unix:{}", path.display()).as_str())
    );
    assert_eq!(published.ready["road"], "local");
    use std::os::unix::fs::PermissionsExt as _;
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let server = tokio::spawn(published.run());
    for (host, want) in [("git.example", "git"), ("www.example", "root")] {
        let mut stream = tokio::net::UnixStream::connect(&path).await.unwrap();
        let head = format!("GET / HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
        stream.write_all(head.as_bytes()).await.unwrap();
        let mut answer = String::new();
        tokio::time::timeout(Duration::from_secs(10), stream.read_to_string(&mut answer))
            .await
            .unwrap()
            .unwrap();
        assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
        assert!(answer.ends_with(want), "{answer}");
    }
    // A second road on the same live socket is refused (exit 69).
    let again = Local {
        listen: format!("unix:{}", path.display()),
        replace: false,
    };
    let err = again.publish(text("x")).await.expect_err("a live socket");
    assert!(
        format!("{err:#}").contains("is served by another process"),
        "{err:#}"
    );
    server.abort();
    let _ = server.await;
    assert!(!path.exists(), "the end of the road removes its socket");
}

#[tokio::test]
async fn ingress_local_road_on_tcp_names_the_bound_port() {
    pod_testkit::need_loopback!();
    let road = Local {
        listen: "tcp://127.0.0.1:0".into(),
        replace: false,
    };
    let published = road.publish(text("tcp")).await.unwrap();
    let url = published.url.clone().unwrap();
    assert!(
        url.starts_with("http://127.0.0.1:") && !url.ends_with(":0/"),
        "{url}"
    );
    drop(published);
}

#[tokio::test]
async fn ingress_a_private_socket_goes_with_its_server() {
    let dir = tempfile::tempdir().unwrap();
    let (path, run) = private_socket(dir.path(), "test", text("p")).unwrap();
    assert!(path
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("test-"));
    assert!(path.exists());
    drop(run);
    assert!(!path.exists());
}

/// One request on a unix socket: the status line and the body.
async fn ask(path: &std::path::Path, method: &str, auth: Option<&str>) -> (String, String) {
    let mut stream = tokio::net::UnixStream::connect(path).await.unwrap();
    let auth = auth.map_or(String::new(), |a| format!("Authorization: Basic {a}\r\n"));
    let head = format!(
        "{method} /a.txt HTTP/1.1\r\nHost: x\r\n{auth}Content-Length: 0\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(head.as_bytes()).await.unwrap();
    let mut answer = String::new();
    tokio::time::timeout(Duration::from_secs(10), stream.read_to_string(&mut answer))
        .await
        .unwrap()
        .unwrap();
    let status = answer.lines().next().unwrap_or("").to_string();
    let body = answer.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
    (status, body)
}

/// T-084: the one layer of users and roles on a road: 401 with no
/// credential, 403 for a change by a read user, and the service for each
/// right user. A unix socket of the local road carries it.
#[tokio::test]
async fn auth_layer_guards_a_service_on_the_local_road() {
    use base64::Engine as _;
    let mut users = Users::default();
    users
        .add(
            pod_proxy::Credentials::new("reader", "pw-r").unwrap(),
            Role::Read,
        )
        .unwrap();
    users
        .add(
            pod_proxy::Credentials::new("writer", "pw-w").unwrap(),
            Role::Write,
        )
        .unwrap();
    let access = Access {
        users: Some(users),
        ..Access::default()
    };
    let service = axum::Router::new().route(
        "/a.txt",
        get(|| async { "read" }).put(|| async { "written" }),
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("auth.sock");
    let road = Local {
        listen: format!("unix:{}", path.display()),
        replace: false,
    };
    let published = road.publish(guard(service, access)).await.unwrap();
    let server = tokio::spawn(published.run());
    let basic = |user: &str, pw: &str| {
        base64::engine::general_purpose::STANDARD.encode(format!("{user}:{pw}"))
    };
    let (status, _) = ask(&path, "GET", None).await;
    assert!(status.contains(" 401 "), "{status}");
    let (status, _) = ask(&path, "GET", Some(&basic("reader", "bad"))).await;
    assert!(status.contains(" 401 "), "{status}");
    let (status, body) = ask(&path, "GET", Some(&basic("reader", "pw-r"))).await;
    assert!(status.contains(" 200 "), "{status}");
    assert_eq!(body, "read");
    let (status, _) = ask(&path, "PUT", Some(&basic("reader", "pw-r"))).await;
    assert!(status.contains(" 403 "), "{status}");
    let (status, body) = ask(&path, "PUT", Some(&basic("writer", "pw-w"))).await;
    assert!(status.contains(" 200 "), "{status}");
    assert_eq!(body, "written");
    server.abort();
}

#[test]
fn auth_layer_leaves_an_open_service_as_it_is() {
    // No user, no PIN, no anonymous read: no layer (a request gets the
    // service with no Authorization check).
    let open = Access::default();
    assert!(open.is_open());
    let _ = guard(axum::Router::new(), open);
}
