use super::*;

/// Issue 60: a non-ASCII name, a quote and a newline.
#[test]
fn content_disposition_is_ascii_with_an_rfc5987_name() {
    assert_eq!(
        content_disposition("a.txt"),
        "attachment; filename=\"a.txt\""
    );
    assert_eq!(
        content_disposition("café.txt"),
        "attachment; filename=\"caf_.txt\"; filename*=UTF-8''caf%C3%A9.txt"
    );
    let quoted = content_disposition("a\"b\nc.txt");
    assert!(
        quoted.starts_with("attachment; filename=\"a_b_c.txt\""),
        "{quoted}"
    );
    for name in ["café.txt", "a\"b\nc.txt", "日本.pdf"] {
        let value = content_disposition(name);
        assert!(value.is_ascii(), "{value}");
        assert!(http::HeaderValue::from_str(&value).is_ok(), "{value}");
    }
}

/// --stop-after: a HEAD does not end the session; the end of a GET body
/// does.
#[tokio::test]
async fn stop_after_needs_a_full_get() {
    use tower::ServiceExt;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("ten.txt");
    std::fs::write(&file, b"0123456789").unwrap();
    let config = ShareConfig::download(&file).stop_after(true);
    let path = format!("/s/{}/file", config.token);
    let app = ShareApp::new(config).unwrap();
    let control = app.control();
    let request = |method: &str| {
        http::Request::builder()
            .method(method)
            .uri(&path)
            .body(axum::body::Body::empty())
            .unwrap()
    };
    let head = app.router().oneshot(request("HEAD")).await.unwrap();
    assert_eq!(head.status(), 200);
    let _ = axum::body::to_bytes(head.into_body(), 1 << 20).await;
    assert!(!control.stop_requested(), "HEAD ended the session");

    let get = app.router().oneshot(request("GET")).await.unwrap();
    assert!(
        !control.stop_requested(),
        "the session ended before the body"
    );
    let body = axum::body::to_bytes(get.into_body(), 1 << 20)
        .await
        .unwrap();
    assert_eq!(&body[..], b"0123456789");
    assert!(control.stop_requested());
    tokio::time::timeout(std::time::Duration::from_secs(1), control.wait_stop())
        .await
        .expect("wait_stop sees an earlier stop");
}

#[test]
fn ranges() {
    assert_eq!(parse_range("bytes=0-4", 10), Ok(Some((0, 4))));
    assert_eq!(parse_range("bytes=5-", 10), Ok(Some((5, 9))));
    assert_eq!(parse_range("bytes=-3", 10), Ok(Some((7, 9))));
    assert_eq!(parse_range("bytes=8-99", 10), Ok(Some((8, 9))));
    assert_eq!(parse_range("bytes=0-1,4-5", 10), Ok(None));
    assert_eq!(parse_range("items=0-1", 10), Ok(None));
    assert_eq!(parse_range("bytes=10-", 10), Err(()));
    assert_eq!(parse_range("bytes=5-2", 10), Err(()));
    assert_eq!(parse_range("bytes=x-1", 10), Err(()));
}

