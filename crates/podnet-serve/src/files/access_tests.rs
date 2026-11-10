//! The tests of the access of the file server (T-089).

use super::*;
use crate::files::tests::send;
use crate::files::{router, FilesConfig};
use base64::Engine as _;

fn basic(user: &str, password: &str) -> String {
    let token = base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}"));
    format!("Basic {token}")
}

fn request(method: &str, path: &str, auth: Option<&str>) -> http::Request<Body> {
    let mut builder = http::Request::builder().method(method).uri(path);
    if let Some(auth) = auth {
        builder = builder.header("authorization", auth);
    }
    builder.body(Body::empty()).unwrap()
}

/// A tree with one file, and a users file with a hash user (read) and a
/// password-file user (write), each file of mode 0600.
fn setup() -> (tempfile::TempDir, std::path::PathBuf, Users) {
    use std::os::unix::fs::PermissionsExt;
    let base = tempfile::tempdir().unwrap();
    let root = base.path().join("www");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("a.txt"), "alpha").unwrap();
    let private = |path: &std::path::Path, text: &str| {
        std::fs::write(path, text).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    };
    private(&base.path().join("bob.pass"), "bob-secret\n");
    let users = base.path().join("users");
    private(
        &users,
        &format!(
            "# the users\n\nalice:read:{}\nbob:write:@bob.pass\n",
            hash_password_with("alice-secret", 1000)
        ),
    );
    let loaded = Users::load(&users).unwrap();
    (base, root, loaded)
}

/// T-089: a read user reads and cannot change, a write user can, a wrong
/// password and no credential get 401 with a Basic challenge. With
/// --anon-read, a GET needs no user; a change still needs a user.
#[tokio::test]
async fn serve_users_have_roles() {
    let (base, root, users) = setup();
    assert!(!format!("{users:?}").contains("secret"), "{users:?}");
    assert!(!format!("{users:?}").contains("pbkdf2"), "{users:?}");
    assert_eq!(users.check("alice", "alice-secret"), Some(Role::Read));
    // The second check comes from the cache, with the same answer.
    assert_eq!(users.check("alice", "alice-secret"), Some(Role::Read));
    assert_eq!(users.check("alice", "alice-secreT"), None);
    assert_eq!(users.check("bob", "bob-secret"), Some(Role::Write));
    assert_eq!(users.check("carol", "x"), None);
    let files = |anon_read| {
        router(&FilesConfig {
            access: Access {
                users: Some(users.clone()),
                pin: None,
                anon_read,
            },
            ..FilesConfig::new(&root)
        })
        .unwrap()
    };
    let strict = files(false);
    let alice = basic("alice", "alice-secret");
    let bob = basic("bob", "bob-secret");
    let (status, _, body) = send(&strict, request("GET", "/a.txt", Some(&alice))).await;
    assert_eq!((status, body.as_slice()), (200, &b"alpha"[..]));
    let (status, _, _) = send(&strict, request("GET", "/a.txt", Some(&bob))).await;
    assert_eq!(status, 200);
    for auth in [None, Some(basic("alice", "wrong")), Some("Bearer x".into())] {
        let (status, headers, body) =
            send(&strict, request("GET", "/a.txt", auth.as_deref())).await;
        assert_eq!(status, 401, "{auth:?}");
        assert!(
            headers["www-authenticate"]
                .to_str()
                .unwrap()
                .starts_with("Basic realm=\"podnet\""),
            "{headers:?}"
        );
        assert!(!String::from_utf8_lossy(&body).contains("alpha"));
    }
    // A change needs the write role: the read user gets 403. With no
    // upload, the write user gets 405 from the file server.
    let (status, _, _) = send(&strict, request("PUT", "/a.txt", Some(&alice))).await;
    assert_eq!(status, 403);
    let (status, _, _) = send(&strict, request("PUT", "/a.txt", Some(&bob))).await;
    assert_eq!(status, 405);
    let anon = files(true);
    let (status, _, body) = send(&anon, request("GET", "/a.txt", None)).await;
    assert_eq!((status, body.as_slice()), (200, &b"alpha"[..]));
    let (status, _, _) = send(&anon, request("PUT", "/a.txt", None)).await;
    assert_eq!(status, 401);

    // A bad users file names its line; a file open to others is refused.
    let bad = Users::parse("alice:admin:x\n", base.path()).unwrap_err();
    assert!(format!("{bad:#}").contains("line 1"), "{bad:#}");
    let bad = Users::parse("alice:read:pbkdf2-sha256$0$00$00\n", base.path()).unwrap_err();
    assert!(format!("{bad:#}").contains("iterations"), "{bad:#}");
    assert!(Users::parse("a:read:@nofile\n", base.path()).is_err());
    assert!(Users::parse("# none\n", base.path()).is_err());
    let open = base.path().join("open-users");
    std::fs::write(&open, "alice:read:@bob.pass\n").unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o644)).unwrap();
    }
    assert!(Users::load(&open).is_err());
    // A hash of hash_password checks.
    let hashed = Users::parse(&format!("u:write:{}\n", hash_password("pw")), base.path()).unwrap();
    assert_eq!(hashed.check("u", "pw"), Some(Role::Write));
}

