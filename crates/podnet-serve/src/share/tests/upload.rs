use super::send;
use super::*;

/// A multipart body with one file part for each (name, size).
fn multipart(files: &[(&str, usize)]) -> Vec<u8> {
    let mut body = Vec::new();
    for (name, size) in files {
        body.extend_from_slice(
            format!(
                "--XB\r\nContent-Disposition: form-data; name=\"files\"; filename=\"{name}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend(std::iter::repeat_n(b'x', *size));
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(b"--XB--\r\n");
    body
}

fn upload_request(token: &str, body: axum::body::Body) -> http::Request<axum::body::Body> {
    http::Request::builder()
        .method("POST")
        .uri(format!("/s/{token}/upload"))
        .header("content-type", "multipart/form-data; boundary=XB")
        .body(body)
        .unwrap()
}

fn files_in(dir: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// Issue 58: a request over the file limit or the session total gets
/// 413 and saves no file of it (before, the files before the limit
/// stayed); the slots and the bytes go back, so a smaller request works.
#[tokio::test]
async fn an_upload_over_a_limit_saves_nothing() {
    use tower::ServiceExt;
    let dir = tempfile::tempdir().unwrap();
    let mut config = ShareConfig::upload(dir.path());
    config.max_files = 2;
    config.max_bytes = 10;
    config.max_total = 15;
    let token = config.token.clone();
    let app = ShareApp::new(config).unwrap();
    let send = |files: &[(&str, usize)]| {
        app.router().oneshot(upload_request(
            &token,
            axum::body::Body::from(multipart(files)),
        ))
    };
    let three = send(&[("a.txt", 1), ("b.txt", 1), ("c.txt", 1)])
        .await
        .unwrap();
    assert_eq!(three.status(), 413);
    assert!(
        files_in(dir.path()).is_empty(),
        "{:?}",
        files_in(dir.path())
    );
    let total = send(&[("a.txt", 10), ("b.txt", 10)]).await.unwrap();
    assert_eq!(total.status(), 413);
    let text = axum::body::to_bytes(total.into_body(), 4096).await.unwrap();
    assert!(String::from_utf8_lossy(&text).contains("session limit"));
    assert!(
        files_in(dir.path()).is_empty(),
        "{:?}",
        files_in(dir.path())
    );
    let two = send(&[("a.txt", 10), ("b.txt", 5)]).await.unwrap();
    assert_eq!(two.status(), 200);
    assert_eq!(files_in(dir.path()), ["a.txt", "b.txt"]);
    assert_eq!(
        std::fs::metadata(dir.path().join("a.txt")).unwrap().len(),
        10
    );
}

// Issue 233: an uploaded file has mode 0600, with each umask.
#[tokio::test]
async fn an_upload_has_mode_0600() {
    use std::os::unix::fs::PermissionsExt;
    use tower::ServiceExt;
    let dir = tempfile::tempdir().unwrap();
    let config = ShareConfig::upload(dir.path());
    let token = config.token.clone();
    let app = ShareApp::new(config).unwrap();
    for _ in 0..2 {
        let response = app
            .router()
            .oneshot(upload_request(
                &token,
                axum::body::Body::from(multipart(&[("a.txt", 5)])),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
    }
    assert_eq!(files_in(dir.path()), ["a-1.txt", "a.txt"]);
    for name in ["a.txt", "a-1.txt"] {
        let mode = std::fs::metadata(dir.path().join(name))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "{name}: {mode:o}");
    }
    let (_, file) = create_unique(dir.path(), "b.txt").await.unwrap();
    let mode = file.metadata().await.unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600, "{mode:o}");
}

/// A body in chunks of 1000 bytes that counts the bytes read.
fn counted(body: Vec<u8>) -> (axum::body::Body, Arc<std::sync::atomic::AtomicUsize>) {
    let read = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = read.clone();
    let chunks: Vec<Result<bytes::Bytes, std::io::Error>> = body
        .chunks(1000)
        .map(|c| Ok(bytes::Bytes::copy_from_slice(c)))
        .collect();
    use futures::StreamExt as _;
    let stream = futures::stream::iter(chunks).inspect(move |c| {
        if let Ok(c) = c {
            seen.fetch_add(c.len(), Ordering::SeqCst);
        }
    });
    (axum::body::Body::from_stream(stream), read)
}

// Issue 232: a field with no file name counts toward --max-request. A
// chunked body (no Content-Length) with a large text field gets 413.
#[tokio::test]
async fn a_field_with_no_file_name_is_counted() {
    use tower::ServiceExt;
    let dir = tempfile::tempdir().unwrap();
    let mut config = ShareConfig::upload(dir.path());
    config.max_request = 1000;
    let token = config.token.clone();
    let app = ShareApp::new(config).unwrap();
    let field = |size: usize| {
        let mut body = b"--XB\r\nContent-Disposition: form-data; name=\"note\"\r\n\r\n".to_vec();
        body.extend(std::iter::repeat_n(b'n', size));
        body.extend_from_slice(b"\r\n");
        body
    };
    let mut body = field(3000);
    body.extend(multipart(&[("a.txt", 10)]));
    let (stream, _) = counted(body);
    let response = app
        .router()
        .oneshot(upload_request(&token, stream))
        .await
        .unwrap();
    assert_eq!(response.status(), 413);
    assert!(files_in(dir.path()).is_empty());
    // A small text field and a file are under the limit.
    let mut body = field(100);
    body.extend(multipart(&[("a.txt", 10)]));
    let (stream, _) = counted(body);
    let response = app
        .router()
        .oneshot(upload_request(&token, stream))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(files_in(dir.path()), ["a.txt"]);
}

// Live run 41556133042: a refused body is read to its end before the
// 413, with and with no Content-Length. Before, the body limit of the
// router stopped the read at --max-request, and over HTTP/2 the edge
// gave 500.
#[tokio::test]
async fn a_refused_body_is_read_to_its_end() {
    use tower::ServiceExt;
    let dir = tempfile::tempdir().unwrap();
    let mut config = ShareConfig::upload(dir.path());
    config.max_request = 10_000;
    let token = config.token.clone();
    let app = ShareApp::new(config).unwrap();
    for with_length in [true, false] {
        let body = multipart(&[("big.bin", 60_000)]);
        let len = body.len();
        let (stream, read) = counted(body);
        let mut request = upload_request(&token, stream);
        if with_length {
            request
                .headers_mut()
                .insert("content-length", len.to_string().parse().unwrap());
        }
        let response = app.router().oneshot(request).await.unwrap();
        assert_eq!(response.status(), 413, "content-length {with_length}");
        assert_eq!(
            read.load(Ordering::SeqCst),
            len,
            "content-length {with_length}"
        );
        assert!(files_in(dir.path()).is_empty());
    }
    // An upload under the limit still works.
    let body = multipart(&[("small.bin", 5000)]);
    let response = app
        .router()
        .oneshot(upload_request(&token, axum::body::Body::from(body)))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
}

/// Issue 92: a request over max_request (by its Content-Length) is read
/// and dropped, then gets 413, and saves nothing; the landing page and a
/// smaller upload still work. A body that stops gets its 413 after the
/// pause of DROP_IDLE (the clock is paused here).
#[tokio::test(start_paused = true)]
async fn a_content_length_over_max_request_gets_413_at_once() {
    use tower::ServiceExt;
    let dir = tempfile::tempdir().unwrap();
    let mut config = ShareConfig::upload(dir.path());
    config.max_request = 1000;
    let token = config.token.clone();
    let app = ShareApp::new(config).unwrap();
    let never = futures::stream::pending::<Result<bytes::Bytes, std::io::Error>>();
    let mut stalled = upload_request(&token, axum::body::Body::from_stream(never));
    stalled
        .headers_mut()
        .insert("content-length", "60000000".parse().unwrap());
    let response = app.router().oneshot(stalled).await.unwrap();
    assert_eq!(response.status(), 413);
    let body = multipart(&[("big.bin", 5000)]);
    let mut request = upload_request(&token, axum::body::Body::from(body.clone()));
    request
        .headers_mut()
        .insert("content-length", body.len().to_string().parse().unwrap());
    let response = app.router().oneshot(request).await.unwrap();
    assert_eq!(response.status(), 413);
    let text = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let text = String::from_utf8_lossy(&text);
    assert!(text.contains("at most 1000 B"), "{text}");
    assert!(files_in(dir.path()).is_empty());
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
    assert_eq!(landing.status(), 200);
    let page = axum::body::to_bytes(landing.into_body(), 1 << 16)
        .await
        .unwrap();
    assert!(String::from_utf8_lossy(&page).contains("per upload"));
    let small = app
        .router()
        .oneshot(upload_request(
            &token,
            axum::body::Body::from(multipart(&[("a.txt", 10)])),
        ))
        .await
        .unwrap();
    assert_eq!(small.status(), 200);
    assert_eq!(files_in(dir.path()), ["a.txt"]);
}

/// Issue 59: a body that stops gets 408 after CHUNK_LIMIT, and leaves no
/// file, partial or staged.
#[tokio::test(start_paused = true)]
async fn a_stalled_upload_leaves_no_file() {
    use futures::StreamExt;
    use tower::ServiceExt;
    let dir = tempfile::tempdir().unwrap();
    let config = ShareConfig::upload(dir.path());
    let token = config.token.clone();
    let app = ShareApp::new(config).unwrap();
    let mut start = multipart(&[("big.bin", 1000)]);
    start.truncate(start.len() - 300);
    let stream =
        futures::stream::once(async move { Ok::<_, std::io::Error>(bytes::Bytes::from(start)) })
            .chain(futures::stream::pending());
    let response = app
        .router()
        .oneshot(upload_request(
            &token,
            axum::body::Body::from_stream(stream),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), 408);
    assert!(
        files_in(dir.path()).is_empty(),
        "{:?}",
        files_in(dir.path())
    );
}

/// The upload page shows a progress bar, and its script is whole.
#[test]
fn the_upload_page_has_progress() {
    let page = upload_page("abc", 1 << 20, MAX_REQUEST);
    assert!(page.contains("upload.onprogress") && page.contains("<progress"));
    let script = page
        .split("<script>")
        .nth(1)
        .unwrap()
        .split("</script>")
        .next()
        .unwrap();
    assert_eq!(script.matches('{').count(), script.matches('}').count());
    assert_eq!(script.matches('(').count(), script.matches(')').count());
}

// Issue 234: after the numbered names, a random name; the stem is never
// full.
#[tokio::test]
async fn a_full_stem_gets_a_random_name() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["a.txt", "a-1.txt", "a-2.txt"] {
        std::fs::write(dir.path().join(name), "x").unwrap();
    }
    let (path, _) = create_unique_with(dir.path(), "a.txt", 3).await.unwrap();
    let name = path.file_name().unwrap().to_str().unwrap().to_string();
    assert!(name.starts_with("a-") && name.ends_with(".txt"), "{name}");
    assert_eq!(name.len(), "a-12345678.txt".len(), "{name}");
    assert_eq!(files_in(dir.path()).len(), 4);
}

/// Twelve uploads of one name at the same time: twelve files, each with
/// its own body.
#[tokio::test]
async fn concurrent_names_do_not_collide() {
    let dir = tempfile::tempdir().unwrap();
    let mut tasks = Vec::new();
    for i in 0..12 {
        let dir = dir.path().to_path_buf();
        tasks.push(tokio::spawn(async move {
            let (path, mut file) = create_unique(&dir, "race.txt").await.unwrap();
            tokio::io::AsyncWriteExt::write_all(&mut file, format!("race-{i}").as_bytes())
                .await
                .unwrap();
            // A tokio File writes in the background: the flush waits for
            // the write (CI run 41569743059 read an empty file).
            tokio::io::AsyncWriteExt::flush(&mut file).await.unwrap();
            path
        }));
    }
    let mut paths = Vec::new();
    for t in tasks {
        paths.push(t.await.unwrap());
    }
    paths.sort();
    paths.dedup();
    assert_eq!(paths.len(), 12);
    let mut bodies: Vec<String> = paths
        .iter()
        .map(|p| std::fs::read_to_string(p).unwrap())
        .collect();
    bodies.sort();
    bodies.dedup();
    assert_eq!(bodies.len(), 12);
    assert!(dir.path().join("race.txt").exists());
    assert!(dir.path().join("race-1.txt").exists());
}

/// Issue 194: a POST to a gated upload with no cookie gets 403 and
/// saves nothing; a GET gets the page with 200; a wrong PIN gets 403.
#[tokio::test]
async fn a_locked_post_gets_403_and_saves_nothing() {
    use tower::ServiceExt;
    let dir = tempfile::tempdir().unwrap();
    let mut config = ShareConfig::upload(dir.path());
    config.gate = Gate::pin();
    let token = config.token.clone();
    let app = ShareApp::new(config).unwrap();
    let body = multipart(&[("a.bin", 100)]);
    let response = app
        .router()
        .oneshot(upload_request(&token, axum::body::Body::from(body)))
        .await
        .unwrap();
    assert_eq!(response.status(), 403);
    assert!(files_in(dir.path()).is_empty());
    let router = app.router();
    assert_eq!(
        send(&router, "GET", &format!("/s/{token}/"), "").await.0,
        200
    );
    let (status, _) = send(&router, "POST", UNLOCK_PATH, "password=wrong&next=/").await;
    assert_eq!(status, 403);
    assert_eq!(crate::gate::locked_status("HEAD"), 200);
    assert_eq!(crate::gate::locked_status("PUT"), 403);
}

// Issue 229: an unlock POST of another site gets 403, and the count of
// wrong codes does not change.
#[tokio::test]
async fn an_unlock_from_another_site_is_refused() {
    use tower::ServiceExt;
    let dir = tempfile::tempdir().unwrap();
    let mut config = ShareConfig::upload(dir.path());
    config.gate = Gate::pin();
    let gate = config.gate.clone();
    let app = ShareApp::new(config).unwrap();
    for _ in 0..crate::gate::MAX_FAILURES {
        let request = http::Request::builder()
            .method("POST")
            .uri(UNLOCK_PATH)
            .header("host", "x.trycloudflare.com")
            .header("origin", "https://evil.example")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(axum::body::Body::from("password=000000&next=/"))
            .unwrap();
        let response = app.router().oneshot(request).await.unwrap();
        assert_eq!(response.status(), 403);
    }
    assert!(!gate.locked_out());
    let pin = gate.pin_display().unwrap().to_string();
    let request = http::Request::builder()
        .method("POST")
        .uri(UNLOCK_PATH)
        .header("host", "x.trycloudflare.com")
        .header("origin", "https://x.trycloudflare.com")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(axum::body::Body::from(format!("password={pin}&next=/")))
        .unwrap();
    let response = app.router().oneshot(request).await.unwrap();
    assert_eq!(response.status(), 303);
}
