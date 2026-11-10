//! Listeners: inbound connections, the inbound directory and the front doors.

use super::*;

#[test]
fn seccomp_backend_accepts_inbound() {
    test_host::need_loopback!();
    if !have_compiler() || !seccomp_available() {
        eprintln!("skipping: no compiler or no SECCOMP_RET_USER_NOTIF");
        return;
    }
    let dir = std::env::temp_dir().join(format!("podnet-netns-inbound-{}", std::process::id()));
    let inbound = dir.join("in");
    std::fs::create_dir_all(&inbound).unwrap();
    let proxy = dir.join("socks.sock");
    let listener = UnixListener::bind(&proxy).unwrap();
    let (tx, _rx) = mpsc::channel();
    std::thread::spawn(move || socks5_proxy(listener, tx, false));

    let server = build_program(&dir, "server", SERVER, false);
    let mut child = pod_command()
        .args(["--backend", "seccomp", "--inbound-dir"])
        .arg(&inbound)
        .args(["-x"])
        .arg(format!("unix:{}", proxy.display()))
        .arg("--")
        .arg(&server)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    // The listener is a unix socket named after the bound port.
    let sockpath = inbound.join("8080.sock");
    let mut conn = None;
    for _ in 0..100 {
        if let Ok(c) = UnixStream::connect(&sockpath) {
            conn = Some(c);
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let mut c = conn.expect("the inbound listener did not appear");
    c.write_all(b"PING\n").unwrap();
    let mut buf = [0u8; 16];
    let n = c.read(&mut buf).unwrap();
    assert_eq!(
        &buf[..n],
        b"PONG\n",
        "got {:?}",
        String::from_utf8_lossy(&buf[..n])
    );
    drop(c);
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Issue 306: with no --inbound-dir, the listener is in
/// `$XDG_RUNTIME_DIR/podnet-netns-inbound`, a directory of mode 0700, not in
/// a shared name in /tmp.
#[test]
fn the_default_inbound_directory_is_private() {
    test_host::need_loopback!();
    use std::os::unix::fs::PermissionsExt;
    if !have_compiler() || !seccomp_available() {
        eprintln!("skipping: no compiler or no SECCOMP_RET_USER_NOTIF");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let runtime = dir.path().join("run");
    std::fs::create_dir(&runtime).unwrap();
    std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o700)).unwrap();
    let proxy = dir.path().join("socks.sock");
    let listener = UnixListener::bind(&proxy).unwrap();
    let (tx, _rx) = mpsc::channel();
    std::thread::spawn(move || socks5_proxy(listener, tx, false));

    let server = build_program(dir.path(), "server", SERVER, false);
    let mut child = pod_command()
        .args(["--backend", "seccomp", "-x"])
        .arg(format!("unix:{}", proxy.display()))
        .arg("--")
        .arg(&server)
        .env("XDG_RUNTIME_DIR", &runtime)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let inbound = runtime.join("podnet-netns-inbound");
    let sockpath = inbound.join("8080.sock");
    let mut conn = None;
    for _ in 0..100 {
        if let Ok(c) = UnixStream::connect(&sockpath) {
            conn = Some(c);
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let mut c = conn.expect("the inbound listener is not in $XDG_RUNTIME_DIR");
    let mode = std::fs::metadata(&inbound).unwrap().permissions().mode() & 0o777;
    c.write_all(b"PING\n").unwrap();
    let mut buf = [0u8; 16];
    let _ = c.read(&mut buf);
    drop(c);
    let _ = child.wait();
    assert_eq!(mode, 0o700, "the mode of {}", inbound.display());
}

#[test]
fn podnet_netns_serves_a_front_door_and_chains_with_itself() {
    if !have_compiler() || !seccomp_available() {
        eprintln!("skipping: no compiler or no SECCOMP_RET_USER_NOTIF");
        return;
    }
    let dir = std::env::temp_dir().join(format!("podnet-netns-serve-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let echo = dir.join("echo.sock");
    let front = dir.join("front.sock");
    let listener = UnixListener::bind(&echo).unwrap();
    let (tx, _rx) = mpsc::channel();
    std::thread::spawn(move || socks5_proxy(listener, tx, false));

    // One instance serves SOCKS5 and forwards through the echo proxy.
    let mut server = pod_command()
        .args(["--serve"])
        .arg(format!("unix:{}", front.display()))
        .args(["-x"])
        .arg(format!("unix:{}", echo.display()))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    for _ in 0..100 {
        if front.exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    // A second instance runs a client through the first one's front door.
    let program = build_program(&dir, "client-serve", CLIENT, false);
    let stdout = run(&dir, &[&front], &program);
    assert!(stdout.contains("HELLO-FROM-TARGET"), "{stdout:?}");

    let _ = server.kill();
    let _ = server.wait();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Issue 69: on the seccomp backend, an epoll listener wakes for an inbound
/// connection and is not readable again after the accept; and a thread
/// that blocks in accept does not stop the other system calls of the child.
pub(super) const EPOLL_SERVER: &str = r#"
#define _GNU_SOURCE
#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include <pthread.h>
#include <arpa/inet.h>
#include <sys/epoll.h>
#include <sys/socket.h>
static int listen_on(int port, int flags){
  int s=socket(AF_INET,SOCK_STREAM|flags,0);
  struct sockaddr_in a; memset(&a,0,sizeof a); a.sin_family=AF_INET;
  a.sin_port=htons(port); a.sin_addr.s_addr=htonl(INADDR_LOOPBACK);
  if(bind(s,(struct sockaddr*)&a,sizeof a)<0){perror("bind");return -1;}
  if(listen(s,4)<0){perror("listen");return -1;}
  return s;
}
static void *blocked(void *arg){
  int c=accept(*(int*)arg,0,0);
  if(c>=0){ if(write(c,"T\n",2)<0){} close(c); }
  return 0;
}
int main(void){
  int s=listen_on(8090,SOCK_NONBLOCK); if(s<0) return 1;
  int ep=epoll_create1(0); struct epoll_event ev; memset(&ev,0,sizeof ev);
  ev.events=EPOLLIN; ev.data.fd=s; epoll_ctl(ep,EPOLL_CTL_ADD,s,&ev);
  int n=epoll_wait(ep,&ev,1,20000);
  if(n!=1){ printf("no event\n"); return 2; }
  int c=accept(s,0,0); if(c<0){ perror("accept"); return 3; }
  if(write(c,"E\n",2)<0){} close(c);
  printf("again %d\n", epoll_wait(ep,&ev,1,300));
  int s2=listen_on(8091,0); if(s2<0) return 4;
  pthread_t t; pthread_create(&t,0,blocked,&s2);
  usleep(300000);
  int u=socket(AF_INET,SOCK_DGRAM,0);
  printf("socket while blocked %d\n", u>=0); fflush(stdout);
  if(u>=0) close(u);
  pthread_join(t,0);
  printf("done\n");
  return 0;
}
"#;

#[test]
fn seccomp_inbound_wakes_epoll_and_accept_does_not_block_the_child() {
    test_host::need_loopback!();
    if !have_compiler() || !seccomp_available() {
        eprintln!("skipping: no compiler or no SECCOMP_RET_USER_NOTIF");
        return;
    }
    let dir = std::env::temp_dir().join(format!("podnet-netns-epoll-{}", std::process::id()));
    let inbound = dir.join("in");
    std::fs::create_dir_all(&inbound).unwrap();
    let proxy = dir.join("socks.sock");
    let listener = UnixListener::bind(&proxy).unwrap();
    let (tx, _rx) = mpsc::channel();
    std::thread::spawn(move || socks5_proxy(listener, tx, false));
    let server = build_program(&dir, "epoll-server", EPOLL_SERVER, false);
    let mut child = pod_command()
        .args(["--backend", "seccomp", "--inbound-dir"])
        .arg(&inbound)
        .args(["-x"])
        .arg(format!("unix:{}", proxy.display()))
        .arg("--")
        .arg(&server)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let fetch = |port: u16| {
        let path = inbound.join(format!("{port}.sock"));
        for _ in 0..200 {
            if let Ok(mut c) = UnixStream::connect(&path) {
                c.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
                let mut buf = [0u8; 8];
                let n = c.read(&mut buf).unwrap_or(0);
                return String::from_utf8_lossy(&buf[..n]).into_owned();
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("no inbound listener for port {port}");
    };
    assert_eq!(fetch(8090), "E\n", "epoll did not wake for the connection");
    assert_eq!(fetch(8091), "T\n");
    let start = std::time::Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if start.elapsed() > Duration::from_secs(20) {
            let _ = child.kill();
            panic!("the child did not end in 20 s");
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let mut out = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut out)
        .unwrap();
    assert!(status.success(), "{out}");
    assert!(
        out.contains("again 0"),
        "the listener stayed readable: {out}"
    );
    assert!(out.contains("socket while blocked 1"), "{out}");
    assert!(out.contains("done"), "{out}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn seccomp_backend_dials_the_tailnet_with_no_shim() {
    if !have_compiler() || !seccomp_available() {
        eprintln!("skipping: no compiler or no SECCOMP_RET_USER_NOTIF");
        return;
    }
    let dir = std::env::temp_dir().join(format!("podnet-netns-ts-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let api = dir.join("tailscaled.sock");
    let listener = UnixListener::bind(&api).unwrap();
    // A stand-in for tailscaled's LocalAPI: accept the ts-dial upgrade, greet,
    // then echo.
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let mut c = match conn {
                Ok(c) => c,
                Err(_) => break,
            };
            std::thread::spawn(move || {
                let mut head = Vec::new();
                let mut b = [0u8; 1];
                while c.read(&mut b).unwrap_or(0) == 1 {
                    head.push(b[0]);
                    if head.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }
                if !String::from_utf8_lossy(&head).contains("Upgrade: ts-dial") {
                    return;
                }
                let _ = c.write_all(
                    b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: ts-dial\r\nConnection: upgrade\r\n\r\n",
                );
                let _ = c.write_all(b"HELLO-FROM-TAILNET\n");
                let mut buf = [0u8; 4096];
                while let Ok(n) = c.read(&mut buf) {
                    if n == 0 || c.write_all(&buf[..n]).is_err() {
                        break;
                    }
                }
            });
        }
    });

    let program = build_program(&dir, "client-ts", CLIENT, false);
    let output = pod_command()
        .args(["--backend", "seccomp", "-x"])
        .arg(format!("tailscale:{}", api.display()))
        .arg("--")
        .arg(&program)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("HELLO-FROM-TAILNET"),
        "{:?}",
        String::from_utf8_lossy(&output.stdout)
    );
    let _ = std::fs::remove_dir_all(&dir);
}
