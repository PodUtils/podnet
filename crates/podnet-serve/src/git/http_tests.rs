//! The tests of the smart HTTP router and the CGI runner (T-092). The
//! test with the `git` client needs a loopback interface; the other tests
//! send each request to the router in the process. A test that needs
//! `git` is skipped, with one line, when `git` is not on `PATH`.

use std::path::Path;
use std::process::{Command, Output};

use axum::body::Body;
use base64::Engine as _;

use super::cgi::{header_variable, parse_head};
use super::http::{parse, router, Call, HttpOptions};
use super::Service;
use crate::files::access::hash_password_with;
use crate::files::{Access, Users};

/// True (with one line) when `git` is not on `PATH`.
fn git_missing() -> bool {
    let missing = Command::new("git").arg("--version").output().is_err();
    if missing {
        eprintln!("skipped: git is not on PATH");
    }
    missing
}

fn basic(user: &str, password: &str) -> String {
    let token = base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}"));
    format!("Basic {token}")
}

/// The options for the root of `make_root`: the user `pusher` (password
/// `pw`) with the write role, and an anonymous read.
fn options(root: &Path) -> HttpOptions {
    let hash = hash_password_with("pw", 1000);
    let users = Users::parse(&format!("pusher:write:{hash}\n"), root).unwrap();
    HttpOptions {
        access: Access {
            users: Some(users),
            pin: None,
            anon_read: true,
        },
        ..HttpOptions::new(root)
    }
}

fn get(path: &str, auth: Option<&str>) -> http::Request<Body> {
    let mut builder = http::Request::builder().uri(path);
    if let Some(auth) = auth {
        builder = builder.header("authorization", auth);
    }
    builder.body(Body::empty()).unwrap()
}

#[test]
fn parse_takes_the_smart_protocol_only() {
    let uri = |s: &str| s.parse::<http::Uri>().unwrap();
    let (get, post) = (http::Method::GET, http::Method::POST);
    assert_eq!(
        parse(&get, &uri("/a/b.git/info/refs?service=git-upload-pack")),
        Some(("/a/b.git".into(), Call::Refs(Service::UploadPack)))
    );
    assert_eq!(
        parse(&get, &uri("/r/info/refs?x=1&service=git-receive-pack")),
        Some(("/r".into(), Call::Refs(Service::ReceivePack)))
    );
    assert_eq!(
        parse(&post, &uri("/r%20s/git-receive-pack")),
        Some(("/r s".into(), Call::Rpc(Service::ReceivePack)))
    );
    for (method, path) in [
        (&get, "/r/info/refs"),
        (&get, "/r/info/refs?service=git-upload-archive"),
        (&post, "/r/info/refs?service=git-upload-pack"),
        (&get, "/r/git-upload-pack"),
        (&post, "/r/git-upload-archive"),
        (&get, "/r/HEAD"),
        (&get, "/.git/config"),
        (&get, "/r/%zz/info/refs?service=git-upload-pack"),
    ] {
        assert_eq!(parse(method, &uri(path)), None, "{method} {path}");
    }
}

#[test]
fn cgi_head_and_variables() {
    let response = parse_head(b"Status: 404 Not Found\r\nContent-Type: text/plain\r\n").unwrap();
    assert_eq!(response.status(), 404);
    assert_eq!(response.headers()["content-type"], "text/plain");
    let response = parse_head(b"Location: /x\n").unwrap();
    assert_eq!(response.status(), 302);
    let response = parse_head(b"Expires: Fri, 01 Jan 1980 00:00:00 GMT\n").unwrap();
    assert_eq!(response.status(), 200);
    assert!(parse_head(b"Status: abc\n").is_err());
    assert!(parse_head(b"no colon\n").is_err());
    assert_eq!(header_variable("git-protocol"), "HTTP_GIT_PROTOCOL");
    let request = http::Request::builder()
        .method("POST")
        .uri("/r/git-upload-pack?a=b")
        .header("authorization", "Basic eDp5")
        .header("cookie", "k=v")
        .header("content-type", "application/x-git-upload-pack-request")
        .header("content-encoding", "gzip")
        .header("transfer-encoding", "chunked")
        .header("content-length", "9")
        .header("git-protocol", "version=2")
        .body(Body::empty())
        .unwrap();
    let env: Vec<(String, String)> = super::cgi::meta_variables(&request)
        .into_iter()
        .map(|(k, v)| (k.into_string().unwrap(), v.into_string().unwrap()))
        .collect();
    let value = |name: &str| env.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str());
    assert_eq!(value("REQUEST_METHOD"), Some("POST"));
    assert_eq!(value("QUERY_STRING"), Some("a=b"));
    assert_eq!(value("HTTP_GIT_PROTOCOL"), Some("version=2"));
    assert_eq!(value("HTTP_CONTENT_ENCODING"), Some("gzip"));
    assert_eq!(
        value("CONTENT_TYPE"),
        Some("application/x-git-upload-pack-request")
    );
    // A chunked body has no CONTENT_LENGTH. The credentials stay out.
    assert_eq!(value("CONTENT_LENGTH"), None);
    assert_eq!(value("HTTP_AUTHORIZATION"), None);
    assert_eq!(value("HTTP_COOKIE"), None);
    assert_eq!(value("REMOTE_USER"), None);
}

