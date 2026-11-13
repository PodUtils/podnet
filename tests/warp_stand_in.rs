//! T-012: the account commands of `podnet warp` against the API stand-in of
//! `pod-testkit` (`pod_testkit::stand::warp::Api`) on the loopback. The
//! binary reads the base of the API from `PODNET_WARP_API`. No test
//! registers a device at the WARP service.

mod test_host;

use std::path::Path;
use std::process::{Command, Output};

use pod_testkit::stand;

/// The stand-in API, served over plain HTTP on the loopback.
struct World {
    api: stand::warp::Api,
    served: stand::Stand,
    dir: tempfile::TempDir,
}

impl World {
    fn new() -> World {
        let api = stand::warp::Api::new("", "192.0.2.1", "2001:db8::1");
        let served = stand::http(api.handler()).unwrap();
        World {
            api,
            served,
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn identity(&self) -> std::path::PathBuf {
        self.dir.path().join("warp.json")
    }

    /// `podnet warp --identity FILE ARGS` with the API of the stand-in.
    fn warp(&self, args: &[&str]) -> Output {
        self.warp_with(args, None)
    }

    fn warp_with(&self, args: &[&str], stdin: Option<&str>) -> Output {
        use std::io::Write as _;
        let identity = self.identity();
        let mut command = Command::new(env!("CARGO_BIN_EXE_podnet"));
        command
            .args([
                "warp",
                "--no-proxy",
                "--identity",
                identity.to_str().unwrap(),
            ])
            .args(args)
            .env("PODNET_WARP_API", format!("http://{}", self.served.addr()))
            .env("PODNET_STATE_DIR", self.dir.path().join("state"))
            .env_remove("PODNET_ERROR_PROGRAM")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let mut child = command.spawn().unwrap();
        if let Some(text) = stdin {
            child
                .stdin
                .take()
                .unwrap()
                .write_all(text.as_bytes())
                .unwrap();
        } else {
            drop(child.stdin.take());
        }
        child.wait_with_output().unwrap()
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn stored(path: &Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// The text of the stored file, with each string value.
fn file_text(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap()
}

fn ok(out: &Output, what: &str) {
    assert_eq!(
        out.status.code(),
        Some(0),
        "{what}: {}{}",
        text(&out.stdout),
        text(&out.stderr)
    );
}

#[test]
fn warp_stand_in_register_saves_one_device_and_a_second_run_loads_it() {
    test_host::need_loopback!();
    let world = World::new();
    let out = world.warp(&["register"]);
    ok(&out, "register");
    assert_eq!(world.api.devices(), 1);
    assert_eq!(world.api.asked("POST"), 1);
    let path = world.identity();
    assert!(path.exists());
    assert!(file_text(&path).contains("stand-in-device-1"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "the identity file holds a key");
    }
    // The second run loads the device: no new POST.
    let out = world.warp(&["register"]);
    ok(&out, "register again");
    assert_eq!(world.api.asked("POST"), 1);
    assert_eq!(world.api.devices(), 1);
}

#[test]
fn warp_stand_in_status_reads_the_account_of_the_saved_device() {
    test_host::need_loopback!();
    let world = World::new();
    ok(&world.warp(&["register"]), "register");
    let out = world.warp(&["status", "--json"]);
    ok(&out, "status");
    let stdout = text(&out.stdout);
    let doc: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("status --json: {e}: {stdout}"));
    assert!(
        doc.to_string().contains("free"),
        "the account type of the stand-in: {doc}"
    );
    assert_eq!(world.api.asked("POST"), 1, "status registers no device");
}

#[test]
fn warp_stand_in_license_from_stdin_reaches_the_api() {
    test_host::need_loopback!();
    let world = World::new();
    ok(&world.warp(&["register"]), "register");
    let key = "abcd1234-efgh5678-ijkl9012";
    let out = world.warp_with(&["license", "-"], Some(key));
    ok(&out, "license");
    let puts = world.api.bodies("PUT");
    assert_eq!(puts.len(), 1, "{puts:?}");
    assert!(puts[0].contains(key), "{puts:?}");
    // A key that the API refuses (400) is an error.
    let out = world.warp_with(&["license", "-"], Some("short"));
    assert_ne!(out.status.code(), Some(0), "{}", text(&out.stderr));
}

#[test]
fn warp_stand_in_rotate_enrols_a_new_key_for_the_same_device() {
    test_host::need_loopback!();
    let world = World::new();
    ok(&world.warp(&["register"]), "register");
    let path = world.identity();
    let before = stored(&path);
    let patches = world.api.asked("PATCH");
    let out = world.warp(&["rotate"]);
    ok(&out, "rotate");
    assert_eq!(world.api.asked("POST"), 1, "rotate registers no device");
    assert_eq!(world.api.devices(), 1);
    assert_eq!(world.api.asked("PATCH"), patches + 1, "one enrolment");
    let after = stored(&path);
    assert_ne!(before, after, "the stored key changes");
    assert!(file_text(&path).contains("stand-in-device-1"));
}

#[test]
fn warp_stand_in_delete_removes_the_device_and_the_file() {
    test_host::need_loopback!();
    let world = World::new();
    ok(&world.warp(&["register"]), "register");
    let out = world.warp(&["delete"]);
    ok(&out, "delete");
    assert_eq!(world.api.asked("DELETE"), 1);
    assert_eq!(world.api.devices(), 0);
    assert!(!world.identity().exists(), "delete removes the file");
}

#[test]
fn warp_stand_in_register_names_the_limit_of_new_devices() {
    test_host::need_loopback!();
    let world = World::new();
    world.api.fail_post(Some(429));
    let out = world.warp(&["register"]);
    assert_ne!(out.status.code(), Some(0));
    let err = text(&out.stderr);
    assert!(err.contains("429"), "{err}");
    assert!(!world.identity().exists(), "no file for a refused device");
}

// Step 2 of T-012: `warp proxy` through the edge stand-in of
// `tests/warp_edge` (the same edge as the tests of the network `warp`).
#[allow(dead_code)]
mod warp_edge;

mod through_the_edge {
    use std::io::{Read as _, Write as _};
    use std::net::{SocketAddr, TcpStream};
    use std::process::{Child, Stdio};
    use std::time::{Duration, Instant};

    use super::warp_edge::{echo_server, Edge, Route};
    use super::*;

    /// The port inside the tunnel that the edge carries to the echo server.
    const ECHO_TCP: u16 = 9007;

    fn free_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    /// `warp proxy` on a free port of the loopback, with the edge as the
    /// front door. Gives the child and its port when it listens, or the
    /// output of a child that ended.
    fn proxy(world: &World, edge: SocketAddr) -> Result<(Child, u16), Output> {
        let port = free_port();
        let identity = world.identity();
        let mut child = Command::new(env!("CARGO_BIN_EXE_podnet"))
            .args([
                "warp",
                "--no-proxy",
                "--identity",
                identity.to_str().unwrap(),
            ])
            .args(["--masque-endpoint", &edge.to_string()])
            .args(["proxy", "--listen", &format!("tcp://127.0.0.1:{port}")])
            .args(["--run-for", "60"])
            .env("PODNET_WARP_API", format!("http://{}", world.served.addr()))
            .env("PODNET_STATE_DIR", world.dir.path().join("state"))
            .env_remove("PODNET_ERROR_PROGRAM")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            if let Ok(Some(_)) = child.try_wait() {
                return Err(child.wait_with_output().unwrap());
            }
            if TcpStream::connect(("127.0.0.1", port)).is_ok() {
                return Ok((child, port));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = child.kill();
        Err(child.wait_with_output().unwrap())
    }

    /// A SOCKS5 CONNECT to `203.0.113.9:port` through the proxy on
    /// `proxy`: the reply code, and the stream.
    fn connect(proxy: u16, port: u16) -> (u8, TcpStream) {
        let mut s = TcpStream::connect(("127.0.0.1", proxy)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
        let mut request = vec![5, 1, 0, 1, 203, 0, 113, 9];
        request.extend_from_slice(&port.to_be_bytes());
        let mut method = [0u8; 2];
        let mut reply = [0u8; 10];
        // A reset or an end of the stream is a refusal: 0xff.
        let asked = s.write_all(&[5, 1, 0]).is_ok()
            && s.read_exact(&mut method).is_ok()
            && method == [5, 0]
            && s.write_all(&request).is_ok()
            && s.read_exact(&mut reply).is_ok();
        (if asked { reply[1] } else { 0xff }, s)
    }

    async fn world_and_edge() -> (World, Edge) {
        let world = World::new();
        let echo = echo_server().await;
        let edge = Edge::start(
            world.api.clone(),
            vec![Route {
                port: ECHO_TCP,
                outside: echo,
            }],
        )
        .await;
        world.api.serve_key(&edge.pem());
        (world, edge)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn warp_stand_in_proxy_carries_a_socks5_connect_through_the_edge() {
        test_host::need_loopback!();
        let (world, edge) = world_and_edge().await;
        let addr = edge.addr;
        let api = world.api.clone();
        let echoed = tokio::task::spawn_blocking(move || {
            let (mut child, port) = match proxy(&world, addr) {
                Ok(up) => up,
                Err(out) => panic!("warp proxy: {}{}", text(&out.stdout), text(&out.stderr)),
            };
            let (code, mut s) = connect(port, ECHO_TCP);
            let mut got = [0u8; 18];
            let echoed = code == 0
                && s.write_all(b"through the edge!!").is_ok()
                && s.read_exact(&mut got).is_ok()
                && &got == b"through the edge!!";
            let _ = child.kill();
            let out = child.wait_with_output().unwrap();
            (code, echoed, text(&out.stderr))
        })
        .await
        .unwrap();
        assert_eq!(echoed.0, 0, "the SOCKS5 reply: {}", echoed.2);
        assert!(echoed.1, "the echo through the edge: {}", echoed.2);
        assert_eq!(api.devices(), 1, "one device");
    }

    /// The plant of T-012: the record pins a key that the edge does not
    /// show. `warp proxy` must not carry the connection.
    #[tokio::test(flavor = "multi_thread")]
    async fn warp_stand_in_proxy_refuses_an_edge_with_another_key() {
        test_host::need_loopback!();
        let (world, edge) = world_and_edge().await;
        let other = Edge::start(world.api.clone(), vec![]).await;
        world.api.serve_key(&other.pem());
        let addr = edge.addr;
        let result = tokio::task::spawn_blocking(move || match proxy(&world, addr) {
            Err(out) => (None, text(&out.stderr)),
            Ok((mut child, port)) => {
                let (code, _s) = connect(port, ECHO_TCP);
                let _ = child.kill();
                let out = child.wait_with_output().unwrap();
                (Some(code), text(&out.stderr))
            }
        })
        .await
        .unwrap();
        assert_ne!(result.0, Some(0), "a CONNECT went through: {}", result.1);
    }

    /// `podnet warp ARGS` with the edge as the front door, `input` on stdin.
    fn warp_through(world: &World, edge: SocketAddr, args: &[&str], input: &[u8]) -> Output {
        let identity = world.identity();
        let mut child = Command::new(env!("CARGO_BIN_EXE_podnet"))
            .args([
                "warp",
                "--no-proxy",
                "--identity",
                identity.to_str().unwrap(),
            ])
            .args(["--masque-endpoint", &edge.to_string()])
            .args(args)
            .env("PODNET_WARP_API", format!("http://{}", world.served.addr()))
            .env("PODNET_STATE_DIR", world.dir.path().join("state"))
            .env("PODNET_RUN_DIR", world.dir.path().join("run"))
            .env_remove("PODNET_ERROR_PROGRAM")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(input).unwrap();
        child.wait_with_output().unwrap()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn warp_stand_in_nc_pipes_tcp_and_udp_through_the_edge() {
        test_host::need_loopback!();
        let (world, edge) = world_and_edge().await;
        let addr = edge.addr;
        let port = ECHO_TCP.to_string();
        let echo_udp = super::warp_edge::ECHO_PORT.to_string();
        let (tcp, udp) = tokio::task::spawn_blocking(move || {
            let tcp = warp_through(&world, addr, &["nc", "203.0.113.9", &port], b"nc over tcp");
            let udp = warp_through(
                &world,
                addr,
                &["nc", "-u", "--idle", "2", "203.0.113.9", &echo_udp],
                b"nc over udp",
            );
            (tcp, udp)
        })
        .await
        .unwrap();
        assert_eq!(text(&tcp.stdout), "nc over tcp", "{}", text(&tcp.stderr));
        assert_eq!(text(&udp.stdout), "nc over udp", "{}", text(&udp.stderr));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn warp_stand_in_forward_carries_a_local_port_through_the_edge() {
        test_host::need_loopback!();
        let (world, edge) = world_and_edge().await;
        let addr = edge.addr;
        let got = tokio::task::spawn_blocking(move || {
            let local = free_port();
            let identity = world.identity();
            let mut child = Command::new(env!("CARGO_BIN_EXE_podnet"))
                .args([
                    "warp",
                    "--no-proxy",
                    "--identity",
                    identity.to_str().unwrap(),
                ])
                .args(["--masque-endpoint", &addr.to_string()])
                .args(["forward", "--run-for", "60"])
                .args(["-L", &format!("127.0.0.1:{local}=203.0.113.9:{ECHO_TCP}")])
                .env("PODNET_WARP_API", format!("http://{}", world.served.addr()))
                .env("PODNET_STATE_DIR", world.dir.path().join("state"))
                .env_remove("PODNET_ERROR_PROGRAM")
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(30);
            let mut echoed = false;
            while Instant::now() < deadline && !echoed {
                if let Ok(Some(_)) = child.try_wait() {
                    break;
                }
                if let Ok(mut s) = TcpStream::connect(("127.0.0.1", local)) {
                    s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
                    let mut got = [0u8; 16];
                    echoed = s.write_all(b"forward the port").is_ok()
                        && s.read_exact(&mut got).is_ok()
                        && &got == b"forward the port";
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            let _ = child.kill();
            let out = child.wait_with_output().unwrap();
            (echoed, text(&out.stderr))
        })
        .await
        .unwrap();
        assert!(got.0, "the echo through the forward: {}", got.1);
    }

    /// The port inside the tunnel that the edge carries to the HTTP server.
    const HTTP_TCP: u16 = 9080;

    /// The body of the HTTP server behind the edge.
    const BODY: &str = "warp shell reached the edge";

    /// An HTTP server on the loopback that answers each request with
    /// [`BODY`].
    async fn http_server() -> SocketAddr {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut s, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut buf = [0u8; 4096];
                    let mut head = Vec::new();
                    while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                        match s.read(&mut buf).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => head.extend_from_slice(&buf[..n]),
                        }
                    }
                    let answer = format!(
                        "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{BODY}",
                        BODY.len()
                    );
                    let _ = s.write_all(answer.as_bytes()).await;
                    let _ = s.shutdown().await;
                });
            }
        });
        addr
    }

    /// `podnet warp ARGS` with the edge as the front door and the run
    /// directory of the world, with `envs` added.
    fn warp_cmd(world: &World, edge: SocketAddr, args: &[&str]) -> Command {
        let identity = world.identity();
        let mut command = Command::new(env!("CARGO_BIN_EXE_podnet"));
        command
            .args([
                "warp",
                "--no-proxy",
                "--identity",
                identity.to_str().unwrap(),
            ])
            .args(["--masque-endpoint", &edge.to_string()])
            .args(args)
            .env("PODNET_WARP_API", format!("http://{}", world.served.addr()))
            .env("PODNET_STATE_DIR", world.dir.path().join("state"))
            .env("PODNET_RUN_DIR", world.dir.path().join("run"))
            .env_remove("PODNET_ERROR_PROGRAM")
            .env_remove("ALL_PROXY")
            .env_remove("all_proxy")
            .env_remove("HTTPS_PROXY")
            .env_remove("https_proxy")
            .env_remove("HTTP_PROXY")
            .env_remove("http_proxy")
            .env_remove("NO_PROXY")
            .env_remove("no_proxy")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    }

    /// Stops the `warp up` of the world at the end of the test.
    struct Down<'a>(&'a World, SocketAddr);

    impl Drop for Down<'_> {
        fn drop(&mut self) {
            let _ = warp_cmd(self.0, self.1, &["down"]).output();
        }
    }

    /// `warp up` over the edge stand-in, then `warp shell` with `$SHELL` a
    /// script: the script runs `curl` to a route of the edge. Under the pod
    /// the connection goes through the socket of `warp up`; when the pod
    /// cannot start, the shell gets the proxy variables of the env file and
    /// curl reads them. Both ways reach the HTTP server behind the edge.
    /// The test prints the way that ran (the backend line of the pod, or
    /// the line of the shell with no pod).
    #[tokio::test(flavor = "multi_thread")]
    async fn warp_stand_in_shell_runs_a_command_that_reaches_the_edge() {
        test_host::need_loopback!();
        shell_reaches_the_edge("auto").await;
    }

    /// The same with the seccomp backend of the pod.
    #[tokio::test(flavor = "multi_thread")]
    async fn warp_stand_in_shell_with_the_seccomp_backend_reaches_the_edge() {
        test_host::need_loopback!();
        shell_reaches_the_edge("seccomp").await;
    }

    async fn shell_reaches_the_edge(backend: &'static str) {
        let Ok(curl) = which("curl") else {
            eprintln!("skipping, this host has no curl");
            return;
        };
        let world = World::new();
        let http = http_server().await;
        let edge = Edge::start(
            world.api.clone(),
            vec![Route {
                port: HTTP_TCP,
                outside: http,
            }],
        )
        .await;
        world.api.serve_key(&edge.pem());
        let addr = edge.addr;
        let (up, shell, files) = tokio::task::spawn_blocking(move || {
            let port = free_port().to_string();
            let up = warp_cmd(&world, addr, &["up", "--port", &port]).output().unwrap();
            let _down = Down(&world, addr);
            let out = world.dir.path().join("out");
            std::fs::create_dir(&out).unwrap();
            let script = world.dir.path().join("shell.sh");
            std::fs::write(
                &script,
                format!(
                    "#!/bin/sh\n\
                     out={out}\n\
                     echo \"$PODNET_WARP_SHELL\" > \"$out/var\"\n\
                     env | grep -i '^all_proxy=' > \"$out/proxy\"\n\
                     {curl} -sS -m 20 http://203.0.113.9:{HTTP_TCP}/ > \"$out/body\" 2> \"$out/err\"\n\
                     echo $? > \"$out/code\"\n",
                    out = out.display(),
                    curl = curl.display(),
                ),
            )
            .unwrap();
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
            let shell = warp_cmd(&world, addr, &["shell", "--backend", backend])
                .env("SHELL", &script)
                .output()
                .unwrap();
            let read = |name: &str| std::fs::read_to_string(out.join(name)).unwrap_or_default();
            let files = ["var", "proxy", "body", "err", "code"].map(read);
            (up, shell, files)
        })
        .await
        .unwrap();
        let [var, proxy, body, err, code] = files;
        assert!(
            up.status.success(),
            "warp up: {}{}",
            text(&up.stdout),
            text(&up.stderr)
        );
        let said = text(&shell.stderr);
        if code.is_empty() && !said.contains(" backend: ") && !said.contains("no pod") {
            eprintln!("skipping, the pod of warp shell cannot run on this host: {said}");
            return;
        }
        eprintln!("warp shell said: {said}");
        eprintln!("the shell saw: {proxy}");
        assert_eq!(var.trim(), "1", "the shell has PODNET_WARP_SHELL: {said}");
        assert_eq!(code.trim(), "0", "curl: {err}; warp shell: {said}");
        assert_eq!(body, BODY, "warp shell: {said}");
    }

    fn which(name: &str) -> Result<std::path::PathBuf, ()> {
        let path = std::env::var_os("PATH").ok_or(())?;
        std::env::split_paths(&path)
            .map(|d| d.join(name))
            .find(|p| p.is_file())
            .ok_or(())
    }
}
