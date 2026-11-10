//! The tests of the file server.

use super::*;

async fn get(router: &Router, path: &str) -> (u16, String) {
    use tower::ServiceExt;
    let request = http::Request::builder()
        .uri(path)
        .body(axum::body::Body::empty())
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status().as_u16();
    let body = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&body).into_owned())
}

/// T-085: `--tar` gives a directory as one ustar stream. The list of
/// `tar -tf` is the tree: no dotfile, no link out of the root.
#[tokio::test]
async fn a_directory_downloads_as_a_tar() {
    use std::os::unix::fs::symlink;
    let base = tempfile::tempdir().unwrap();
    let root = base.path().join("www");
    std::fs::create_dir_all(root.join("sub/deep")).unwrap();
    std::fs::write(root.join("a.txt"), "alpha").unwrap();
    std::fs::write(root.join("sub/b c.txt"), vec![9u8; 70_000]).unwrap();
    std::fs::write(root.join("sub/deep/ü.bin"), [0u8, 255]).unwrap();
    std::fs::write(root.join(".env"), "SECRET=1").unwrap();
    std::fs::write(base.path().join("outside.txt"), "outside").unwrap();
    symlink(base.path().join("outside.txt"), root.join("out-link")).unwrap();
    symlink(root.join("a.txt"), root.join("in-link")).unwrap();
    let router = static_router_with(&root, false, true).unwrap();
    use tower::ServiceExt;
    let request = http::Request::builder()
        .uri("/?tar")
        .body(axum::body::Body::empty())
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-type"], "application/x-tar");
    let body = axum::body::to_bytes(response.into_body(), 1 << 22)
        .await
        .unwrap();
    let mut reader = crate::tar::Reader::new(&body[..]);
    let mut names = Vec::new();
    while let Some(entry) = reader.next_entry().unwrap() {
        if entry.path == "www/sub/b c.txt" {
            let mut data = Vec::new();
            reader.copy_data(&mut data).unwrap();
            assert_eq!(data, vec![9u8; 70_000]);
        }
        names.push(entry.path);
    }
    let want = [
        "www",
        "www/a.txt",
        "www/in-link",
        "www/sub",
        "www/sub/b c.txt",
        "www/sub/deep",
        "www/sub/deep/ü.bin",
    ];
    assert_eq!(names, want);
    let file = base.path().join("www.tar");
    std::fs::write(&file, &body).unwrap();
    // GNU tar; another tar (no --quoting-style) leaves the own reader.
    match std::process::Command::new("tar")
        .args(["--quoting-style=literal", "-tf"])
        .arg(&file)
        .output()
    {
        Ok(out) if out.status.success() => {
            let listed: Vec<String> = String::from_utf8(out.stdout)
                .unwrap()
                .lines()
                .map(|l| l.trim_end_matches('/').to_string())
                .collect();
            assert_eq!(listed, want);
        }
        other => eprintln!("no GNU tar ({other:?}): only the own reader checks"),
    }
    // A sub directory, and HEAD.
    let (status, body) = get(&router, "/sub/deep/?tar").await;
    assert_eq!(status, 200);
    assert!(body.starts_with("deep/"), "{body:?}");
    // A file with ?tar is the file; a dot directory is not served.
    assert_eq!(get(&router, "/a.txt?tar").await, (200, "alpha".into()));
    // Without --tar, ?tar has no effect.
    let plain = static_router(&root, false).unwrap();
    assert_ne!(get(&plain, "/sub/?tar").await.1.get(..4), Some("sub/"));
}