/// A resume: 206 with Content-Range; 304 for the same ETag; 416 for a
/// range past the end; --stop-after only after the end of the file.
#[tokio::test]
async fn downloads_resume() {
    use tower::ServiceExt;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("ten.txt");
    std::fs::write(&file, b"0123456789").unwrap();
    let config = ShareConfig::download(&file).stop_after(true);
    let path = format!("/s/{}/file", config.token);
    let app = ShareApp::new(config).unwrap();
    let control = app.control();
    let get = |extra: Vec<(&'static str, String)>| {
        let mut b = http::Request::builder().uri(&path);
        for (k, v) in extra {
            b = b.header(k, v);
        }
        app.router()
            .oneshot(b.body(axum::body::Body::empty()).unwrap())
    };
    let full = get(vec![]).await.unwrap();
    let etag = full.headers()[header::ETAG].to_str().unwrap().to_string();
    assert_eq!(full.headers()[header::ACCEPT_RANGES], "bytes");
    drop(full);

    let part = get(vec![("range", "bytes=2-4".into())]).await.unwrap();
    assert_eq!(part.status(), 206);
    assert_eq!(part.headers()[header::CONTENT_RANGE], "bytes 2-4/10");
    let body = axum::body::to_bytes(part.into_body(), 1 << 20)
        .await
        .unwrap();
    assert_eq!(&body[..], b"234");
    assert!(!control.stop_requested(), "a part ended the session");

    let not_modified = get(vec![("if-none-match", etag.clone())]).await.unwrap();
    assert_eq!(not_modified.status(), 304);
    let past = get(vec![("range", "bytes=50-".into())]).await.unwrap();
    assert_eq!(past.status(), 416);
    let stale = get(vec![
        ("range", "bytes=2-4".into()),
        ("if-range", "\"old\"".into()),
    ])
    .await
    .unwrap();
    assert_eq!(stale.status(), 200);
    drop(stale);
    assert!(!control.stop_requested());

    let rest = get(vec![("range", "bytes=5-".into())]).await.unwrap();
    let body = axum::body::to_bytes(rest.into_body(), 1 << 20)
        .await
        .unwrap();
    assert_eq!(&body[..], b"56789");
    assert!(
        control.stop_requested(),
        "the end of the file ends the session"
    );
}

// Issue 234: a download error has no error detail of the host.
#[tokio::test]
async fn a_download_error_has_no_host_detail() {
    use tower::ServiceExt;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("gone.bin");
    std::fs::write(&file, b"data").unwrap();
    let config = ShareConfig::download(&file);
    let token = config.token.clone();
    let app = ShareApp::new(config).unwrap();
    std::fs::remove_file(&file).unwrap();
    let response = app
        .router()
        .oneshot(
            http::Request::builder()
                .uri(format!("/s/{token}/file"))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 404);
    let body = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let body = String::from_utf8_lossy(&body);
    assert!(!body.contains("os error"), "{body}");
    assert!(!body.contains(dir.path().to_str().unwrap()), "{body}");
    assert!(body.contains("not available"), "{body}");
}

// Issue 271: `If-None-Match: *` and a weak tag give 304; each response
// of a share has `Cache-Control: no-store`.
#[tokio::test]
async fn a_download_answers_a_weak_tag_and_is_not_stored() {
    use tower::ServiceExt;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a.txt");
    std::fs::write(&file, b"abc").unwrap();
    let mut config = ShareConfig::download(&file);
    let token = config.token.clone();
    let app = ShareApp::new(config.clone()).unwrap();
    let get = |path: String, inm: Option<String>| {
        let mut b = http::Request::builder().uri(path);
        if let Some(v) = inm {
            b = b.header("if-none-match", v);
        }
        app.router()
            .oneshot(b.body(axum::body::Body::empty()).unwrap())
    };
    let full = get(format!("/s/{token}/file"), None).await.unwrap();
    assert_eq!(full.headers()[header::CACHE_CONTROL], "no-store");
    let etag = full.headers()[header::ETAG].to_str().unwrap().to_string();
    for inm in [
        "*".to_string(),
        format!("W/{etag}"),
        format!("\"x\", {etag}"),
    ] {
        let response = get(format!("/s/{token}/file"), Some(inm.clone()))
            .await
            .unwrap();
        assert_eq!(response.status(), 304, "{inm}");
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    }
    let landing = get(format!("/s/{token}/"), None).await.unwrap();
    assert_eq!(landing.headers()[header::CACHE_CONTROL], "no-store");

    config.gate = Gate::pin();
    let app = ShareApp::new(config).unwrap();
    let locked = app
        .router()
        .oneshot(
            http::Request::builder()
                .uri(format!("/s/{token}/"))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(locked.headers()[header::CACHE_CONTROL], "no-store");
    let dir = tempfile::tempdir().unwrap();
    let config = ShareConfig::upload(dir.path());
    let token = config.token.clone();
    let app = ShareApp::new(config).unwrap();
    let landing = app
        .router()
        .oneshot(
            http::Request::builder()
                .uri(format!("/s/{token}/"))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(landing.headers()[header::CACHE_CONTROL], "no-store");
}

/// podssh #105: a FIFO put at the path of the shared file after the
/// start gets 404 at once (an open of a FIFO blocked the request).
#[tokio::test]
async fn a_fifo_in_place_of_the_shared_file_does_not_block() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a.bin");
    std::fs::write(&file, b"data").unwrap();
    let config = ShareConfig::download(&file);
    let token = config.token.clone();
    let app = ShareApp::new(config).unwrap();
    std::fs::remove_file(&file).unwrap();
    let fifo = std::ffi::CString::new(file.to_str().unwrap()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o644) }, 0);
    let (status, _) = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        send(&app.router(), "GET", &format!("/s/{token}/file"), ""),
    )
    .await
    .expect("no block");
    assert_eq!(status, 404);
}

