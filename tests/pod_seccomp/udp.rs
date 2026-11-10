//! UDP: datagrams in the stream, names, front doors and the source address.

use super::*;

pub(super) const UDP_CLIENT: &str = r#"
#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include <errno.h>
#include <arpa/inet.h>
#include <sys/socket.h>
int main(void) {
    alarm(10);
    int s = socket(AF_INET, SOCK_DGRAM, 0);
    struct sockaddr_in d; memset(&d, 0, sizeof d);
    d.sin_family = AF_INET; d.sin_port = htons(9999);
    inet_pton(AF_INET, "127.0.0.1", &d.sin_addr);
    if (sendto(s, "PING", 4, 0, (struct sockaddr *)&d, sizeof d) < 0) { perror("sendto"); return 1; }
    char b[128] = {0};
    int n = recvfrom(s, b, sizeof b - 1, 0, NULL, NULL);
    printf("UDP:%d:%s", n, n > 0 ? b : "");
    return 0;
}
"#;

/// Issue 84: a proxy on a unix socket that knows only the UDP association
/// in the stream (0x83) and refuses UDP ASSOCIATE. The datagram of the
/// program goes as a frame on the control connection, and the answer comes
/// back the same way: the supervisor needs no kernel UDP socket.
#[test]
fn seccomp_backend_relays_udp_in_the_stream() {
    if !have_compiler() || !seccomp_available() {
        eprintln!("skipping: no compiler or no SECCOMP_RET_USER_NOTIF");
        return;
    }
    let dir = std::env::temp_dir().join(format!("podnet-netns-udp-stream-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let sock = dir.join("socks.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    let (commands, seen) = mpsc::channel::<u8>();
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let mut c = match conn {
                Ok(c) => c,
                Err(_) => break,
            };
            let commands = commands.clone();
            std::thread::spawn(move || {
                let Some((cmd, _host, _port)) = read_request(&mut c) else {
                    return;
                };
                let _ = commands.send(cmd);
                if cmd != 0x83 {
                    let _ = c.write_all(&[5, 7, 0, 1, 0, 0, 0, 0, 0, 0]);
                    return;
                }
                let _ = c.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
                loop {
                    let mut len = [0u8; 2];
                    if c.read_exact(&mut len).is_err() {
                        return;
                    }
                    let mut datagram = vec![0u8; u16::from_be_bytes(len) as usize];
                    if c.read_exact(&mut datagram).is_err() {
                        return;
                    }
                    // An IPv4 header: 10 bytes; the answer comes from the
                    // same address.
                    if datagram.len() < 10 || datagram[3] != 1 {
                        continue;
                    }
                    let mut answer = datagram[..10].to_vec();
                    answer.extend_from_slice(b"ECHO:");
                    answer.extend_from_slice(&datagram[10..]);
                    let mut frame = (answer.len() as u16).to_be_bytes().to_vec();
                    frame.extend_from_slice(&answer);
                    if c.write_all(&frame).is_err() {
                        return;
                    }
                }
            });
        }
    });
    let program = build_program(&dir, "udp-stream-client", UDP_CLIENT, false);
    let stdout = run(&dir, &[&sock], &program);
    assert!(stdout.contains("UDP:9:ECHO:PING"), "{stdout:?}");
    let commands: Vec<u8> = seen.try_iter().collect();
    assert_eq!(commands, vec![0x83], "no UDP ASSOCIATE: {commands:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

pub(super) const UDP_NAME_CLIENT: &str = r#"
#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include <netdb.h>
#include <sys/socket.h>
int main(void) {
    alarm(10);
    struct addrinfo hints, *res = NULL;
    memset(&hints, 0, sizeof hints);
    hints.ai_family = AF_INET; hints.ai_socktype = SOCK_DGRAM;
    int r = getaddrinfo("stun.example", "19302", &hints, &res);
    if (r != 0) { printf("GAI:%d\n", r); return 1; }
    int s = socket(AF_INET, SOCK_DGRAM, 0);
    if (sendto(s, "PING", 4, 0, res->ai_addr, res->ai_addrlen) < 0) { perror("sendto"); return 1; }
    char b[128] = {0};
    int n = recvfrom(s, b, sizeof b - 1, 0, NULL, NULL);
    printf("UDP:%d:%s", n, n > 0 ? b : "");
    return 0;
}
"#;

/// Issue 113: a datagram to a fake IP of the pod goes to the proxy with
/// the name (ATYP 3), as a TCP connect does. Before, the proxy got the fake
/// IP (198.18.x.x), which no host has.
#[test]
fn seccomp_backend_sends_udp_to_a_name_as_the_name() {
    if !have_compiler() || !seccomp_available() {
        eprintln!("skipping: no compiler or no SECCOMP_RET_USER_NOTIF");
        return;
    }
    let dir = std::env::temp_dir().join(format!("podnet-netns-udp-name-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let sock = dir.join("socks.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    let (dests, seen) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut c) = conn else { break };
            let dests = dests.clone();
            std::thread::spawn(move || {
                let Some((cmd, _, _)) = read_request(&mut c) else {
                    return;
                };
                if cmd != 0x83 {
                    let _ = c.write_all(&[5, 7, 0, 1, 0, 0, 0, 0, 0, 0]);
                    return;
                }
                let _ = c.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
                loop {
                    let mut len = [0u8; 2];
                    if c.read_exact(&mut len).is_err() {
                        return;
                    }
                    let mut d = vec![0u8; u16::from_be_bytes(len) as usize];
                    if c.read_exact(&mut d).is_err() {
                        return;
                    }
                    let header = match d[3] {
                        1 => 10,
                        3 => 7 + d[4] as usize,
                        4 => 22,
                        _ => continue,
                    };
                    let dest = match d[3] {
                        3 => String::from_utf8_lossy(&d[5..5 + d[4] as usize]).into_owned(),
                        _ => format!("atyp {}", d[3]),
                    };
                    let port = u16::from_be_bytes([d[header - 2], d[header - 1]]);
                    let _ = dests.send(format!("{dest}:{port}"));
                    let mut answer = d[..header].to_vec();
                    answer.extend_from_slice(b"ECHO:");
                    answer.extend_from_slice(&d[header..]);
                    let mut frame = (answer.len() as u16).to_be_bytes().to_vec();
                    frame.extend_from_slice(&answer);
                    if c.write_all(&frame).is_err() {
                        return;
                    }
                }
            });
        }
    });
    let program = build_program(&dir, "udp-name-client", UDP_NAME_CLIENT, false);
    let stdout = run(&dir, &[&sock], &program);
    assert!(stdout.contains("UDP:9:ECHO:PING"), "{stdout:?}");
    assert_eq!(seen.try_recv().unwrap(), "stun.example:19302");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn seccomp_backend_relays_udp() {
    test_host::need_loopback!();
    if !have_compiler() || !seccomp_available() {
        eprintln!("skipping: no compiler or no SECCOMP_RET_USER_NOTIF");
        return;
    }
    let dir = std::env::temp_dir().join(format!("podnet-netns-udp-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let sock = dir.join("socks.sock");

    // A UDP echo, the thing the client believes it is talking to.
    let echo = UdpSocket::bind("127.0.0.1:0").unwrap();
    let echo_addr = echo.local_addr().unwrap();
    std::thread::spawn(move || {
        let mut buf = [0u8; 65535];
        while let Ok((n, from)) = echo.recv_from(&mut buf) {
            let mut reply = b"ECHO:".to_vec();
            reply.extend_from_slice(&buf[..n]);
            let _ = echo.send_to(&reply, from);
        }
    });

    // A SOCKS5 proxy with UDP ASSOCIATE on a unix control socket.
    let relay = UdpSocket::bind("127.0.0.1:0").unwrap();
    let relay_addr = relay.local_addr().unwrap();
    let listener = UnixListener::bind(&sock).unwrap();
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let mut c = match conn {
                Ok(c) => c,
                Err(_) => break,
            };
            let Some((cmd, _host, _port)) = read_request(&mut c) else {
                continue;
            };
            if cmd != 3 {
                continue;
            }
            let mut bnd = vec![5u8, 0, 0, 1];
            bnd.extend_from_slice(
                &relay_addr
                    .ip()
                    .to_string()
                    .parse::<Ipv4Addr>()
                    .unwrap()
                    .octets(),
            );
            bnd.extend_from_slice(&relay_addr.port().to_be_bytes());
            let _ = c.write_all(&bnd);
            // Hold the control connection open for the life of the test.
            let mut buf = [0u8; 1];
            while c.read(&mut buf).unwrap_or(0) == 1 {}
        }
    });
    // The relay: unwrap a SOCKS5 UDP header, forward, wrap the reply.
    {
        std::thread::spawn(move || {
            let mut buf = [0u8; 65535];
            while let Ok((n, from)) = relay.recv_from(&mut buf) {
                if n < 4 {
                    continue;
                }
                let off = match buf[3] {
                    1 => 10,
                    3 => 4 + 1 + buf[4] as usize + 2,
                    4 => 22,
                    _ => continue,
                };
                if n < off {
                    continue;
                }
                let up = UdpSocket::bind("127.0.0.1:0").unwrap();
                up.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
                let _ = up.send_to(&buf[off..n], echo_addr);
                let mut reply = [0u8; 65535];
                if let Ok((rn, _)) = up.recv_from(&mut reply) {
                    let mut out = vec![0u8, 0, 0, 1, 127, 0, 0, 1];
                    out.extend_from_slice(&echo_addr.port().to_be_bytes());
                    out.extend_from_slice(&reply[..rn]);
                    let _ = relay.send_to(&out, from);
                }
            }
        });
    }

    let program = build_program(&dir, "udp-client", UDP_CLIENT, false);
    let stdout = run(&dir, &[&sock], &program);
    assert!(stdout.contains("UDP:9:ECHO:PING"), "{stdout:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn podnet_netns_serves_a_udp_front_door() {
    test_host::need_loopback!();
    if !have_compiler() || !seccomp_available() {
        eprintln!("skipping: no compiler or no SECCOMP_RET_USER_NOTIF");
        return;
    }
    let dir = std::env::temp_dir().join(format!("podnet-netns-udpfront-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    let echo = UdpSocket::bind("127.0.0.1:0").unwrap();
    let echo_addr = echo.local_addr().unwrap();
    std::thread::spawn(move || {
        let mut buf = [0u8; 65535];
        while let Ok((n, from)) = echo.recv_from(&mut buf) {
            let mut reply = b"ECHO:".to_vec();
            reply.extend_from_slice(&buf[..n]);
            let _ = echo.send_to(&reply, from);
        }
    });

    // A UDP ASSOCIATE proxy for the front door to forward through.
    let socks = dir.join("udp.sock");
    let relay = UdpSocket::bind("127.0.0.1:0").unwrap();
    let relay_addr = relay.local_addr().unwrap();
    let listener = UnixListener::bind(&socks).unwrap();
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let mut c = match conn {
                Ok(c) => c,
                Err(_) => break,
            };
            let Some((cmd, _, _)) = read_request(&mut c) else {
                continue;
            };
            if cmd != 3 {
                continue;
            }
            let mut bnd = vec![5u8, 0, 0, 1];
            bnd.extend_from_slice(
                &relay_addr
                    .ip()
                    .to_string()
                    .parse::<Ipv4Addr>()
                    .unwrap()
                    .octets(),
            );
            bnd.extend_from_slice(&relay_addr.port().to_be_bytes());
            let _ = c.write_all(&bnd);
            let mut buf = [0u8; 1];
            while c.read(&mut buf).unwrap_or(0) == 1 {}
        }
    });
    {
        std::thread::spawn(move || {
            let mut buf = [0u8; 65535];
            while let Ok((n, from)) = relay.recv_from(&mut buf) {
                if n < 4 {
                    continue;
                }
                let off = match buf[3] {
                    1 => 10,
                    3 => 4 + 1 + buf[4] as usize + 2,
                    4 => 22,
                    _ => continue,
                };
                if n < off {
                    continue;
                }
                let up = UdpSocket::bind("127.0.0.1:0").unwrap();
                up.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
                let _ = up.send_to(&buf[off..n], echo_addr);
                let mut reply = [0u8; 65535];
                if let Ok((rn, _)) = up.recv_from(&mut reply) {
                    let mut out = vec![0u8, 0, 0, 1, 127, 0, 0, 1];
                    out.extend_from_slice(&echo_addr.port().to_be_bytes());
                    out.extend_from_slice(&reply[..rn]);
                    let _ = relay.send_to(&out, from);
                }
            }
        });
    }

    let front = dir.join("front.sock");
    let mut server = pod_command()
        .args(["--serve"])
        .arg(format!("unix:{}", front.display()))
        .args(["-x"])
        .arg(format!("unix:{}", socks.display()))
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

    let program = build_program(&dir, "udp-client-front", UDP_CLIENT, false);
    let output = pod_command()
        .args(["--backend", "seccomp", "-x"])
        .arg(format!("unix:{}", front.display()))
        .arg("--")
        .arg(&program)
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("UDP:9:ECHO:PING"),
        "{:?}",
        String::from_utf8_lossy(&output.stdout)
    );

    let _ = server.kill();
    let _ = server.wait();
    let _ = std::fs::remove_dir_all(&dir);
}