/// A link out of the root, a relative link, a link to a directory out of
/// the root, a chain of links, a dotfile and a FIFO are not served. A
/// link in the root, a file and a directory index are served.
#[tokio::test]
async fn the_static_root_is_a_boundary() {
    use std::os::unix::fs::symlink;
    let base = tempfile::tempdir().unwrap();
    let root = base.path().join("www");
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::write(root.join("index.html"), "index").unwrap();
    std::fs::write(root.join("sub/page.html"), "page").unwrap();
    std::fs::write(root.join(".env"), "SECRET=1").unwrap();
    std::fs::write(base.path().join("outside.txt"), "outside").unwrap();
    std::fs::create_dir(base.path().join("outdir")).unwrap();
    std::fs::write(base.path().join("outdir/x.txt"), "outdir").unwrap();
    symlink(base.path().join("outside.txt"), root.join("abs-link")).unwrap();
    symlink("../outside.txt", root.join("rel-link")).unwrap();
    symlink(base.path().join("outdir"), root.join("dir-link")).unwrap();
    symlink("abs-link", root.join("chain-link")).unwrap();
    symlink("sub/page.html", root.join("in-link")).unwrap();
    let fifo = std::ffi::CString::new(root.join("fifo").to_str().unwrap()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o644) }, 0);

    let router = static_router(&root, false).unwrap();
    assert_eq!(get(&router, "/").await, (200, "index".into()));
    assert_eq!(get(&router, "/sub/page.html").await, (200, "page".into()));
    assert_eq!(get(&router, "/in-link").await, (200, "page".into()));
    for path in [
        "/abs-link",
        "/rel-link",
        "/dir-link/x.txt",
        "/chain-link",
        "/.env",
        "/%2eenv",
        "/sub/../../outside.txt",
        "/fifo",
        "/missing",
    ] {
        let (status, body) = get(&router, path).await;
        assert_eq!(status, 404, "{path}: {body}");
        // Issue 182: each 404 has one body; a missing path does not
        // look other than a refused one.
        assert_eq!(body, "Not Found", "{path}");
        assert!(
            !body.contains("outside") && !body.contains("SECRET"),
            "{path}: {body}"
        );
    }
    let long = format!("/{}", "a".repeat(300));
    assert_eq!(get(&router, &long).await.0, 404);

    // A multipart range gets the whole file.
    {
        use tower::ServiceExt;
        let request = http::Request::builder()
            .uri("/sub/page.html")
            .header("range", "bytes=0-0,2-3")
            .body(axum::body::Body::empty())
            .unwrap();
        let response = router.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), 200);
    }
    // --spa needs an index.html.
    let bare = base.path().join("bare");
    std::fs::create_dir(&bare).unwrap();
    let err = static_router(&bare, true).err().unwrap().to_string();
    assert!(err.contains("--spa needs an index.html"), "{err}");

    // An SPA route answers the index with 200; a link out of the root
    // is still refused.
    let spa = static_router(&root, true).unwrap();
    assert_eq!(get(&spa, "/some/route").await, (200, "index".into()));
    assert_eq!(get(&spa, "/abs-link").await.0, 404);
}

// T-207: the router of `serve` and the library origin decode a path alike.
#[test]
fn static_paths_agree_on_plus() {
    use crate::origin::static_files::percent_decode;
    for path in ["/a+b", "/a%20b", "/a%2Bb", "/%41", "/a%7e"] {
        assert_eq!(
            percent_decode_path(path).as_deref(),
            Some(percent_decode(path).as_str()),
            "{path}"
        );
    }
}

/// A GET with headers; the status, the headers and the body.
pub(crate) async fn send(
    router: &Router,
    request: http::Request<axum::body::Body>,
) -> (u16, http::HeaderMap, Vec<u8>) {
    use tower::ServiceExt;
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status().as_u16();
    let headers = response.headers().clone();
    let body = axum::body::to_bytes(response.into_body(), 1 << 22)
        .await
        .unwrap();
    (status, headers, body.to_vec())
}

/// A tree for the listing: a name with `<script>`, a dotfile, a dot
/// directory, a link in the root, a link out of the root and a FIFO.
fn listing_tree() -> (tempfile::TempDir, PathBuf) {
    use std::os::unix::fs::symlink;
    let base = tempfile::tempdir().unwrap();
    let root = base.path().join("www");
    std::fs::create_dir_all(root.join("sub dir")).unwrap();
    std::fs::create_dir_all(root.join(".git")).unwrap();
    std::fs::write(root.join("<script>alert(1).txt"), "x").unwrap();
    std::fs::write(root.join("a&b \"q\".txt"), "12345").unwrap();
    std::fs::write(root.join(".env"), "SECRET=1").unwrap();
    std::fs::write(base.path().join("outside.txt"), "outside").unwrap();
    symlink(base.path().join("outside.txt"), root.join("out-link")).unwrap();
    symlink(root.join("a&b \"q\".txt"), root.join("in-link")).unwrap();
    let fifo = std::ffi::CString::new(root.join("fifo").to_str().unwrap()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o644) }, 0);
    (base, root)
}

