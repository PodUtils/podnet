//! Generic UDP of the netns backend (T-111): each flow goes through the
//! proxy, in the stream of the proxy or with UDP ASSOCIATE.

use super::udp::{UDP_CLIENT, UDP_NAME_CLIENT};
use super::*;

/// A proxy on a unix socket that knows only the UDP association in the
/// stream (0x83). It sends each datagram back with `ECHO:` before the
/// payload, from the address of the datagram, and gives each destination
/// (`name:port` or `ip:port`) to `dests`.
fn stream_echo_proxy(listener: UnixListener, dests: mpsc::Sender<String>) {
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
                    _ => continue,
                };
                let dest = match d[3] {
                    3 => String::from_utf8_lossy(&d[5..5 + d[4] as usize]).into_owned(),
                    _ => Ipv4Addr::new(d[4], d[5], d[6], d[7]).to_string(),
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
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("podnet-netns-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn netns_backend_relays_udp_in_the_stream() {
    if !have_compiler() || !netns_available() {
        eprintln!("skipping: no compiler or no netns backend");
        return;
    }
    let dir = temp_dir("nudp-stream");
    let sock = dir.join("socks.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    let (dests, seen) = mpsc::channel();
    std::thread::spawn(move || stream_echo_proxy(listener, dests));
    // A name of the pod goes to the proxy as the name.
    let program = build_program(&dir, "udp-name", UDP_NAME_CLIENT, false);
    let stdout = run_on("netns", &[&sock], &program);
    assert!(stdout.contains("UDP:9:ECHO:PING"), "{stdout:?}");
    assert_eq!(seen.try_recv().unwrap(), "stun.example:19302");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn netns_backend_relays_udp() {
    test_host::need_loopback!();
    if !have_compiler() || !netns_available() {
        eprintln!("skipping: no compiler or no netns backend");
        return;
    }
    let dir = temp_dir("nudp-assoc");
    let sock = dir.join("socks.sock");
    // The relay of the association: it answers each datagram with ECHO:
    // and the same header.
    let relay = UdpSocket::bind("127.0.0.1:0").unwrap();
    let relay_addr = relay.local_addr().unwrap();
    std::thread::spawn(move || {
        let mut buf = [0u8; 65535];
        while let Ok((n, from)) = relay.recv_from(&mut buf) {
            if n < 10 || buf[3] != 1 {
                continue;
            }
            let mut out = buf[..10].to_vec();
            out.extend_from_slice(b"ECHO:");
            out.extend_from_slice(&buf[10..n]);
            let _ = relay.send_to(&out, from);
        }
    });
    let listener = UnixListener::bind(&sock).unwrap();
    let (commands, seen) = mpsc::channel::<u8>();
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut c) = conn else { break };
            let Some((cmd, _, _)) = read_request(&mut c) else {
                continue;
            };
            let _ = commands.send(cmd);
            if cmd != 3 {
                // A server that does not know 0x83 closes.
                continue;
            }
            let mut bnd = vec![5u8, 0, 0, 1, 127, 0, 0, 1];
            bnd.extend_from_slice(&relay_addr.port().to_be_bytes());
            let _ = c.write_all(&bnd);
            std::thread::spawn(move || {
                let mut buf = [0u8; 1];
                while c.read(&mut buf).unwrap_or(0) == 1 {}
            });
        }
    });
    let source = UDP_CLIENT.replace("127.0.0.1", "93.184.216.34");
    let program = build_program(&dir, "udp-assoc", &source, false);
    let stdout = run_on("netns", &[&sock], &program);
    assert!(stdout.contains("UDP:9:ECHO:PING"), "{stdout:?}");
    let commands: Vec<u8> = seen.try_iter().collect();
    assert_eq!(commands, vec![0x83, 3], "in the stream, then UDP ASSOCIATE");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Two sockets send 50 datagrams each to one port. Each socket gets its
/// own 50 answers.
const TWO_FLOWS: &str = r#"
#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include <arpa/inet.h>
#include <sys/socket.h>
#include <sys/time.h>
int main(void) {
    alarm(20);
    int s[2];
    struct timeval tv = { 5, 0 };
    for (int k = 0; k < 2; k++) {
        s[k] = socket(AF_INET, SOCK_DGRAM, 0);
        setsockopt(s[k], SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof tv);
    }
    struct sockaddr_in d; memset(&d, 0, sizeof d);
    d.sin_family = AF_INET; d.sin_port = htons(7000);
    inet_pton(AF_INET, "93.184.216.34", &d.sin_addr);
    char m[32];
    for (int i = 0; i < 50; i++) {
        for (int k = 0; k < 2; k++) {
            int n = snprintf(m, sizeof m, "%c%d", 'A' + k, i);
            if (sendto(s[k], m, n, 0, (struct sockaddr *)&d, sizeof d) < 0) { perror("sendto"); return 1; }
        }
    }
    int got[2] = {0, 0}, mixed = 0;
    for (int k = 0; k < 2; k++) {
        char b[64];
        for (;;) {
            int n = recv(s[k], b, sizeof b - 1, 0);
            if (n <= 0) break;
            b[n] = 0;
            if (strncmp(b, "ECHO:", 5) != 0 || b[5] != 'A' + k) mixed++;
            else got[k]++;
            if (got[k] == 50) break;
        }
    }
    printf("A:%d B:%d MIXED:%d\n", got[0], got[1], mixed);
    return 0;
}
"#;

#[test]
fn netns_two_flows_to_one_port_lose_nothing() {
    if !have_compiler() || !netns_available() {
        eprintln!("skipping: no compiler or no netns backend");
        return;
    }
    let dir = temp_dir("nudp-two");
    let sock = dir.join("socks.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    let (dests, seen) = mpsc::channel();
    std::thread::spawn(move || stream_echo_proxy(listener, dests));
    let program = build_program(&dir, "udp-two", TWO_FLOWS, false);
    let stdout = run_on("netns", &[&sock], &program);
    assert!(stdout.contains("A:50 B:50 MIXED:0"), "{stdout:?}");
    let dests: Vec<String> = seen.try_iter().collect();
    assert_eq!(dests.len(), 100);
    assert!(dests.iter().all(|d| d == "93.184.216.34:7000"), "{dests:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
