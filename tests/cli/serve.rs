//! `podnet serve` and `podnet share`.

use super::*;

/// Issue 250: a bare `podnet share` names the modes, and starts nothing.
#[test]
fn a_bare_share_names_the_modes() {
    let out = Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(["share", "--check"])
        .env("PODNET_EDGE", "bogus::")
        .env_remove("TUNNEL_TOKEN")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(64));
    let text = stderr(&out);
    for flag in ["--mode upload", "--dir", "--file", "--port", "--unix"] {
        assert!(text.contains(flag), "{text}");
    }
}

/// Issue 204: share --mode proxy takes a unix socket origin.
#[test]
fn share_proxy_takes_a_unix_socket() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("api.sock");
    let _listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
    let sock = sock.to_str().unwrap();
    let out = podnet(&["share", "--unix", sock, "--check", "--json"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let service = doc["plan"]["ingress"][0]["service"].as_str().unwrap();
    assert!(service.contains(&format!("unix:{sock}")), "{doc}");
    let out = podnet(&["share", "--unix", sock, "--port", "1", "--check"]);
    assert_eq!(out.status.code(), Some(64), "{}", stderr(&out));
    let out = podnet(&["share", "--unix", "/nonexistent.sock", "--check"]);
    assert_eq!(out.status.code(), Some(69), "{}", stderr(&out));
}

/// Start `podnet serve --dir DIR --local ADDR` and give the child and its
/// first stdout line (issue 307).
pub(super) fn serve_local(dir: &std::path::Path, addr: &str) -> (std::process::Child, String) {
    use std::io::BufRead;
    let mut child = Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(["serve", "--dir", dir.to_str().unwrap(), "--local", addr])
        .env_remove("PODNET_ERROR_PROGRAM")
        .env_remove("TUNNEL_TOKEN")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let _ = std::io::BufReader::new(stdout).read_line(&mut line);
        let _ = tx.send(line);
    });
    let line = rx
        .recv_timeout(std::time::Duration::from_secs(20))
        .unwrap_or_default();
    (child, line)
}

/// One GET over `stream`; the status line and the body.
pub(super) fn http_get(
    mut stream: impl std::io::Read + std::io::Write,
    path: &str,
) -> (String, Vec<u8>) {
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut reply = Vec::new();
    stream.read_to_end(&mut reply).unwrap();
    let split = reply
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("a head");
    let head = String::from_utf8_lossy(&reply[..split]).into_owned();
    let status = head.lines().next().unwrap_or_default().to_string();
    (status, reply[split + 4..].to_vec())
}

/// Stop the child with SIGTERM: exit 0.
pub(super) fn stop(mut child: std::process::Child) {
    unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
    let status = child.wait().unwrap();
    assert_eq!(status.code(), Some(0), "{status:?}");
}

/// Issue 307: `serve --local 127.0.0.1:0` serves the files on a local
/// TCP listener with no tunnel, and prints the bound port.
#[test]
fn serve_local_serves_files_on_tcp() {
    test_host::need_loopback!();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("index.html"), "hello local").unwrap();
    std::fs::write(dir.path().join(".env"), "SECRET=1").unwrap();
    let (child, line) = serve_local(dir.path(), "127.0.0.1:0");
    let url = line
        .split_whitespace()
        .find(|w| w.starts_with("http://127.0.0.1:"))
        .unwrap_or_else(|| panic!("no bound address in {line:?}"));
    let addr = url
        .trim_start_matches("http://")
        .trim_end_matches('/')
        .to_string();
    assert!(!addr.ends_with(":0"), "{line}");
    let (status, body) = http_get(std::net::TcpStream::connect(&addr).unwrap(), "/");
    assert!(status.contains(" 200 "), "{status}");
    assert_eq!(body, b"hello local");
    let (status, _) = http_get(std::net::TcpStream::connect(&addr).unwrap(), "/.env");
    assert!(status.contains(" 404 "), "{status}");
    stop(child);

    let dir_arg = dir.path().to_str().unwrap();
    // A bad address is a usage error.
    let out = podnet(&["serve", "--dir", dir_arg, "--local", "nowhere"]);
    assert_eq!(out.status.code(), Some(64), "{}", stderr(&out));
    // --check has no meaning with --local.
    let out = podnet(&[
        "serve",
        "--dir",
        dir_arg,
        "--local",
        "127.0.0.1:0",
        "--check",
    ]);
    assert_eq!(out.status.code(), Some(64), "{}", stderr(&out));
}