fn listed(root: &Path) -> Router {
    router(&FilesConfig {
        list: true,
        ..FilesConfig::new(root)
    })
    .unwrap()
}

/// T-088: the HTML listing escapes each name, and hides the dotfiles, a
/// link out of the root and a FIFO. A directory with no `/` at its end
/// goes to the path with the `/`. With no --list, a directory with no
/// index.html answers 404.
#[tokio::test]
async fn a_listing_escapes_names_and_hides_dotfiles() {
    let (_base, root) = listing_tree();
    let router = listed(&root);
    let (status, body) = get(&router, "/").await;
    assert_eq!(status, 200, "{body}");
    assert!(!body.contains("<script>alert"), "{body}");
    assert!(body.contains("&lt;script&gt;alert(1).txt"), "{body}");
    assert!(
        body.contains("href=\"%3Cscript%3Ealert%281%29.txt\""),
        "{body}"
    );
    assert!(body.contains("a&amp;b &quot;q&quot;.txt"), "{body}");
    assert!(body.contains("href=\"sub%20dir/\""), "{body}");
    assert!(body.contains("in-link"), "{body}");
    for hidden in [".env", ".git", "out-link", "fifo", "outside"] {
        assert!(!body.contains(hidden), "{hidden}: {body}");
    }
    // Each link of the listing is served.
    let (status, body) = get(&router, "/a%26b%20%22q%22.txt").await;
    assert_eq!((status, body.as_str()), (200, "12345"));
    // A sub directory, with and with no slash at its end.
    let (status, body) = get(&router, "/sub%20dir/").await;
    assert_eq!(status, 200);
    assert!(body.contains("Index of /sub dir/"), "{body}");
    assert!(body.contains("href=\"../\""), "{body}");
    let request = http::Request::get("/sub%20dir?x=1")
        .body(axum::body::Body::empty())
        .unwrap();
    let (status, headers, _) = send(&router, request).await;
    assert_eq!(status, 308);
    assert_eq!(headers["location"], "/sub%20dir/?x=1");
    // A dot directory is not listed.
    assert_eq!(get(&router, "/.git/").await.0, 404);
    // A directory with an index.html answers the index.
    std::fs::write(root.join("sub dir/index.html"), "index").unwrap();
    assert_eq!(get(&router, "/sub%20dir/").await, (200, "index".into()));
    // With no --list: 404 for a directory with no index.html.
    let plain = static_router(&root, false).unwrap();
    assert_eq!(get(&plain, "/").await, (404, "Not Found".into()));
}

/// T-088: `Accept: application/json` gives the listing as JSON, with the
/// same entries as the HTML listing.
#[tokio::test]
async fn a_listing_in_json() {
    let (_base, root) = listing_tree();
    let router = listed(&root);
    let request = http::Request::get("/")
        .header("accept", "text/html;q=0.5, application/json")
        .body(axum::body::Body::empty())
        .unwrap();
    let (status, headers, body) = send(&router, request).await;
    assert_eq!(status, 200);
    assert_eq!(headers["content-type"], "application/json");
    assert_eq!(headers["vary"], "accept");
    let doc: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(doc["path"], "/");
    let names: Vec<(&str, &str)> = doc["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| (e["name"].as_str().unwrap(), e["type"].as_str().unwrap()))
        .collect();
    assert_eq!(
        names,
        [
            ("sub dir", "dir"),
            ("<script>alert(1).txt", "file"),
            ("a&b \"q\".txt", "file"),
            ("in-link", "file"),
        ]
    );
    let file = &doc["entries"][2];
    assert_eq!(file["size"], 5);
    assert!(file["modified"].as_u64().unwrap() > 1_600_000_000, "{file}");
    // HEAD has the head and no body.
    let request = http::Request::head("/")
        .body(axum::body::Body::empty())
        .unwrap();
    let (status, headers, _) = send(&router, request).await;
    assert_eq!(status, 200);
    assert!(headers["content-type"]
        .to_str()
        .unwrap()
        .starts_with("text/html"));
}