/// Only the session id of the share opens its paths, and an unlock goes
/// back to a path of this site only.
#[tokio::test]
async fn the_session_id_is_checked() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a.bin");
    std::fs::write(&file, b"secret file").unwrap();
    let mut config = ShareConfig::download(&file);
    config.gate = Gate::pin();
    let pin = config.gate.pin_display().unwrap().to_string();
    let token = config.token.clone();
    let app = ShareApp::new(config).unwrap();
    let router = app.router();

    // The gate answers 200 with the lock page for the right id.
    assert_eq!(
        send(&router, "GET", &format!("/s/{token}/"), "").await.0,
        200
    );
    for wrong in ["/s/1/", "/s/1/file", "/s/deadbeef/", "/s/", "/"] {
        assert_eq!(send(&router, "GET", wrong, "").await.0, 404, "{wrong}");
    }
    let upper = format!("/s/{}/file", token.to_ascii_uppercase());
    assert_eq!(send(&router, "GET", &upper, "").await.0, 404);

    let (status, location) = send(
        &router,
        "POST",
        UNLOCK_PATH,
        &format!("password={pin}&next=//example.com"),
    )
    .await;
    assert_eq!((status, location.as_deref()), (303, Some("/")));

    let big = format!("password=1&next={}", "a".repeat(10_000));
    assert_eq!(send(&router, "POST", UNLOCK_PATH, &big).await.0, 413);
}

/// T-091: a directory in a download session goes as one tar stream of the
/// tree (no dotfile), named DIR.tar. --stop-after ends the session after
/// the last byte of a GET.
#[tokio::test]
async fn a_directory_downloads_as_one_tar() {
    use tower::ServiceExt;
    let base = tempfile::tempdir().unwrap();
    let dir = base.path().join("photos");
    std::fs::create_dir_all(dir.join("2026")).unwrap();
    std::fs::write(dir.join("2026/a.jpg"), vec![3u8; 5000]).unwrap();
    std::fs::write(dir.join(".secret"), "x").unwrap();
    let config = ShareConfig::download(&dir).stop_after(true);
    let token = config.token.clone();
    let app = ShareApp::new(config).unwrap();
    let control = app.control();
    let landing = http::Request::get(format!("/s/{token}/"))
        .body(axum::body::Body::empty())
        .unwrap();
    let response = app.router().oneshot(landing).await.unwrap();
    let body = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    assert!(String::from_utf8_lossy(&body).contains("photos.tar"));
    let get = http::Request::get(format!("/s/{token}/file"))
        .body(axum::body::Body::empty())
        .unwrap();
    let response = app.router().oneshot(get).await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-type"], "application/x-tar");
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert!(!control.stop_requested());
    let body = axum::body::to_bytes(response.into_body(), 1 << 22)
        .await
        .unwrap();
    assert!(control.stop_requested());
    let mut reader = crate::tar::Reader::new(&body[..]);
    let mut names = Vec::new();
    while let Some(entry) = reader.next_entry().unwrap() {
        names.push(entry.path);
    }
    assert_eq!(names, ["photos", "photos/2026", "photos/2026/a.jpg"]);
}