/// Issue 307: `serve --local unix:PATH` serves the files on a unix socket,
/// with the socket rule of docs/decisions.md: a live socket is refused
/// (69), a file that is not a socket is refused (64), a stale socket is
/// removed. The socket is removed at the end.
#[test]
fn serve_local_serves_files_on_a_unix_socket() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "over unix").unwrap();
    let sock = dir.path().join("s.sock");
    let addr = format!("unix:{}", sock.display());
    let dir_arg = dir.path().to_str().unwrap();

    // A stale socket: bound, then the listener is closed.
    drop(std::os::unix::net::UnixListener::bind(&sock).unwrap());
    assert!(sock.exists());
    let (child, line) = serve_local(dir.path(), &addr);
    assert!(line.contains(&addr), "{line:?}");
    let (status, body) = http_get(
        std::os::unix::net::UnixStream::connect(&sock).unwrap(),
        "/a.txt",
    );
    assert!(status.contains(" 200 "), "{status}");
    assert_eq!(body, b"over unix");

    // A live socket (this server) is refused.
    let out = podnet(&["serve", "--dir", dir_arg, "--local", &addr]);
    assert_eq!(out.status.code(), Some(69), "{}", stderr(&out));
    stop(child);
    assert!(!sock.exists(), "the socket stays after the stop");

    // A file that is not a socket is refused and stays.
    std::fs::write(&sock, "keep").unwrap();
    let out = podnet(&["serve", "--dir", dir_arg, "--local", &addr]);
    assert_eq!(out.status.code(), Some(64), "{}", stderr(&out));
    assert!(stderr(&out).contains("is not a socket"), "{}", stderr(&out));
    assert_eq!(std::fs::read_to_string(&sock).unwrap(), "keep");
}

/// One request over `stream` with extra header lines; the status line, the
/// head and the body.
pub(super) fn http_send(
    mut stream: impl std::io::Read + std::io::Write,
    method: &str,
    path: &str,
    headers: &[&str],
    body: &[u8],
) -> (String, String, Vec<u8>) {
    let mut head = format!(
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    for line in headers {
        head.push_str(line);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).unwrap();
    // A server can answer (413) and close before the end of the body.
    let _ = stream.write_all(body);
    let mut reply = Vec::new();
    let _ = stream.read_to_end(&mut reply);
    let split = reply
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("a head");
    let head = String::from_utf8_lossy(&reply[..split]).into_owned();
    let status = head.lines().next().unwrap_or_default().to_string();
    let body = reply[split + 4..].to_vec();
    if head
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked")
    {
        return (status, head, dechunk(&body));
    }
    (status, head, body)
}

/// The data of a chunked body (RFC 9112, section 7.1).
fn dechunk(mut body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let end = body
            .windows(2)
            .position(|w| w == b"\r\n")
            .expect("a size line");
        let size = std::str::from_utf8(&body[..end]).unwrap();
        let size = usize::from_str_radix(size.split(';').next().unwrap().trim(), 16).unwrap();
        body = &body[end + 2..];
        if size == 0 {
            return out;
        }
        out.extend_from_slice(&body[..size]);
        body = &body[size + 2..];
    }
}

/// Start `podnet serve` with `args` and give the child and its first
/// stdout line.
pub(super) fn serve_with(args: &[&str]) -> (std::process::Child, String) {
    use std::io::BufRead;
    let mut child = Command::new(env!("CARGO_BIN_EXE_podnet"))
        .arg("serve")
        .args(args)
        .env_remove("PODNET_ERROR_PROGRAM")
        .env_remove("TUNNEL_TOKEN")
        .env_remove("PODNET_AUTH_USER")
        .env_remove("PODNET_AUTH_PASSWORD_FILE")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let _ = std::io::BufReader::new(stdout).read_line(&mut line);
        let _ = tx.send(line);
    });
    let line = rx
        .recv_timeout(std::time::Duration::from_secs(20))
        .unwrap_or_default();
    (child, line)
}

/// Write `text` to `path` with mode 0600.
pub(super) fn private_file(path: &std::path::Path, text: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, text).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
}