/// T-092: the refusals of the router, with no network: `..`, a dot
/// segment, `/.git/config` and a dumb path give 404; a push with no
/// credentials and with a wrong password gives 401 with a challenge.
#[tokio::test]
async fn git_http_refuses_paths_and_bad_credentials() {
    if git_missing() {
        return;
    }
    let dir = super::tests::make_root();
    let app = router(options(dir.path())).unwrap();
    let send = |request| crate::files::tests::send(&app, request);
    for path in [
        "/../repo.git/info/refs?service=git-upload-pack",
        "/%2e%2e/repo.git/info/refs?service=git-upload-pack",
        "/repo.git/.git/info/refs?service=git-upload-pack",
        "/.git/config",
        "/repo.git/config",
        "/repo.git/HEAD",
        "/nothing/info/refs?service=git-upload-pack",
    ] {
        let (status, _, _) = send(get(path, None)).await;
        assert_eq!(status, 404, "{path}");
    }
    let refs = "/repo.git/info/refs?service=git-receive-pack";
    for auth in [None, Some(basic("pusher", "bad")), Some(basic("x", "pw"))] {
        let (status, headers, _) = send(get(refs, auth.as_deref())).await;
        assert_eq!(status, 401, "{auth:?}");
        assert!(headers["www-authenticate"]
            .to_str()
            .unwrap()
            .starts_with("Basic "));
    }
    let (status, headers, body) = send(get(refs, Some(&basic("pusher", "pw")))).await;
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
    assert_eq!(
        headers["content-type"],
        "application/x-git-receive-pack-advertisement"
    );
    let (status, headers, body) = send(get("/repo/info/refs?service=git-upload-pack", None)).await;
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
    assert_eq!(
        headers["content-type"],
        "application/x-git-upload-pack-advertisement"
    );
    assert!(body.starts_with(b"001e# service=git-upload-pack\n"));
}

/// A `git` command with no user and no system configuration, no proxy and
/// no prompt.
fn git_in(dir: &Path, home: &Path, args: &[&str], envs: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new("git");
    cmd.current_dir(dir).args(args);
    for name in [
        "http_proxy",
        "HTTP_PROXY",
        "https_proxy",
        "HTTPS_PROXY",
        "all_proxy",
        "ALL_PROXY",
        "GIT_DIR",
    ] {
        cmd.env_remove(name);
    }
    cmd.env("HOME", home)
        .env("XDG_CONFIG_HOME", home)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("NO_PROXY", "*")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .envs(envs.iter().copied());
    cmd.output().unwrap()
}

/// Assert that `out` is a success, and give its stderr.
fn ok(out: Output, what: &str) -> String {
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(out.status.success(), "{what}: {stderr}");
    stderr
}

/// Start the router of `options` on 127.0.0.1 in a thread of its own.
fn start(options: HttpOptions) -> u16 {
    let app = router(options).unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async move {
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            axum::serve(listener, app).await.unwrap();
        });
    });
    port
}

