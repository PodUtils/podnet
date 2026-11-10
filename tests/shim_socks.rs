//! End-to-end test for the Tailscale socksifying shim.
//!
//! A fake `tailscaled` LocalAPI accepts the `ts-dial` upgrade and echoes
//! whatever is written after it. The real `podnet net tailscale` front door is
//! started on a unix socket in front of it. A compiled C program calls
//! `connect(AF_INET, 100.64.0.1:1234)` under `LD_PRELOAD` and must reach the
//! echo through the front door, with `getpeername` reporting the logical
//! address rather than the `AF_UNIX` one.
//!
//! This is the whole path the sandbox requires: `AF_UNIX` in, `AF_UNIX` to the
//! daemon, no `AF_INET` connect of the program's own. If no C compiler is
//! available the test reports that and passes, as `shim_direct.rs` does.

use std::io::Write;
use std::process::Command;

use podnet::vnet::proxy::ProxyListen;
use podnet::vnet::shim;
use podnet::vnet::tailscale::{self, Dialer};

fn have_compiler() -> bool {
    shim::compiler().is_ok()
}

fn build_program(dir: &std::path::Path, name: &str, source: &str) -> std::path::PathBuf {
    let source_path = dir.join(format!("{name}.c"));
    std::fs::write(&source_path, source).unwrap();
    let output = dir.join(name);
    let cc = shim::compiler().unwrap();
    let status = Command::new(cc)
        .arg("-O2")
        .arg("-o")
        .arg(&output)
        .arg(&source_path)
        .status()
        .unwrap();
    assert!(status.success(), "compiling {name} failed");
    output
}

/// Accept one `ts-dial` request and echo the bytes that follow it. A read
/// of the status gets `Running`.
async fn fake_tailscaled(listener: tokio::net::UnixListener) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    // T-081: the door reads the status first; answer Running.
    let (mut stream, head) = loop {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while stream.read(&mut byte).await.unwrap_or(0) == 1 {
            head.push(byte[0]);
            if head.ends_with(b"\r\n\r\n") {
                break;
            }
        }
        if head.starts_with(b"GET /localapi/v0/status") {
            let _ = stream
                .write_all(b"HTTP/1.0 200 OK\r\n\r\n{\"BackendState\":\"Running\"}")
                .await;
            continue;
        }
        break (stream, head);
    };
    assert!(
        String::from_utf8_lossy(&head).contains("Upgrade: ts-dial"),
        "not a ts-dial request"
    );
    stream
        .write_all(
            b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: ts-dial\r\nConnection: upgrade\r\n\r\n",
        )
        .await
        .unwrap();
    let mut buf = [0u8; 1024];
    loop {
        match stream.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if stream.write_all(&buf[..n]).await.is_err() {
                    break;
                }
            }
        }
    }
}

#[test]
fn af_inet_connect_is_redirected_to_the_tailnet_front_door() {
    if !have_compiler() {
        eprintln!("skipping: no C compiler");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let api = dir.path().join("tailscaled.sock");
    let front = dir.path().join("front.sock");

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    let client = build_program(
        dir.path(),
        "client",
        r#"
#include <stdio.h>
#include <string.h>
#include <arpa/inet.h>
#include <unistd.h>
int main(void){
  int s=socket(AF_INET,SOCK_STREAM,0);
  struct sockaddr_in a; memset(&a,0,sizeof a);
  a.sin_family=AF_INET; a.sin_port=htons(1234);
  inet_pton(AF_INET,"100.64.0.1",&a.sin_addr);
  if(connect(s,(struct sockaddr*)&a,sizeof a)<0){perror("connect");return 1;}
  if(write(s,"ping",4)!=4){perror("write");return 2;}
  char buf[16]={0}; int n=read(s,buf,15);
  printf("client: got %d bytes: %s\n", n, buf);
  struct sockaddr_in peer; socklen_t pl=sizeof peer;
  if(getpeername(s,(struct sockaddr*)&peer,&pl)==0)
    printf("client: peer %s:%d\n", inet_ntoa(peer.sin_addr), ntohs(peer.sin_port));
  close(s); return 0;
}
"#,
    );

    runtime.block_on(async {
        let api_listener = tokio::net::UnixListener::bind(&api).unwrap();
        tokio::spawn(fake_tailscaled(api_listener));
        let proxy = tailscale::serve(Dialer::new(&api), ProxyListen::Unix(front.clone()), 80)
            .await
            .unwrap();

        let shim = shim::build_socks(dir.path()).unwrap();
        let output = Command::new(&client)
            .env("LD_PRELOAD", &shim.path)
            .env("PODNET_SOCKS_PROXY", &front)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "client failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("got 4 bytes: ping"), "{stdout}");
        assert!(stdout.contains("peer 100.64.0.1:1234"), "{stdout}");
        proxy.abort();
    });

    let _ = std::io::stdout().flush();
}

/// A fake SOCKS5 door: each request is recorded as `addr:port`, answered
/// with success, then the door writes `door` and closes.
async fn fake_door(
    listener: tokio::net::UnixListener,
    seen: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    while let Ok((mut stream, _)) = listener.accept().await {
        let seen = seen.clone();
        tokio::spawn(async move {
            let mut head = [0u8; 3];
            stream.read_exact(&mut head).await.ok()?;
            stream.write_all(&[5, 0]).await.ok()?;
            let mut request = [0u8; 4];
            stream.read_exact(&mut request).await.ok()?;
            let ip: std::net::IpAddr = if request[3] == 1 {
                let mut a = [0u8; 4];
                stream.read_exact(&mut a).await.ok()?;
                a.into()
            } else {
                let mut a = [0u8; 16];
                stream.read_exact(&mut a).await.ok()?;
                a.into()
            };
            let mut port = [0u8; 2];
            stream.read_exact(&mut port).await.ok()?;
            let target = std::net::SocketAddr::new(ip, u16::from_be_bytes(port));
            seen.lock().unwrap().push(target.to_string());
            stream
                .write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0])
                .await
                .ok()?;
            stream.write_all(b"door").await.ok()?;
            stream.shutdown().await.ok()
        });
    }
}