/// T-089: `serve --local unix:PATH --users FILE` checks the users on the
/// local listener; --auth-user adds a write user there too. A hash of
/// --hash-password is a valid secret. --pin prints the PIN and locks the
/// files.
#[test]
fn serve_local_checks_users_and_the_pin() {
    use std::io::Write as _;
    let base = tempfile::tempdir().unwrap();
    let dir = base.path().join("www");
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("a.txt"), "alpha").unwrap();
    let mut hash = Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(["serve", "--hash-password"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    hash.stdin.take().unwrap().write_all(b"pw-one\n").unwrap();
    let out = hash.wait_with_output().unwrap();
    assert!(out.status.success());
    let hash = String::from_utf8(out.stdout).unwrap();
    assert!(hash.starts_with("pbkdf2-sha256$"), "{hash}");
    let users = base.path().join("users");
    private_file(&users, &format!("reader:read:{}", hash.trim()));
    let pass = base.path().join("pass");
    private_file(&pass, "pw-two\n");
    let sock = base.path().join("s.sock");
    let addr = format!("unix:{}", sock.display());
    let (dir_arg, users_arg, pass_arg) = (
        dir.to_str().unwrap(),
        users.to_str().unwrap(),
        pass.to_str().unwrap(),
    );
    let (child, line) = serve_with(&[
        "--dir",
        dir_arg,
        "--local",
        &addr,
        "--users",
        users_arg,
        "--auth-user",
        "writer",
        "--auth-password-file",
        pass_arg,
    ]);
    assert!(line.contains(&addr), "{line:?}");
    let get = |auth: Option<&str>| {
        let header = auth.map(|a| format!("Authorization: Basic {a}"));
        let headers: Vec<&str> = header.iter().map(String::as_str).collect();
        let stream = std::os::unix::net::UnixStream::connect(&sock).unwrap();
        http_send(stream, "GET", "/a.txt", &headers, b"")
    };
    let (status, head, _) = get(None);
    assert!(status.contains(" 401 "), "{status}");
    assert!(
        head.to_lowercase().contains("www-authenticate: basic"),
        "{head}"
    );
    // reader:pw-one and writer:pw-two.
    let (status, _, body) = get(Some("cmVhZGVyOnB3LW9uZQ=="));
    assert!(status.contains(" 200 "), "{status}");
    assert_eq!(body, b"alpha");
    let (status, _, _) = get(Some("d3JpdGVyOnB3LXR3bw=="));
    assert!(status.contains(" 200 "), "{status}");
    let (status, _, _) = get(Some("cmVhZGVyOnB3LXR3bw=="));
    assert!(status.contains(" 401 "), "{status}");
    stop(child);

    let (child, line) = serve_with(&["--dir", dir_arg, "--local", &addr, "--pin", "--json"]);
    let doc: serde_json::Value = serde_json::from_str(&line).unwrap();
    let pin = doc["pin"].as_str().unwrap().to_string();
    assert_eq!(pin.len(), 6, "{doc}");
    let (status, _, body) = get(None);
    assert!(status.contains(" 200 "), "{status}");
    let body = String::from_utf8_lossy(&body);
    assert!(
        body.contains("6-digit code") && !body.contains("alpha"),
        "{body}"
    );
    stop(child);

    // --anon-read alone has no meaning.
    let out = podnet(&["serve", "--dir", dir_arg, "--local", &addr, "--anon-read"]);
    assert_eq!(out.status.code(), Some(64), "{}", stderr(&out));
}

/// T-091: `podnet serve DIR` with two local listeners, --list, --upload,
/// --tar and --users: the listing (JSON), a range, a PUT of 1 MiB by a
/// write user, a 403 for a read user, a 401 with no user, and a tar of a
/// directory, on each listener.
#[test]
fn serve_dir_with_list_upload_and_users() {
    let base = tempfile::tempdir().unwrap();
    let dir = base.path().join("www");
    std::fs::create_dir_all(dir.join("in")).unwrap();
    std::fs::write(dir.join("a.txt"), "0123456789").unwrap();
    std::fs::write(dir.join(".env"), "SECRET=1").unwrap();
    let pass = base.path().join("pass");
    private_file(&pass, "pw\n");
    let users = base.path().join("users");
    private_file(&users, "w:write:@pass\nr:read:@pass\n");
    let socks = [base.path().join("one.sock"), base.path().join("two.sock")];
    let addrs: Vec<String> = socks
        .iter()
        .map(|s| format!("unix:{}", s.display()))
        .collect();
    let (child, line) = serve_with(&[
        dir.to_str().unwrap(),
        "--local",
        &addrs[0],
        "--local",
        &addrs[1],
        "--list",
        "--upload",
        "--max-upload",
        "2000000",
        "--tar",
        "--users",
        users.to_str().unwrap(),
        "--json",
    ]);
    let doc: serde_json::Value = serde_json::from_str(&line).unwrap_or_else(|_| panic!("{line:?}"));
    assert_eq!(doc["listens"].as_array().unwrap().len(), 2, "{doc}");
    // w:pw and r:pw.
    let (w, r) = (
        "Authorization: Basic dzpwdw==",
        "Authorization: Basic cjpwdw==",
    );
    let send = |sock: &std::path::Path, method: &str, path: &str, headers: &[&str], body: &[u8]| {
        let stream = std::os::unix::net::UnixStream::connect(sock).unwrap();
        http_send(stream, method, path, headers, body)
    };
    for sock in &socks {
        let (status, _, body) = send(sock, "GET", "/", &[r, "Accept: application/json"], b"");
        assert!(status.contains(" 200 "), "{status}");
        let listing: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let names: Vec<&str> = listing["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["name"].as_str().unwrap())
            .collect();
        assert!(
            names.contains(&"a.txt") && !names.contains(&".env"),
            "{listing}"
        );
        let (status, _, body) = send(sock, "GET", "/a.txt", &[r, "Range: bytes=2-4"], b"");
        assert!(status.contains(" 206 "), "{status}");
        assert_eq!(body, b"234");
        let (status, _, _) = send(sock, "GET", "/a.txt", &[], b"");
        assert!(status.contains(" 401 "), "{status}");
        let (status, _, _) = send(sock, "PUT", "/in/r.bin", &[r], b"x");
        assert!(status.contains(" 403 "), "{status}");
    }
    let mib = vec![0x5au8; 1 << 20];
    let (status, _, _) = send(&socks[0], "PUT", "/in/one.bin", &[w], &mib);
    assert!(status.contains(" 201 "), "{status}");
    assert_eq!(std::fs::read(dir.join("in/one.bin")).unwrap(), mib);
    // The second listener sees the file of the first one.
    let (status, _, body) = send(&socks[1], "GET", "/in/one.bin", &[r], b"");
    assert!(status.contains(" 200 "), "{status}");
    assert_eq!(body.len(), 1 << 20);
    let (status, head, body) = send(&socks[1], "GET", "/in/?tar", &[r], b"");
    assert!(status.contains(" 200 "), "{status}");
    assert!(head.contains("application/x-tar"), "{head}");
    assert!(body.starts_with(b"in/"), "{:?}", &body[..8]);
    // Over --max-upload: 413, and no file.
    let big = vec![0u8; 2_000_001];
    let (status, _, _) = send(&socks[0], "PUT", "/in/big.bin", &[w], &big);
    assert!(status.contains(" 413 "), "{status}");
    assert!(!dir.join("in/big.bin").exists());
    stop(child);
    for sock in &socks {
        assert!(!sock.exists(), "{} stays", sock.display());
    }

    // --via none needs --local; --upload needs a write user.
    let dir_arg = dir.to_str().unwrap();
    let out = podnet(&["serve", dir_arg, "--via", "none"]);
    assert_eq!(out.status.code(), Some(64), "{}", stderr(&out));
    let out = podnet(&["serve", dir_arg, "--local", &addrs[0], "--upload"]);
    assert_eq!(out.status.code(), Some(64), "{}", stderr(&out));
    let out = podnet(&["serve", "--local", &addrs[0]]);
    assert_eq!(out.status.code(), Some(64), "{}", stderr(&out));
}

/// T-091: `share --file DIR --local unix:PATH` gives the directory as one
/// tar stream on a local listener, with no tunnel. --stop-after ends the
/// command after the download.
#[test]
fn share_local_serves_a_directory() {
    let base = tempfile::tempdir().unwrap();
    let dir = base.path().join("photos");
    std::fs::create_dir_all(dir.join("2026")).unwrap();
    std::fs::write(dir.join("2026/a.jpg"), vec![1u8; 3000]).unwrap();
    let sock = base.path().join("share.sock");
    let addr = format!("unix:{}", sock.display());
    use std::io::BufRead;
    let mut child = Command::new(env!("CARGO_BIN_EXE_podnet"))
        .args(["share", "--file", dir.to_str().unwrap(), "--local", &addr])
        .args(["--stop-after", "--json"])
        .env_remove("PODNET_ERROR_PROGRAM")
        .env_remove("TUNNEL_TOKEN")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let mut line = String::new();
    std::io::BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let doc: serde_json::Value = serde_json::from_str(&line).unwrap_or_else(|_| panic!("{line:?}"));
    let path = doc["path"].as_str().unwrap().to_string();
    assert!(path.starts_with("/s/"), "{doc}");
    assert_eq!(doc["listen"], addr.as_str());
    let (status, _, body) = http_send(
        std::os::unix::net::UnixStream::connect(&sock).unwrap(),
        "GET",
        &format!("{path}file"),
        &[],
        b"",
    );
    assert!(status.contains(" 200 "), "{status}");
    assert!(body.starts_with(b"photos/"), "{:?}", &body[..8]);
    assert!(body.len() > 3000);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        assert!(std::time::Instant::now() < deadline, "share did not stop");
        std::thread::sleep(std::time::Duration::from_millis(100));
    };
    assert_eq!(status.code(), Some(0), "{status:?}");
    assert!(!sock.exists());
    // Proxy mode needs the tunnel.
    let out = podnet(&["share", "--port", "1", "--local", &addr]);
    assert_eq!(out.status.code(), Some(64), "{}", stderr(&out));
}