/// Bytes that do not compress (xorshift).
fn noise(len: usize) -> Vec<u8> {
    let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

/// T-092: the `git` client clones, fetches with protocol v2 (also with a
/// gzip body over 1 KiB), and pushes with credentials (also 8 MiB as a
/// chunked body). An anonymous push and a wrong password fail.
#[test]
fn git_http_clone_fetch_and_push() {
    if git_missing() {
        return;
    }
    pod_testkit::need_loopback!();
    let dir = super::tests::make_root();
    let root = dir.path();
    let home = tempfile::tempdir().unwrap();
    let home = home.path();
    let port = start(options(root));
    let anon = format!("http://127.0.0.1:{port}/repo.git");
    let creds = format!("http://pusher:pw@127.0.0.1:{port}/repo");
    let wrong = format!("http://pusher:bad@127.0.0.1:{port}/repo.git");
    let git = |dir: &Path, args: &[&str]| git_in(dir, home, args, &[]);

    ok(git(home, &["clone", "-q", &anon, "work"]), "clone");
    ok(git(home, &["clone", "-q", &anon, "other"]), "clone");
    let work = home.join("work");
    let other = home.join("other");
    std::fs::write(work.join("big.bin"), noise(8 << 20)).unwrap();
    ok(git(&work, &["add", "big.bin"]), "add");
    ok(git(&work, &["commit", "-q", "-m", "big"]), "commit");

    let out = git(&work, &["push", &anon, "main"]);
    assert!(!out.status.success(), "an anonymous push passes");
    let out = git(&work, &["push", &wrong, "main"]);
    assert!(!out.status.success(), "a push with a wrong password passes");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("Authentication failed"), "{stderr}");

    let trace = [("GIT_TRACE_CURL", "1"), ("GIT_TRACE_CURL_NO_DATA", "1")];
    let args = ["-c", "http.postBuffer=1024", "push", "-q", &creds, "main"];
    let stderr = ok(git_in(&work, home, &args, &trace), "push");
    assert!(
        stderr.contains("Transfer-Encoding: chunked"),
        "the push has no chunked body"
    );
    let head = |dir: &Path, rev: &str| {
        let out = git(dir, &["rev-parse", rev]);
        String::from_utf8_lossy(&ok_out(out)).trim().to_string()
    };
    let pushed = head(&work, "HEAD");
    let bare = root.join("repo.git");
    assert_eq!(head(&bare, "refs/heads/main"), pushed);

    // 100 local commits: the haves of the fetch are more than 1 KiB.
    for n in 0..100 {
        let message = format!("local {n}");
        ok(
            git(&other, &["commit", "-q", "--allow-empty", "-m", &message]),
            "commit",
        );
    }
    let trace = [
        ("GIT_TRACE_CURL", "1"),
        ("GIT_TRACE_CURL_NO_DATA", "1"),
        ("GIT_TRACE_PACKET", "1"),
    ];
    let args = ["-c", "protocol.version=2", "fetch", "-q", "origin"];
    let stderr = ok(git_in(&other, home, &args, &trace), "fetch");
    assert!(stderr.contains("git< version 2"), "no protocol v2");
    assert!(
        stderr.contains("Content-Encoding: gzip"),
        "the fetch has no gzip body"
    );
    assert_eq!(head(&other, "origin/main"), pushed);
}

fn ok_out(out: Output) -> Vec<u8> {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}

/// T-094: with `create_on_push`, a push of a write user to a new path
/// makes a bare repository; a fetch of a new path does not.
#[tokio::test]
async fn git_http_create_on_push() {
    if git_missing() {
        return;
    }
    let dir = super::tests::make_root();
    let app = router(HttpOptions {
        create_on_push: true,
        ..options(dir.path())
    })
    .unwrap();
    let send = |request| crate::files::tests::send(&app, request);
    let (status, _, _) = send(get("/new/info/refs?service=git-upload-pack", None)).await;
    assert_eq!(status, 404);
    assert!(!dir.path().join("new.git").exists());
    let refs = "/team/new/info/refs?service=git-receive-pack";
    let (status, _, _) = send(get(refs, None)).await;
    assert_eq!(status, 401);
    assert!(!dir.path().join("team").exists());
    let (status, _, body) = send(get(refs, Some(&basic("pusher", "pw")))).await;
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
    assert!(dir.path().join("team/new.git/HEAD").is_file());
}