/// T-089: --pin locks each path with the PIN page. A right PIN gives the
/// session cookie, and the cookie opens the files. A wrong PIN and an
/// unlock from another site are refused. With --anon-read, a GET needs no
/// PIN; a change does.
#[tokio::test]
async fn serve_pin_unlocks() {
    let (_base, root, _users) = setup();
    let gate = Gate::pin();
    let pin = gate.pin_display().unwrap().to_string();
    let files = |anon_read| {
        router(&FilesConfig {
            access: Access {
                users: None,
                pin: Some(gate.clone()),
                anon_read,
            },
            ..FilesConfig::new(&root)
        })
        .unwrap()
    };
    let locked = files(false);
    let (status, _, body) = send(&locked, request("GET", "/a.txt", None)).await;
    assert_eq!(status, 200);
    let body = String::from_utf8_lossy(&body).into_owned();
    assert!(body.contains("Enter the 6-digit code"), "{body}");
    assert!(!body.contains("alpha") && !body.contains(&pin), "{body}");
    let (status, _, _) = send(&locked, request("PUT", "/a.txt", None)).await;
    assert_eq!(status, 403);

    let unlock = |password: &str, origin: Option<&str>| {
        let mut builder = http::Request::post(UNLOCK_PATH)
            .header("content-type", "application/x-www-form-urlencoded")
            .header("host", "files.test");
        if let Some(origin) = origin {
            builder = builder.header("origin", origin);
        }
        builder
            .body(Body::from(format!("password={password}&next=/a.txt")))
            .unwrap()
    };
    let (status, _, _) = send(&locked, unlock("000000x", None)).await;
    assert_eq!(status, 403);
    let (status, _, _) = send(&locked, unlock(&pin, Some("https://evil.test"))).await;
    assert_eq!(status, 403);
    let (status, headers, _) = send(&locked, unlock(&pin, Some("https://files.test"))).await;
    assert_eq!(status, 303, "{headers:?}");
    assert_eq!(headers["location"], "/a.txt");
    let cookie = headers["set-cookie"].to_str().unwrap();
    let cookie = cookie.split(';').next().unwrap().to_string();
    let open = http::Request::get("/a.txt")
        .header("cookie", &cookie)
        .body(Body::empty())
        .unwrap();
    let (status, _, body) = send(&locked, open).await;
    assert_eq!((status, body.as_slice()), (200, &b"alpha"[..]));
    // A GET of the unlock path is not a try.
    let (status, _, _) = send(&locked, request("GET", UNLOCK_PATH, None)).await;
    assert_eq!(status, 405);

    let anon = files(true);
    let (status, _, body) = send(&anon, request("GET", "/a.txt", None)).await;
    assert_eq!((status, body.as_slice()), (200, &b"alpha"[..]));
    let (status, _, _) = send(&anon, request("PUT", "/a.txt", None)).await;
    assert_eq!(status, 403);
    // A PIN with no users gives the write role.
    let access = Access {
        users: None,
        pin: Some(gate.clone()),
        anon_read: false,
    };
    assert!(access.can_write());
    assert!(!Access::default().can_write());
}