pub(super) const HUGE_SEND_CLIENT: &str = r#"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <errno.h>
#include <unistd.h>
#include <arpa/inet.h>
#include <sys/socket.h>
#include <sys/uio.h>
int main(void) {
    alarm(10);
    int s = socket(AF_INET, SOCK_DGRAM, 0);
    struct sockaddr_in d; memset(&d, 0, sizeof d);
    d.sin_family = AF_INET; d.sin_port = htons(9999);
    inet_pton(AF_INET, "10.66.0.1", &d.sin_addr);
    static char b[80000];
    long n = sendto(s, b, (size_t)1 << 40, 0, (struct sockaddr *)&d, sizeof d);
    printf("HUGE:%ld:%d\n", n, n < 0 ? errno : 0);
    n = sendto(s, b, 70000, 0, (struct sockaddr *)&d, sizeof d);
    printf("LONG:%ld:%d\n", n, n < 0 ? errno : 0);
    struct iovec iov[2] = { { b, 40000 }, { b, 40000 } };
    struct msghdr m; memset(&m, 0, sizeof m);
    m.msg_name = &d; m.msg_namelen = sizeof d; m.msg_iov = iov; m.msg_iovlen = 2;
    n = sendmsg(s, &m, 0);
    printf("IOV:%ld:%d\n", n, n < 0 ? errno : 0);
    iov[0].iov_len = (size_t)1 << 40;
    m.msg_iovlen = 1;
    n = sendmsg(s, &m, 0);
    printf("IOVHUGE:%ld:%d\n", n, n < 0 ? errno : 0);
    printf("ALIVE\n");
    return 0;
}
"#;