/// A client that connects to each `FAMILY ADDR PORT` triple of its
/// arguments, with a connect limit of 1 s, and prints `ADDR PORT door`,
/// `ADDR PORT fail` or `ADDR PORT other`.
const ROUTE_CLIENT: &str = r#"
#include <stdio.h>
#include <string.h>
#include <stdlib.h>
#include <arpa/inet.h>
#include <sys/time.h>
#include <unistd.h>
int main(int argc, char **argv){
  for (int i = 1; i + 2 < argc; i += 3) {
    int v6 = strcmp(argv[i], "6") == 0;
    int s = socket(v6 ? AF_INET6 : AF_INET, SOCK_STREAM, 0);
    struct timeval tv = {1, 0};
    setsockopt(s, SOL_SOCKET, SO_SNDTIMEO, &tv, sizeof tv);
    setsockopt(s, SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof tv);
    struct sockaddr_storage st; memset(&st, 0, sizeof st);
    socklen_t len;
    if (v6) {
      struct sockaddr_in6 *a = (struct sockaddr_in6 *)&st;
      a->sin6_family = AF_INET6; a->sin6_port = htons(atoi(argv[i + 2]));
      inet_pton(AF_INET6, argv[i + 1], &a->sin6_addr); len = sizeof *a;
    } else {
      struct sockaddr_in *a = (struct sockaddr_in *)&st;
      a->sin_family = AF_INET; a->sin_port = htons(atoi(argv[i + 2]));
      inet_pton(AF_INET, argv[i + 1], &a->sin_addr); len = sizeof *a;
    }
    const char *what = "fail";
    if (connect(s, (struct sockaddr *)&st, len) == 0) {
      char buf[8] = {0};
      int n = read(s, buf, 4);
      what = (n == 4 && memcmp(buf, "door", 4) == 0) ? "door" : "other";
    }
    printf("%s %s %s\n", argv[i + 1], argv[i + 2], what);
    close(s);
  }
  return 0;
}
"#;

/// Run `client` with `env` against a fake door at `door`; the output lines
/// and the requests of the door.
fn run_routes(
    client: &std::path::Path,
    door: &std::path::Path,
    env: &[(&str, &str)],
    targets: &[&str],
) -> (Vec<String>, Vec<String>) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let output = runtime.block_on(async {
        let listener = tokio::net::UnixListener::bind(door).unwrap();
        tokio::spawn(fake_door(listener, seen.clone()));
        let mut cmd = tokio::process::Command::new(client);
        cmd.args(targets.iter().flat_map(|t| t.split(' ')));
        for (key, value) in env {
            cmd.env(key, value);
        }
        cmd.output().await.unwrap()
    });
    assert!(
        output.status.success(),
        "client failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let lines = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_string)
        .collect();
    let seen = seen.lock().unwrap().clone();
    (lines, seen)
}

