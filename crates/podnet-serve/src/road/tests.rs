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
