//! The tests of the uploads into the tree (T-090).

use super::*;
use crate::files::tests::send;
use crate::files::{router, Access, FilesConfig, Role, Users};
use base64::Engine as _;

fn basic(user: &str, password: &str) -> String {
    let token = base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}"));
    format!("Basic {token}")
}

/// The names in `dir`, sorted (also the hidden part files).
fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

/// A tree with `sub/`, a writer `w:pw` and a reader `r:pw`, uploads of at
/// most 1000 bytes.
fn setup() -> (tempfile::TempDir, PathBuf, axum::Router) {
    let base = tempfile::tempdir().unwrap();
    let root = base.path().join("www");
    std::fs::create_dir_all(root.join("sub")).unwrap();
    let mut users = Users::default();
    users
        .add(pod_proxy::Credentials::new("w", "pw").unwrap(), Role::Write)
        .unwrap();
    users
        .add(pod_proxy::Credentials::new("r", "pw").unwrap(), Role::Read)
        .unwrap();
    let files = router(&FilesConfig {
        list: true,
        upload: Some(1000),
        access: Access {
            users: Some(users),
            pin: None,
            anon_read: false,
        },
        ..FilesConfig::new(&root)
    })
    .unwrap();
    (base, root, files)
}

fn put(path: &str, user: &str, body: Body) -> http::Request<Body> {
    http::Request::put(path)
        .header("authorization", basic(user, "pw"))
        .body(body)
        .unwrap()
}

/// T-090: a PUT gives 201 (and 204 when it replaces a file). A PUT over
/// the limit gives 413, by its Content-Length before the body or while
/// the body streams, and leaves no file and no part file. A PUT with `..`,
/// a dot name, a missing directory or a read user is refused.
#[tokio::test]
async fn put_into_the_tree_is_atomic_and_bounded() {
    let (_base, root, files) = setup();
    let (status, _, _) = send(&files, put("/sub/a.txt", "w", Body::from("alpha"))).await;
    assert_eq!(status, 201);
    assert_eq!(std::fs::read(root.join("sub/a.txt")).unwrap(), b"alpha");
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(root.join("sub/a.txt"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    let (status, _, _) = send(&files, put("/sub/a.txt", "w", Body::from("beta"))).await;
    assert_eq!(status, 204);
    assert_eq!(std::fs::read(root.join("sub/a.txt")).unwrap(), b"beta");
    // A name with a space, percent-encoded.
    let (status, _, _) = send(&files, put("/b%20c.bin", "w", Body::from(vec![7u8; 1000]))).await;
    assert_eq!(status, 201);
    assert_eq!(
        std::fs::read(root.join("b c.bin")).unwrap(),
        vec![7u8; 1000]
    );

    // Over the limit by Content-Length: 413 before the body.
    let request = http::Request::put("/big.bin")
        .header("authorization", basic("w", "pw"))
        .header("content-length", "1001")
        .body(Body::from(vec![0u8; 1001]))
        .unwrap();
    let (status, _, _) = send(&files, request).await;
    assert_eq!(status, 413);
    // Over the limit while it streams (no Content-Length).
    let chunks: Vec<Result<bytes::Bytes, std::io::Error>> = (0..3)
        .map(|_| Ok(bytes::Bytes::from(vec![1u8; 400])))
        .collect();
    let stream = Body::from_stream(futures::stream::iter(chunks));
    let (status, _, _) = send(&files, put("/big.bin", "w", stream)).await;
    assert_eq!(status, 413);
    // A body that fails part way leaves no file.
    let chunks: Vec<Result<bytes::Bytes, std::io::Error>> = vec![
        Ok(bytes::Bytes::from_static(b"part")),
        Err(std::io::Error::other("cut")),
    ];
    let stream = Body::from_stream(futures::stream::iter(chunks));
    let (status, _, _) = send(&files, put("/cut.bin", "w", stream)).await;
    assert_eq!(status, 400);
    assert_eq!(names(&root), ["b c.bin", "sub"]);
    assert_eq!(names(&root.join("sub")), ["a.txt"]);

    // Refused paths.
    for (path, want) in [
        ("/sub/../x.txt", 403),
        ("/%2e%2e/x.txt", 403),
        ("/.env", 403),
        ("/sub/.hidden", 403),
        ("/missing/x.txt", 409),
        ("/sub", 409),
        ("/sub/", 405),
    ] {
        let (status, _, body) = send(&files, put(path, "w", Body::from("x"))).await;
        assert_eq!(status, want, "{path}: {}", String::from_utf8_lossy(&body));
    }
    let (status, _, _) = send(&files, put("/sub/r.txt", "r", Body::from("x"))).await;
    assert_eq!(status, 403);
    assert_eq!(names(&root), ["b c.bin", "sub"]);
    assert_eq!(names(&root.join("sub")), ["a.txt"]);
    // A link out of the root is not a directory of the tree.
    std::os::unix::fs::symlink(root.parent().unwrap(), root.join("up")).unwrap();
    let (status, _, _) = send(&files, put("/up/x.txt", "w", Body::from("x"))).await;
    assert_eq!(status, 409);
    assert!(!root.parent().unwrap().join("x.txt").exists());
}

/// T-090: a multipart form to a directory saves each file; a name in use
/// gets a number. A form over the limit saves nothing. A write user gets
/// the form in the listing, a read user does not.
#[tokio::test]
async fn a_form_uploads_into_a_directory() {
    let (_base, root, files) = setup();
    std::fs::write(root.join("sub/a.txt"), "old").unwrap();
    let form = |files: &[(&str, &[u8])]| {
        let mut body = Vec::new();
        for (name, data) in files {
            body.extend_from_slice(
                format!(
                    "--XX\r\nContent-Disposition: form-data; name=\"files\"; filename=\"{name}\"\r\n\r\n"
                )
                .as_bytes(),
            );
            body.extend_from_slice(data);
            body.extend_from_slice(b"\r\n");
        }
        body.extend_from_slice(b"--XX--\r\n");
        http::Request::post("/sub/")
            .header("authorization", basic("w", "pw"))
            .header("content-type", "multipart/form-data; boundary=XX")
            .body(Body::from(body))
            .unwrap()
    };
    let (status, _, body) = send(&files, form(&[("a.txt", b"new"), ("../b.txt", b"bee")])).await;
    assert_eq!(status, 201, "{}", String::from_utf8_lossy(&body));
    assert_eq!(names(&root.join("sub")), ["a-1.txt", "a.txt", "b.txt"]);
    assert_eq!(std::fs::read(root.join("sub/a.txt")).unwrap(), b"old");
    assert_eq!(std::fs::read(root.join("sub/a-1.txt")).unwrap(), b"new");
    let (status, _, _) = send(&files, form(&[("c.bin", &[0u8; 1200])])).await;
    assert_eq!(status, 413);
    assert_eq!(names(&root.join("sub")), ["a-1.txt", "a.txt", "b.txt"]);

    let listing = |user: &str| {
        http::Request::get("/")
            .header("authorization", basic(user, "pw"))
            .body(Body::empty())
            .unwrap()
    };
    let (_, _, body) = send(&files, listing("w")).await;
    assert!(String::from_utf8_lossy(&body).contains("multipart/form-data"));
    let (_, _, body) = send(&files, listing("r")).await;
    assert!(!String::from_utf8_lossy(&body).contains("multipart/form-data"));

    // Uploads need a write user or a PIN.
    let err = router(&FilesConfig {
        upload: Some(1),
        ..FilesConfig::new(&root)
    })
    .err()
    .unwrap();
    assert!(err.to_string().contains("--upload needs"), "{err}");
}