/// Issue 303: a length of the program that is too large for a datagram
/// gets EMSGSIZE. The supervisor does not allocate it. Before, a sendto
/// of 1 TiB made the supervisor abort (exit 134).
#[test]
fn a_huge_datagram_length_gets_emsgsize() {
    if !have_compiler() || !seccomp_available() {
        eprintln!("skipping: no compiler or no SECCOMP_RET_USER_NOTIF");
        return;
    }
    let dir = std::env::temp_dir().join(format!("podnet-netns-hugesend-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let program = build_program(&dir, "hugesend", HUGE_SEND_CLIENT, false);
    let stdout = run(&dir, &[&dir.join("nobody.sock")], &program);
    let emsgsize = libc::EMSGSIZE;
    for call in ["HUGE", "LONG", "IOV", "IOVHUGE"] {
        assert!(
            stdout.contains(&format!("{call}:-1:{emsgsize}\n")),
            "{call}: {stdout:?}"
        );
    }
    assert!(stdout.contains("ALIVE"), "{stdout:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

pub(super) const UDP_SOURCE_CLIENT: &str = r#"
#include <stdio.h>
#include <string.h>
#include <errno.h>
#include <unistd.h>
#include <netdb.h>
#include <arpa/inet.h>
#include <sys/socket.h>
#include <sys/uio.h>
static void show(const char *tag, struct sockaddr_in *a, socklen_t len, const char *b, int n) {
    char ip[32] = {0};
    inet_ntop(AF_INET, &a->sin_addr, ip, sizeof ip);
    printf("%s:%d:%s:%d:%u:%.*s\n", tag, a->sin_family, ip, ntohs(a->sin_port), (unsigned)len, n > 0 ? n : 0, b);
}
int main(void) {
    alarm(10);
    struct addrinfo hints, *res = NULL;
    memset(&hints, 0, sizeof hints);
    hints.ai_family = AF_INET; hints.ai_socktype = SOCK_DGRAM;
    if (getaddrinfo("echo.example", "7777", &hints, &res) != 0) { printf("GAI\n"); return 1; }
    char sent[32] = {0};
    inet_ntop(AF_INET, &((struct sockaddr_in *)res->ai_addr)->sin_addr, sent, sizeof sent);
    printf("SENT:%s\n", sent);
    int s = socket(AF_INET, SOCK_DGRAM, 0);
    char b[128];
    struct sockaddr_in src; socklen_t len;
    int n = recvfrom(s, b, sizeof b, MSG_DONTWAIT, (struct sockaddr *)&src, &len);
    printf("AGAIN:%d:%d\n", n, n < 0 ? errno : 0);
    /* A name: the proxy answers from the address of the host. */
    if (sendto(s, "ONE", 3, 0, res->ai_addr, res->ai_addrlen) < 0) { perror("sendto"); return 1; }
    memset(&src, 0, sizeof src); len = sizeof src;
    n = recvfrom(s, b, sizeof b, MSG_PEEK, (struct sockaddr *)&src, &len);
    show("PEEK", &src, len, b, n);
    memset(&src, 0, sizeof src); len = sizeof src;
    n = recvfrom(s, b, sizeof b, 0, (struct sockaddr *)&src, &len);
    show("FROM", &src, len, b, n);
    /* An address, with recvmsg. */
    struct sockaddr_in d; memset(&d, 0, sizeof d);
    d.sin_family = AF_INET; d.sin_port = htons(7778);
    inet_pton(AF_INET, "10.66.0.1", &d.sin_addr);
    if (sendto(s, "TWO", 3, 0, (struct sockaddr *)&d, sizeof d) < 0) { perror("sendto"); return 1; }
    struct iovec iov = { b, sizeof b };
    struct msghdr m; memset(&m, 0, sizeof m);
    memset(&src, 0, sizeof src);
    m.msg_name = &src; m.msg_namelen = sizeof src; m.msg_iov = &iov; m.msg_iovlen = 1;
    n = recvmsg(s, &m, 0);
    show("MSG", &src, m.msg_namelen, b, n);
    printf("MSGFLAGS:%d\n", m.msg_flags);
    /* recv with no source buffer: the kernel runs it, and the source of
       the next datagram is still right. */
    if (sendto(s, "FOUR", 4, 0, (struct sockaddr *)&d, sizeof d) < 0) { perror("sendto"); return 1; }
    struct sockaddr_in e = d; e.sin_port = htons(7779);
    if (sendto(s, "FIVE", 4, 0, (struct sockaddr *)&e, sizeof e) < 0) { perror("sendto"); return 1; }
    n = recv(s, b, sizeof b, 0);
    printf("RECV:%d:%.*s\n", n, n > 0 ? n : 0, b);
    memset(&src, 0, sizeof src); len = sizeof src;
    n = recvfrom(s, b, sizeof b, 0, (struct sockaddr *)&src, &len);
    show("AFTER", &src, len, b, n);
    /* A short buffer: the datagram is cut, MSG_TRUNC gives its length. */
    if (sendto(s, "THREE", 5, 0, (struct sockaddr *)&d, sizeof d) < 0) { perror("sendto"); return 1; }
    memset(b, 0, sizeof b);
    iov.iov_len = 4; m.msg_namelen = sizeof src;
    n = recvmsg(s, &m, MSG_TRUNC);
    printf("TRUNC:%d:%.4s:%d\n", n, b, (m.msg_flags & MSG_TRUNC) != 0);
    return 0;
}
"#;

/// Issue 305: recvfrom and recvmsg on an unconnected UDP socket give the
/// address that the program sent to as the source, not the AF_UNIX
/// address of the socketpair. A proxy that answers for a name from the
/// address of the host gives the fake IP of the name.
#[test]
fn an_unconnected_udp_reader_sees_the_source_address() {
    if !have_compiler() || !seccomp_available() {
        eprintln!("skipping: no compiler or no SECCOMP_RET_USER_NOTIF");
        return;
    }
    let dir = std::env::temp_dir().join(format!("podnet-netns-udp-source-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let sock = dir.join("socks.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut c) = conn else { break };
            std::thread::spawn(move || {
                let Some((cmd, _, _)) = read_request(&mut c) else {
                    return;
                };
                if cmd != 0x83 {
                    let _ = c.write_all(&[5, 7, 0, 1, 0, 0, 0, 0, 0, 0]);
                    return;
                }
                let _ = c.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
                loop {
                    let mut len = [0u8; 2];
                    if c.read_exact(&mut len).is_err() {
                        return;
                    }
                    let mut d = vec![0u8; u16::from_be_bytes(len) as usize];
                    if c.read_exact(&mut d).is_err() {
                        return;
                    }
                    let header = match d[3] {
                        1 => 10,
                        3 => 7 + d[4] as usize,
                        _ => continue,
                    };
                    // A name gets an answer from the address of the host.
                    let mut answer = if d[3] == 3 {
                        let mut a = vec![0, 0, 0, 1, 93, 184, 216, 34];
                        a.extend_from_slice(&d[header - 2..header]);
                        a
                    } else {
                        d[..header].to_vec()
                    };
                    answer.extend_from_slice(b"ECHO:");
                    answer.extend_from_slice(&d[header..]);
                    let mut frame = (answer.len() as u16).to_be_bytes().to_vec();
                    frame.extend_from_slice(&answer);
                    if c.write_all(&frame).is_err() {
                        return;
                    }
                }
            });
        }
    });
    let program = build_program(&dir, "udp-source", UDP_SOURCE_CLIENT, false);
    for hook in ["", "no-fd"] {
        let output = pod_command()
            .args(["--backend", "seccomp", "-x"])
            .arg(format!("unix:{}", sock.display()))
            .arg("--")
            .arg(&program)
            .env("PODNET_NETNS_TEST_MEM", hook)
            .output()
            .unwrap();
        let out = String::from_utf8_lossy(&output.stdout);
        let err = String::from_utf8_lossy(&output.stderr);
        let sent = out
            .lines()
            .find_map(|l| l.strip_prefix("SENT:"))
            .unwrap_or_else(|| panic!("{hook:?}: {out:?} {err:?}"));
        let want = [
            format!("AGAIN:-1:{}", libc::EAGAIN),
            format!("PEEK:2:{sent}:7777:16:ECHO:ONE"),
            format!("FROM:2:{sent}:7777:16:ECHO:ONE"),
            "MSG:2:10.66.0.1:7778:16:ECHO:TWO".to_string(),
            "MSGFLAGS:0".to_string(),
            "RECV:9:ECHO:FOUR".to_string(),
            "AFTER:2:10.66.0.1:7779:16:ECHO:FIVE".to_string(),
            "TRUNC:10:ECHO:1".to_string(),
        ];
        for line in want {
            assert!(
                out.lines().any(|l| l == line),
                "{hook:?}: no {line:?} in {out:?} {err:?}"
            );
        }
    }
    // With no write to the memory of the program, the read stays direct
    // and the datagram still arrives.
    let output = pod_command()
        .args(["--backend", "seccomp", "-x"])
        .arg(format!("unix:{}", sock.display()))
        .arg("--")
        .arg(&program)
        .env("PODNET_NETNS_TEST_MEM", "none")
        .env("PODNET_NETNS_QUIET", "1")
        .output()
        .unwrap();
    let out = String::from_utf8_lossy(&output.stdout);
    assert!(out.contains(":ECHO:ONE\n"), "{out:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