/// T-167: with PODNET_SOCKS_CIDRS, the socks shim routes only the listed
/// destinations (an address with a port, a prefix, an IPv6 address with a
/// port, an IPv4-mapped address); the tailnet default is gone.
#[test]
fn cidr_list_routes_only_listed() {
    if !have_compiler() {
        eprintln!("skipping: no C compiler");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let door = dir.path().join("door.sock");
    let client = build_program(dir.path(), "routes", ROUTE_CLIENT);
    let shim = shim::build_socks(dir.path()).unwrap();
    let preload = shim.path.to_string_lossy().into_owned();
    let door_path = door.to_string_lossy().into_owned();
    let (lines, seen) = run_routes(
        &client,
        &door,
        &[
            ("LD_PRELOAD", preload.as_str()),
            ("PODNET_SOCKS_PROXY", door_path.as_str()),
            (
                "PODNET_SOCKS_CIDRS",
                "127.0.0.1:3000, 10.0.0.0/8,[::1]:8080,bad/99",
            ),
        ],
        &[
            "4 127.0.0.1 3000",
            "4 127.0.0.1 3001",
            "4 10.1.2.3 80",
            "4 11.1.2.3 80",
            "4 100.64.0.1 80",
            "6 ::1 8080",
            "6 ::1 8081",
            "6 ::ffff:10.0.0.5 22",
        ],
    );
    let routed: Vec<&str> = lines
        .iter()
        .filter(|l| l.ends_with(" door"))
        .map(String::as_str)
        .collect();
    // The socks shim keeps the family of the socket: a host with no IPv6
    // makes no AF_INET6 socket, and the IPv6 lines fail before the shim.
    // SAFETY: socket and close with valid arguments.
    let v6 = unsafe {
        let fd = libc::socket(libc::AF_INET6, libc::SOCK_STREAM, 0);
        if fd >= 0 {
            libc::close(fd);
        }
        fd >= 0
    };
    let mut want = vec!["127.0.0.1 3000 door", "10.1.2.3 80 door"];
    let mut want_seen = vec!["127.0.0.1:3000", "10.1.2.3:80"];
    if v6 {
        want.extend(["::1 8080 door", "::ffff:10.0.0.5 22 door"]);
        want_seen.extend(["[::1]:8080", "[::ffff:10.0.0.5]:22"]);
    } else {
        eprintln!("no AF_INET6 socket on this host: the IPv6 routes are not tested");
    }
    assert_eq!(routed, want, "{lines:?}");
    assert_eq!(seen, want_seen);
}

/// T-167: `net shim --socks-route` (PODNET_VNET_SOCKS and _ROUTES): a
/// stream connect to a listed destination of the shim of the virtual
/// network goes to the door; another goes to its abstract name.
#[test]
fn vnet_shim_sends_a_socks_route_to_the_door() {
    if !have_compiler() {
        eprintln!("skipping: no C compiler");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let door = dir.path().join("door.sock");
    let client = build_program(dir.path(), "routes", ROUTE_CLIENT);
    let shim = shim::build(dir.path()).unwrap();
    let options = shim::ShimOptions {
        prefix: format!("podnet-vnet-route-{}", std::process::id()),
        socks_routes: vec!["192.168.1.5:5432,[fd00::5]:80,10.9.0.0/16".into()],
        socks_socket: door.to_string_lossy().into_owned(),
        ..shim::ShimOptions::default()
    };
    let env = options.environment(&shim.path);
    let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let (lines, seen) = run_routes(
        &client,
        &door,
        &env,
        &[
            "4 192.168.1.5 5432",
            "4 192.168.1.6 5432",
            "4 192.168.1.5 5433",
            "6 fd00::5 80",
            "6 fd00::5 81",
            "6 ::ffff:10.9.1.1 22",
            "4 10.10.0.1 22",
        ],
    );
    // The virtual sockets are AF_UNIX: IPv6 works on a host with no IPv6.
    assert_eq!(
        lines,
        [
            "192.168.1.5 5432 door",
            "192.168.1.6 5432 fail",
            "192.168.1.5 5433 fail",
            "fd00::5 80 door",
            "fd00::5 81 fail",
            "::ffff:10.9.1.1 22 door",
            "10.10.0.1 22 fail",
        ]
    );
    assert_eq!(
        seen,
        ["192.168.1.5:5432", "[fd00::5]:80", "[::ffff:10.9.1.1]:22"]
    );
}
